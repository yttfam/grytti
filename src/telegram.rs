use std::sync::Arc;

use anyhow::Result;
use rumqttc::AsyncClient;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tokio::sync::Mutex;

use crate::api::AppState;
use crate::claude::{ClaudeScreen, ClaudeState, DetectedProcess};

/// Shared state between the Telegram bot and the MQTT bridge
pub struct BotState {
    pub mqtt_client: AsyncClient,
    pub session_id: String,
    /// Last known Claude state
    pub last_state: ClaudeState,
    /// Last detected process
    pub last_process: DetectedProcess,
    /// Last response we sent to Telegram (to avoid duplicates)
    pub last_sent_response: String,
    /// Chat ID to send updates to (set on first message)
    pub chat_id: Option<ChatId>,
}

pub async fn run_bot(
    token: &str,
    state: Arc<Mutex<BotState>>,
    app_state: Arc<AppState>,
) -> Result<()> {
    let bot = Bot::new(token);

    let state_for_handler = state.clone();

    let handler = Update::filter_message().endpoint(
        move |bot: Bot, msg: Message, state: Arc<Mutex<BotState>>| {
            let app = app_state.clone();
            async move {
                let text = match msg.text() {
                    Some(t) => t.to_string(),
                    None => return respond(()),
                };

                {
                    let mut s = state.lock().await;
                    s.chat_id = Some(msg.chat.id);
                }

                tracing::info!(chat = %msg.chat.id, text = %text, "telegram message received");

                // Check if login flow is waiting for a code
                let is_waiting_for_code = {
                    let lf = app.login_flow.lock().await;
                    lf.is_waiting_for_code()
                };

                let session_id = app.mutable.lock().await.session_id.clone();
                let mqtt = state.lock().await.mqtt_client.clone();

                if is_waiting_for_code {
                    // Forward as OAuth code — paste into the "Paste code here" prompt
                    tracing::info!("forwarding auth code to Claude");
                    let _ = bot.send_message(msg.chat.id, "Sending auth code...").await;
                    let mut data = text.into_bytes();
                    data.push(b'\r');
                    let _ = mqtt
                        .publish(
                            &format!("hermytt/{}/pty/in", session_id),
                            rumqttc::QoS::AtMostOnce,
                            false,
                            data,
                        )
                        .await;
                } else {
                    // Normal message — send typing + inject into Claude
                    let _ = bot.send_chat_action(msg.chat.id, ChatAction::Typing).await;
                    let mut data = text.into_bytes();
                    data.push(b'\r');
                    if let Err(e) = mqtt
                        .publish(
                            &format!("hermytt/{}/pty/in", session_id),
                            rumqttc::QoS::AtMostOnce,
                            false,
                            data,
                        )
                        .await
                    {
                        tracing::error!("failed to send stdin: {}", e);
                        let _ = bot.send_message(msg.chat.id, "Failed to send to Claude").await;
                    }
                }

                Ok(())
            }
        },
    );

    Dispatcher::builder(bot.clone(), handler)
        .dependencies(teloxide::dptree::deps![state_for_handler])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Called on each debounced grid snapshot to push state changes to Telegram.
pub async fn on_screen_update(
    bot: &Bot,
    state: &mut BotState,
    screen: &ClaudeScreen,
) {
    let chat_id = match state.chat_id {
        Some(id) => id,
        None => return,
    };

    // State transition: became thinking → send typing indicator
    if screen.state == ClaudeState::Thinking && state.last_state != ClaudeState::Thinking {
        tracing::info!("claude is thinking");
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
    }

    // Keep sending typing while thinking
    if screen.state == ClaudeState::Thinking {
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
    }

    // State transition: response available and different from last sent
    if screen.state == ClaudeState::Idle {
        if let Some(ref response) = screen.response {
            if *response != state.last_sent_response && !response.is_empty() {
                tracing::info!(len = response.len(), "sending response to telegram");
                for chunk in chunk_message(response, 4096) {
                    let _ = bot.send_message(chat_id, chunk).await;
                }
                state.last_sent_response = response.clone();
            }
        }
    }

    // Process change notification
    if screen.process != state.last_process && screen.process != DetectedProcess::Unknown {
        let label = match screen.process {
            DetectedProcess::ClaudeCode => "Claude Code",
            DetectedProcess::Shell => "Shell",
            DetectedProcess::Unknown => "Unknown",
        };
        tracing::info!(process = label, "process changed");
        let _ = bot.send_message(chat_id, format!("[{}]", label)).await;
        state.last_process = screen.process.clone();
    }

    state.last_state = screen.state.clone();
}

fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + max_len).min(text.len());
        let break_at = text[start..end]
            .rfind('\n')
            .map(|i| start + i + 1)
            .unwrap_or(end);
        chunks.push(&text[start..break_at]);
        start = break_at;
    }
    chunks
}
