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

use crate::config::Config;
use crate::grid::Grid;
use crate::login::LoginFlow;
use crate::parser::GridPerformer;
use crate::telegram::BotState;

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

    // MQTT
    let (client, eventloop, mut rx, tx) = mqtt::connect(&config)?;
    let publish_client = client.clone();

    tokio::spawn(async move {
        if let Err(e) = mqtt::run_eventloop(eventloop, tx).await {
            tracing::error!("mqtt loop error: {}", e);
        }
    });

    mqtt::subscribe(&client, &[]).await?;
    mqtt::subscribe_meta(&client, &[]).await?;

    // Build sessions
    let mut session_states: HashMap<String, Arc<api::SessionState>> = HashMap::new();

    for sc in &session_configs {
        let ss = create_session_state(&client, &sc.session_id, sc.telegram.bot_token.clone(), sc.debounce_ms);
        session_states.insert(sc.session_id.clone(), ss);
        tracing::info!(session = %sc.session_id, "session initialized");
    }

    let global_state = Arc::new(api::GlobalState {
        sessions: Mutex::new(session_states),
        mqtt_client: client.clone(),
        mqtt_host: config.mqtt.host.clone(),
        mqtt_port: config.mqtt.port,
        start_time: std::time::Instant::now(),
    });

    // API
    let api_bind = format!("{}:{}", config.api.bind, config.api.port);
    let api_router = api::router(global_state.clone());
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

    // Main loop
    let debounce = Duration::from_millis(config.debounce_ms);
    let mut tick = tokio::time::interval(debounce);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    mqtt::Message::Pty(pty) => {
                        let sessions = global_state.sessions.lock().await;
                        for (_, ss) in sessions.iter() {
                            let ms = ss.mutable.lock().await;
                            if pty.session_id == ms.session_id {
                                drop(ms);
                                let mut rt = ss.runtime.lock().await;
                                let api::SessionRuntime { ref mut vte_parser, ref mut performer, .. } = *rt;
                                GridPerformer::feed(vte_parser, performer, &pty.payload);
                                rt.last_update = Instant::now();
                                ss.messages_processed.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                    mqtt::Message::Meta(meta) => {
                        let sessions = global_state.sessions.lock().await;
                        for (_, ss) in sessions.iter() {
                            let ms = ss.mutable.lock().await;
                            if meta.session_id == ms.session_id {
                                if let Some((cols, rows)) = meta.resize {
                                    tracing::info!("resize {} to {}x{}", meta.session_id, cols, rows);
                                    let mut rt = ss.runtime.lock().await;
                                    rt.performer.grid.resize(cols, rows);
                                }
                                break;
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {
                let sessions = global_state.sessions.lock().await;
                for (key, ss) in sessions.iter() {
                    let ms = ss.mutable.lock().await;
                    let current_debounce = Duration::from_millis(ms.debounce_ms);
                    let sid = ms.session_id.clone();
                    drop(ms);

                    let mut rt = ss.runtime.lock().await;
                    let now = Instant::now();

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
                            tracing::debug!(session = %key, state = ?screen.state, "claude state");

                            let mut lf = ss.login_flow.lock().await;
                            let consumed = lf.on_screen_update(&ss.tg_bot, ss, &screen).await;
                            drop(lf);

                            if !consumed {
                                let mut state = ss.bot_state.lock().await;
                                telegram::on_screen_update(&ss.tg_bot, &mut state, &screen).await;
                            }

                            *ss.last_snapshot.lock().await = snapshot.clone();
                            rt.last_published = snapshot;
                        } else {
                            let state = ss.bot_state.lock().await;
                            if state.last_state == claude::ClaudeState::Thinking {
                                if let Some(chat_id) = state.chat_id {
                                    let _ = ss.tg_bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await;
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

/// Create a new session state with TG bot spawned
fn create_session_state(
    mqtt_client: &rumqttc::AsyncClient,
    session_id: &str,
    bot_token: String,
    debounce_ms: u64,
) -> Arc<api::SessionState> {
    let bot_state = Arc::new(Mutex::new(BotState {
        mqtt_client: mqtt_client.clone(),
        session_id: session_id.to_string(),
        last_state: claude::ClaudeState::Unknown,
        last_process: claude::DetectedProcess::Unknown,
        last_sent_response: String::new(),
        chat_id: None,
    }));

    let tg_bot = Bot::new(&bot_token);

    let ss = Arc::new(api::SessionState {
        bot_state: bot_state.clone(),
        mutable: Mutex::new(api::MutableState {
            session_id: session_id.to_string(),
            debounce_ms,
        }),
        messages_processed: AtomicU64::new(0),
        last_snapshot: Mutex::new(String::new()),
        login_flow: Mutex::new(LoginFlow::new()),
        runtime: Mutex::new(api::SessionRuntime {
            vte_parser: vte::Parser::new(),
            performer: GridPerformer::new(Grid::default()),
            last_update: Instant::now(),
            last_published: String::new(),
        }),
        tg_bot: tg_bot.clone(),
    });

    // Spawn TG bot
    let bot_state_tg = bot_state.clone();
    let ss_tg = ss.clone();
    let mqtt_tg = mqtt_client.clone();
    tokio::spawn(async move {
        if let Err(e) = telegram::run_bot(&bot_token, bot_state_tg, ss_tg, mqtt_tg).await {
            tracing::error!("telegram bot error: {}", e);
        }
    });

    ss
}
