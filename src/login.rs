use rumqttc::AsyncClient;
use std::sync::Arc;
use std::time::Instant;
use teloxide::prelude::*;
use tokio::sync::Mutex;

use crate::api::SessionState;
use crate::claude::ClaudeState;

/// Login flow state machine
#[derive(Debug, Clone, PartialEq)]
pub enum LoginState {
    /// No login in progress
    Idle,
    /// Login flow active — waiting for URL to appear
    InProgress,
    /// URL sent to Telegram, waiting for user to paste code
    WaitingForCode,
}

pub struct LoginFlow {
    pub state: LoginState,
    /// URL we sent to Telegram (to avoid resending)
    sent_url: Option<String>,
    /// When we last started a login attempt (cooldown)
    last_attempt: Option<Instant>,
}

/// Minimum seconds between login attempts
const LOGIN_COOLDOWN_SECS: u64 = 30;

impl LoginFlow {
    pub fn new() -> Self {
        Self {
            state: LoginState::Idle,
            sent_url: None,
            last_attempt: None,
        }
    }

    /// Called on each screen update. Drives the login state machine.
    /// Returns true if this screen update was consumed by the login flow.
    pub async fn on_screen_update(
        &mut self,
        bot: &Bot,
        app_state: &Arc<SessionState>,
        screen: &crate::claude::ClaudeScreen,
    ) -> bool {
        let chat_id = {
            let bs = app_state.tg_state.lock().await;
            match bs.chat_id {
                Some(id) => id,
                None => return false,
            }
        };

        let session_id = app_state.mutable.lock().await.session_id.clone();
        let mqtt = app_state.tg_state.lock().await.mqtt_client.clone();

        match self.state {
            LoginState::Idle => {
                if screen.state == ClaudeState::NotLoggedIn {
                    // Cooldown check
                    if let Some(last) = self.last_attempt {
                        if last.elapsed().as_secs() < LOGIN_COOLDOWN_SECS {
                            return true; // Still cooling down, consume but don't retry
                        }
                    }

                    tracing::info!("detected not logged in, starting login flow");
                    let _ = bot.send_message(chat_id, "Not logged in. Starting login...").await;
                    self.last_attempt = Some(Instant::now());

                    // Send /login, wait for menu, press Enter
                    send_stdin(&mqtt, &session_id, "/login\r").await;
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    send_stdin(&mqtt, &session_id, "\r").await;

                    self.state = LoginState::InProgress;
                    return true;
                }
            }
            LoginState::InProgress => {
                // Check for URL appearing on screen
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

                // Timeout: if no URL after 30s, reset
                if let Some(last) = self.last_attempt {
                    if last.elapsed().as_secs() > 30 {
                        tracing::warn!("login flow timed out waiting for URL");
                        let _ = bot.send_message(chat_id, "Login timed out. Send any message to retry.").await;
                        self.reset();
                    }
                }

                return true; // Consume all updates while in progress
            }
            LoginState::WaitingForCode => {
                // Check if login succeeded — need to press Escape to dismiss the success screen
                if screen.login_success {
                    tracing::info!("login successful, pressing escape to dismiss");
                    let _ = bot.send_message(chat_id, "Logged in!").await;
                    send_stdin(&mqtt, &session_id, "\x1b").await;
                    self.reset();
                    return false;
                }
                // Also check if we're back to idle (login completed and auto-dismissed)
                if screen.state == ClaudeState::Idle {
                    tracing::info!("login complete");
                    let _ = bot.send_message(chat_id, "Logged in!").await;
                    self.reset();
                    return false;
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
