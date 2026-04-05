use std::sync::Arc;

use anyhow::Result;
use rumqttc::AsyncClient;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tokio::sync::Mutex;

use crate::api::SessionState;
use crate::bridge::BridgeEvent;
use crate::claude::DetectedProcess;

/// Telegram transport — dumb pipe. Receives events, sends messages.
/// Knows nothing about Claude state, screen parsing, or response extraction.

/// Minimal state for the TG bot handler
pub struct TgState {
    pub mqtt_client: AsyncClient,
    pub session_id: String,
    pub chat_id: Option<ChatId>,
}

pub async fn run_bot(
    token: &str,
    state: Arc<Mutex<TgState>>,
    session_state: Arc<SessionState>,
    mqtt_client: AsyncClient,
) -> Result<()> {
    let bot = Bot::new(token);

    let handler = Update::filter_message().endpoint(
        move |bot: Bot, msg: Message, state: Arc<Mutex<TgState>>| {
            let ss = session_state.clone();
            let mqtt = mqtt_client.clone();
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
                    let lf = ss.login_flow.lock().await;
                    lf.is_waiting_for_code()
                };

                let session_id = ss.mutable.lock().await.session_id.clone();

                if is_waiting_for_code {
                    tracing::info!("forwarding auth code to Claude");
                    let _ = bot.send_message(msg.chat.id, "Sending auth code...").await;
                }

                // Check if we're in a permission prompt — single digit = option selection
                let in_permission = {
                    let br = ss.bridge.lock().await;
                    br.last_state == crate::claude::ClaudeState::PermissionPrompt
                };

                if in_permission {
                    let trimmed = text.trim();
                    // Accept digit or y/n aliases
                    let digit = match trimmed {
                        "y" | "Y" => Some("1"),
                        "n" | "N" => Some("3"), // No is typically option 3
                        d if d.len() == 1 && d.chars().next().unwrap().is_ascii_digit() => Some(d),
                        _ => None,
                    };
                    if let Some(d) = digit {
                        tracing::info!(option = d, "forwarding permission choice");
                        let _ = mqtt
                            .publish(
                                &format!("hermytt/{}/pty/in", session_id),
                                rumqttc::QoS::AtMostOnce,
                                false,
                                d.as_bytes().to_vec(),
                            )
                            .await;
                        return respond(());
                    }
                }

                // Regular message → stdin with carriage return
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

                Ok(())
            }
        },
    );

    Dispatcher::builder(bot.clone(), handler)
        .dependencies(teloxide::dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Handle a bridge event — just send to Telegram. No logic.
pub async fn handle_event(bot: &Bot, chat_id: ChatId, event: &BridgeEvent) {
    match event {
        BridgeEvent::Response(text) => {
            tracing::info!(len = text.len(), "sending response to telegram");
            for chunk in chunk_message(text, 4096) {
                let _ = bot.send_message(chat_id, chunk).await;
            }
        }
        BridgeEvent::Thinking => {
            let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        }
        BridgeEvent::ProcessChanged(process) => {
            let label = match process {
                DetectedProcess::ClaudeCode => "Claude Code",
                DetectedProcess::Shell => "Shell",
                DetectedProcess::Unknown => return,
            };
            let _ = bot.send_message(chat_id, format!("[{}]", label)).await;
        }
        BridgeEvent::ShellOutput(text) => {
            if !text.trim().is_empty() {
                tracing::info!(len = text.len(), "sending shell output to telegram");
                for chunk in chunk_message(text, 4096) {
                    let _ = bot.send_message(chat_id, chunk).await;
                }
            }
        }
        BridgeEvent::PermissionPrompt(perm) => {
            let mut msg = format!("Permission: {} {}\n\n", perm.tool, perm.command);
            for (i, opt) in perm.options.iter().enumerate() {
                msg.push_str(&format!("{}. {}\n", i + 1, opt));
            }
            msg.push_str("\nReply with the option number.");
            tracing::info!(tool = %perm.tool, options = perm.options.len(), "sending permission prompt to telegram");
            let _ = bot.send_message(chat_id, msg).await;
        }
    }
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
