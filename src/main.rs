mod api;
mod claude;
mod config;
mod grid;
mod login;
mod mqtt;
mod parser;
mod telegram;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rumqttc::QoS;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing_subscriber::EnvFilter;
use vte::Parser;

use crate::config::{Config, SessionConfig};
use crate::grid::Grid;
use crate::login::LoginFlow;
use crate::parser::GridPerformer;
use crate::telegram::BotState;

/// Per-session runtime state
struct SessionRuntime {
    config: SessionConfig,
    vte_parser: Parser,
    performer: GridPerformer,
    last_update: Instant,
    last_published: String,
    bot_state: Arc<Mutex<BotState>>,
    tg_bot: Bot,
    login_flow: LoginFlow,
    app_state: Arc<api::AppState>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("grytti.toml"));

    let config = Config::load(&config_path)?;
    tracing::info!("loaded config from {}", config_path.display());

    let session_configs = config.resolved_sessions();
    if session_configs.is_empty() {
        anyhow::bail!("no sessions configured");
    }

    tracing::info!("configured {} session(s)", session_configs.len());

    // MQTT setup — single connection, wildcard sub
    let (client, eventloop, mut rx, tx) = mqtt::connect(&config)?;
    let publish_client = client.clone();

    tokio::spawn(async move {
        if let Err(e) = mqtt::run_eventloop(eventloop, tx).await {
            tracing::error!("mqtt loop error: {}", e);
        }
    });

    mqtt::subscribe(&client, &[]).await?;
    mqtt::subscribe_meta(&client, &[]).await?;

    // Build per-session runtimes
    let mut sessions: HashMap<String, SessionRuntime> = HashMap::new();

    for sc in &session_configs {
        let bot_state = Arc::new(Mutex::new(BotState {
            mqtt_client: client.clone(),
            session_id: sc.session_id.clone(),
            last_state: claude::ClaudeState::Unknown,
            last_sent_response: String::new(),
            chat_id: None,
        }));

        let app_state = Arc::new(api::AppState {
            bot_state: bot_state.clone(),
            mutable: Mutex::new(api::MutableState {
                session_id: sc.session_id.clone(),
                debounce_ms: sc.debounce_ms,
            }),
            mqtt_host: config.mqtt.host.clone(),
            mqtt_port: config.mqtt.port,
            start_time: std::time::Instant::now(),
            messages_processed: AtomicU64::new(0),
            last_snapshot: Mutex::new(String::new()),
            login_flow: Mutex::new(LoginFlow::new()),
        });

        // Spawn TG bot for this session
        let tg_token = sc.telegram.bot_token.clone();
        let bot_state_tg = bot_state.clone();
        let app_state_tg = app_state.clone();
        tokio::spawn(async move {
            if let Err(e) = telegram::run_bot(&tg_token, bot_state_tg, app_state_tg).await {
                tracing::error!("telegram bot error: {}", e);
            }
        });

        let tg_bot = Bot::new(&sc.telegram.bot_token);

        tracing::info!(session = %sc.session_id, "session initialized");

        sessions.insert(sc.session_id.clone(), SessionRuntime {
            config: sc.clone(),
            vte_parser: Parser::new(),
            performer: GridPerformer::new(Grid::default()),
            last_update: Instant::now(),
            last_published: String::new(),
            bot_state,
            tg_bot,
            login_flow: LoginFlow::new(),
            app_state,
        });
    }

    // API — use the first session's app_state for now
    // TODO: multi-session API
    let first_app_state = sessions.values().next().unwrap().app_state.clone();
    let api_bind = format!("{}:{}", config.api.bind, config.api.port);
    let api_router = api::router(first_app_state);
    let listener = tokio::net::TcpListener::bind(&api_bind).await?;
    tracing::info!("API listening on {}", api_bind);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, api_router).await {
            tracing::error!("api server error: {}", e);
        }
    });

    // Registry heartbeat
    if let Some(ref registry_url) = config.api.hermytt_registry {
        let endpoint = config.api.endpoint.clone()
            .unwrap_or_else(|| format!("http://{}:{}", config.api.bind, config.api.port));
        let url = registry_url.clone();
        let token = config.api.hermytt_token.clone();
        tokio::spawn(async move {
            api::announce_to_registry(&url, &endpoint, token.as_deref()).await;
            api::heartbeat_loop(url, endpoint, token).await;
        });
    }

    // Global debounce tick
    let debounce = Duration::from_millis(config.debounce_ms);
    let mut tick = tokio::time::interval(debounce);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    mqtt::Message::Pty(pty) => {
                        // Find matching session (check mutable state for runtime changes)
                        let matching_sid = {
                            let mut found = None;
                            for (sid, rt) in &sessions {
                                let ms = rt.app_state.mutable.lock().await;
                                if pty.session_id == ms.session_id {
                                    found = Some(sid.clone());
                                    break;
                                }
                            }
                            found
                        };
                        if let Some(sid) = matching_sid {
                            if let Some(rt) = sessions.get_mut(&sid) {
                                GridPerformer::feed(&mut rt.vte_parser, &mut rt.performer, &pty.payload);
                                rt.last_update = Instant::now();
                                rt.app_state.messages_processed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    mqtt::Message::Meta(meta) => {
                        for rt in sessions.values_mut() {
                            let ms = rt.app_state.mutable.lock().await;
                            if meta.session_id == ms.session_id {
                                if let Some((cols, rows)) = meta.resize {
                                    tracing::info!("resize {} to {}x{}", meta.session_id, cols, rows);
                                    rt.performer.grid.resize(cols, rows);
                                }
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {
                for (original_sid, rt) in &mut sessions {
                    let now = Instant::now();
                    let ms = rt.app_state.mutable.lock().await;
                    let current_debounce = Duration::from_millis(ms.debounce_ms);
                    let sid = ms.session_id.clone();
                    drop(ms);

                    if now.duration_since(rt.last_update) >= current_debounce {
                        let snapshot = rt.performer.grid.snapshot();
                        if snapshot != rt.last_published && !snapshot.is_empty() {
                            let _ = publish_client
                                .publish(
                                    &format!("hermytt/{}/pty/text", sid),
                                    QoS::AtMostOnce,
                                    false,
                                    snapshot.as_bytes(),
                                )
                                .await;

                            let screen = claude::parse_screen(&snapshot);
                            tracing::debug!(session = %original_sid, state = ?screen.state, "claude state");

                            // Drive login flow
                            let mut lf = rt.app_state.login_flow.lock().await;
                            let consumed = lf.on_screen_update(&rt.tg_bot, &rt.app_state, &screen).await;
                            drop(lf);

                            if !consumed {
                                let mut state = rt.bot_state.lock().await;
                                telegram::on_screen_update(&rt.tg_bot, &mut state, &screen).await;
                            }

                            *rt.app_state.last_snapshot.lock().await = snapshot.clone();
                            rt.last_published = snapshot;
                        } else {
                            // Snapshot unchanged — but keep sending typing if still thinking
                            let state = rt.bot_state.lock().await;
                            if state.last_state == claude::ClaudeState::Thinking {
                                if let Some(chat_id) = state.chat_id {
                                    let _ = rt.tg_bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
