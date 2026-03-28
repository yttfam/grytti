use rumqttc::AsyncClient;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

use crate::api::AppState;
use crate::claude::ClaudeState;

/// Login flow state machine
#[derive(Debug, Clone, PartialEq)]
pub enum LoginState {
    /// No login in progress
    Idle,
    /// Sent /login, waiting for menu
    WaitingForMenu,
    /// Menu visible, about to press Enter
    SelectingOption,
    /// Waiting for OAuth URL to appear
    WaitingForUrl,
    /// URL sent to Telegram, waiting for user to paste code
    WaitingForCode,
}

pub struct LoginFlow {
    pub state: LoginState,
    /// URL we sent to Telegram (to avoid resending)
    sent_url: Option<String>,
}

impl LoginFlow {
    pub fn new() -> Self {
        Self {
            state: LoginState::Idle,
            sent_url: None,
        }
    }

    /// Called on each screen update. Drives the login state machine.
    /// Returns true if this screen update was consumed by the login flow.
    pub async fn on_screen_update(
        &mut self,
        bot: &Bot,
        app_state: &Arc<AppState>,
        screen: &crate::claude::ClaudeScreen,
    ) -> bool {
        let chat_id = {
            let bs = app_state.bot_state.lock().await;
            match bs.chat_id {
                Some(id) => id,
                None => return false,
            }
        };

        let session_id = app_state.mutable.lock().await.session_id.clone();
        let mqtt = app_state.bot_state.lock().await.mqtt_client.clone();

        match self.state {
            LoginState::Idle => {
                // Detect "Not logged in" and auto-start login
                if screen.state == ClaudeState::NotLoggedIn {
                    tracing::info!("detected not logged in, starting login flow");
                    let _ = bot.send_message(chat_id, "Not logged in. Starting login...").await;
                    send_stdin(&mqtt, &session_id, "/login\r").await;
                    self.state = LoginState::WaitingForMenu;
                    return true;
                }
            }
            LoginState::WaitingForMenu => {
                if screen.state == ClaudeState::LoginPrompt {
                    tracing::info!("login menu visible, selecting option 1");
                    // Small delay then press Enter (option 1 is pre-selected)
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    send_stdin(&mqtt, &session_id, "\r").await;
                    self.state = LoginState::SelectingOption;
                    return true;
                }
            }
            LoginState::SelectingOption => {
                // Wait for URL or "Opening browser" text
                if screen.login_url.is_some() || screen.awaiting_code {
                    self.state = LoginState::WaitingForUrl;
                }
                return true;
            }
            LoginState::WaitingForUrl => {
                if let Some(ref url) = screen.login_url {
                    if self.sent_url.as_ref() != Some(url) {
                        tracing::info!(url_len = url.len(), "sending login URL to telegram");
                        let msg = format!(
                            "Open this URL to sign in:\n\n{}\n\nPaste the code here when done.",
                            url
                        );
                        let _ = bot.send_message(chat_id, msg).await;
                        self.sent_url = Some(url.clone());
                        self.state = LoginState::WaitingForCode;
                    }
                }
                return true;
            }
            LoginState::WaitingForCode => {
                // Code will be handled by the TG message handler — it detects
                // we're in WaitingForCode and forwards the message as the code.
                // Once Claude is logged in, we'll see Idle state.
                if screen.state == ClaudeState::Idle {
                    tracing::info!("login complete");
                    let _ = bot.send_message(chat_id, "Logged in!").await;
                    self.reset();
                }
                return true;
            }
        }

        false
    }

    pub fn is_waiting_for_code(&self) -> bool {
        self.state == LoginState::WaitingForCode
    }

    pub fn reset(&mut self) {
        self.state = LoginState::Idle;
        self.sent_url = None;
    }
}

async fn send_stdin(mqtt: &AsyncClient, session_id: &str, data: &str) {
    if let Err(e) = mqtt
        .publish(
            &format!("hermytt/{}/pty/in", session_id),
            rumqttc::QoS::AtMostOnce,
            false,
            data.as_bytes().to_vec(),
        )
        .await
    {
        tracing::error!("failed to send stdin: {}", e);
    }
}
