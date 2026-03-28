mod api;
mod claude;
mod config;
mod grid;
mod login;
mod mqtt;
mod parser;
mod telegram;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rumqttc::QoS;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing_subscriber::EnvFilter;
use vte::Parser;

use crate::claude::ClaudeState;
use crate::config::Config;
use crate::grid::Grid;
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

    let session_id = config.session_id.clone();

    // MQTT setup
    let (client, eventloop, mut rx, tx) = mqtt::connect(&config)?;
    let publish_client = client.clone();

    tokio::spawn(async move {
        if let Err(e) = mqtt::run_eventloop(eventloop, tx).await {
            tracing::error!("mqtt loop error: {}", e);
        }
    });

    mqtt::subscribe(&client, &[]).await?;
    mqtt::subscribe_meta(&client, &[]).await?;

    // Telegram bot state
    let bot_state = Arc::new(Mutex::new(BotState {
        mqtt_client: client.clone(),
        session_id: session_id.clone(),
        last_state: ClaudeState::Unknown,
        last_sent_response: String::new(),
        chat_id: None,
    }));

    // Spawn REST API
    let app_state = Arc::new(api::AppState {
        bot_state: bot_state.clone(),
        mutable: Mutex::new(api::MutableState {
            session_id: session_id.clone(),
            debounce_ms: config.debounce_ms,
        }),
        mqtt_host: config.mqtt.host.clone(),
        mqtt_port: config.mqtt.port,
        start_time: std::time::Instant::now(),
        messages_processed: std::sync::atomic::AtomicU64::new(0),
        last_snapshot: Mutex::new(String::new()),
        login_flow: Mutex::new(login::LoginFlow::new()),
    });

    // Spawn Telegram bot — pass app_state so it can check login flow
    let tg_token = config.telegram.bot_token.clone();
    let bot_state_tg = bot_state.clone();
    let app_state_tg = app_state.clone();
    tokio::spawn(async move {
        if let Err(e) = telegram::run_bot(&tg_token, bot_state_tg, app_state_tg).await {
            tracing::error!("telegram bot error: {}", e);
        }
    });

    let api_bind = format!("{}:{}", config.api.bind, config.api.port);
    let api_router = api::router(app_state.clone());
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

    let tg_bot = Bot::new(&config.telegram.bot_token);

    // Parser state
    let mut vte_parser = Parser::new();
    let mut performer = GridPerformer::new(Grid::default());
    let mut last_update = Instant::now();
    let mut last_published = String::new();
    let debounce = Duration::from_millis(config.debounce_ms);
    let mut tick = tokio::time::interval(debounce);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    mqtt::Message::Pty(pty) => {
                        let ms = app_state.mutable.lock().await;
                        if pty.session_id == ms.session_id {
                            drop(ms);
                            GridPerformer::feed(&mut vte_parser, &mut performer, &pty.payload);
                            last_update = Instant::now();
                            app_state.messages_processed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    mqtt::Message::Meta(meta) => {
                        let ms = app_state.mutable.lock().await;
                        if meta.session_id == ms.session_id {
                            if let Some((cols, rows)) = meta.resize {
                                tracing::info!("resize to {}x{}", cols, rows);
                                performer.grid.resize(cols, rows);
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let ms = app_state.mutable.lock().await;
                let current_debounce = Duration::from_millis(ms.debounce_ms);
                let sid = ms.session_id.clone();
                drop(ms);

                if now.duration_since(last_update) >= current_debounce {
                    let snapshot = performer.grid.snapshot();
                    if snapshot != last_published && !snapshot.is_empty() {
                        let _ = publish_client
                            .publish(
                                &format!("hermytt/{}/pty/text", sid),
                                QoS::AtMostOnce,
                                false,
                                snapshot.as_bytes(),
                            )
                            .await;

                        let screen = claude::parse_screen(&snapshot);
                        tracing::debug!(state = ?screen.state, "claude state");

                        // Drive login flow first
                        let mut lf = app_state.login_flow.lock().await;
                        let consumed = lf.on_screen_update(&tg_bot, &app_state, &screen).await;
                        drop(lf);

                        // If login flow didn't consume this update, do normal TG handling
                        if !consumed {
                            let mut state = bot_state.lock().await;
                            telegram::on_screen_update(&tg_bot, &mut state, &screen).await;
                        }

                        *app_state.last_snapshot.lock().await = snapshot.clone();
                        last_published = snapshot;
                    }
                }
            }
        }
    }

    Ok(())
}
