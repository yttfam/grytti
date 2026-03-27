use std::sync::Arc;

use anyhow::Result;
use rumqttc::AsyncClient;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tokio::sync::Mutex;

use crate::claude::{ClaudeScreen, ClaudeState};

/// Shared state between the Telegram bot and the MQTT bridge
pub struct BotState {
    pub mqtt_client: AsyncClient,
    pub session_id: String,
    /// Last known Claude state
    pub last_state: ClaudeState,
    /// Last response we sent to Telegram (to avoid duplicates)
    pub last_sent_response: String,
    /// Chat ID to send updates to (set on first message)
    pub chat_id: Option<ChatId>,
}

pub async fn run_bot(
    token: &str,
    state: Arc<Mutex<BotState>>,
) -> Result<()> {
    let bot = Bot::new(token);

    let state_for_handler = state.clone();

    let handler = Update::filter_message().endpoint(
        move |bot: Bot, msg: Message, state: Arc<Mutex<BotState>>| async move {
            let text = match msg.text() {
                Some(t) => t.to_string(),
                None => return respond(()),
            };

            let session_id = {
                let mut s = state.lock().await;
                // Remember the chat ID for pushing updates
                s.chat_id = Some(msg.chat.id);
                s.session_id.clone()
            };

            tracing::info!(chat = %msg.chat.id, text = %text, "telegram message received");

            // Send typing indicator
            let _ = bot.send_chat_action(msg.chat.id, ChatAction::Typing).await;

            // Inject into Claude's stdin
            let mqtt = state.lock().await.mqtt_client.clone();
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

            Ok(())
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
        None => return, // No chat yet, nobody to notify
    };

    // State transition: became thinking → send typing indicator
    if screen.state == ClaudeState::Thinking && state.last_state != ClaudeState::Thinking {
        tracing::info!("claude is thinking");
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
    }

    // Keep sending typing while thinking (Telegram typing expires after ~5s)
    if screen.state == ClaudeState::Thinking {
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
    }

    // State transition: response available and different from last sent
    if screen.state == ClaudeState::Idle {
        if let Some(ref response) = screen.response {
            if *response != state.last_sent_response && !response.is_empty() {
                tracing::info!(len = response.len(), "sending response to telegram");
                // Telegram has a 4096 char limit per message
                for chunk in chunk_message(response, 4096) {
                    let _ = bot.send_message(chat_id, chunk).await;
                }
                state.last_sent_response = response.clone();
            }
        }
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
        // Try to break at a newline
        let break_at = text[start..end]
            .rfind('\n')
            .map(|i| start + i + 1)
            .unwrap_or(end);
        chunks.push(&text[start..break_at]);
        start = break_at;
    }
    chunks
}
