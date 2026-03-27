mod claude;
mod config;
mod grid;
mod mqtt;
mod parser;
mod telegram;

use std::path::PathBuf;
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

const DEBOUNCE_MS: u64 = 200;

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

    // Subscribe to this session's PTY output + meta
    let sessions = vec![session_id.clone()];
    mqtt::subscribe(&client, &sessions).await?;
    mqtt::subscribe_meta(&client, &sessions).await?;

    // Telegram bot state
    let bot_state = Arc::new(Mutex::new(BotState {
        mqtt_client: client.clone(),
        session_id: session_id.clone(),
        last_state: ClaudeState::Unknown,
        last_sent_response: String::new(),
        chat_id: None,
    }));

    // Spawn Telegram bot
    let tg_token = config.telegram.bot_token.clone();
    let bot_state_tg = bot_state.clone();
    tokio::spawn(async move {
        if let Err(e) = telegram::run_bot(&tg_token, bot_state_tg).await {
            tracing::error!("telegram bot error: {}", e);
        }
    });

    let tg_bot = Bot::new(&config.telegram.bot_token);

    // Parser state for the session
    let mut vte_parser = Parser::new();
    let mut performer = GridPerformer::new(Grid::default());
    let mut last_update = Instant::now();
    let mut last_published = String::new();
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let mut tick = tokio::time::interval(debounce);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    mqtt::Message::Pty(pty) => {
                        if pty.session_id == session_id {
                            GridPerformer::feed(&mut vte_parser, &mut performer, &pty.payload);
                            last_update = Instant::now();
                        }
                    }
                    mqtt::Message::Meta(meta) => {
                        if meta.session_id == session_id {
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
                if now.duration_since(last_update) >= debounce {
                    let snapshot = performer.grid.snapshot();
                    if snapshot != last_published && !snapshot.is_empty() {
                        // Publish to MQTT text topic
                        let _ = publish_client
                            .publish(
                                &format!("hermytt/{}/pty/text", session_id),
                                QoS::AtMostOnce,
                                false,
                                snapshot.as_bytes(),
                            )
                            .await;

                        // Parse Claude state and push to Telegram
                        let screen = claude::parse_screen(&snapshot);
                        tracing::debug!(state = ?screen.state, "claude state");
                        if screen.state == claude::ClaudeState::Unknown {
                            // Log last 5 non-empty lines for debugging
                            let tail: Vec<&str> = snapshot.lines()
                                .filter(|l| !l.trim().is_empty())
                                .collect();
                            let tail_start = tail.len().saturating_sub(5);
                            tracing::debug!(tail = ?&tail[tail_start..], "screen tail");
                        }

                        let mut state = bot_state.lock().await;
                        telegram::on_screen_update(&tg_bot, &mut state, &screen).await;

                        last_published = snapshot;
                    }
                }
            }
        }
    }

    Ok(())
}
