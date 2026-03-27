use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::claude::ClaudeState;
use crate::telegram::BotState;

/// Shared app state for the API
pub struct AppState {
    pub bot_state: Arc<Mutex<BotState>>,
    pub session_id: String,
    pub debounce_ms: u64,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub start_time: std::time::Instant,
}

#[derive(Serialize)]
struct StatusResponse {
    session_id: String,
    uptime_secs: u64,
    claude_state: String,
    telegram_chat_id: Option<i64>,
    debounce_ms: u64,
}

#[derive(Serialize)]
struct ConfigResponse {
    session_id: String,
    debounce_ms: u64,
    mqtt_host: String,
    mqtt_port: u16,
    telegram_connected: bool,
    allowed_chats: Vec<i64>,
}

#[derive(Serialize)]
struct SessionResponse {
    session_id: String,
    claude_state: String,
    last_response: Option<String>,
}

#[derive(Deserialize)]
struct SendRequest {
    text: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/config", get(get_config))
        .route("/session", get(get_session))
        .route("/session/send", post(send_to_session))
        .with_state(state)
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let bot = state.bot_state.lock().await;
    let claude_state = match bot.last_state {
        ClaudeState::Idle => "idle",
        ClaudeState::Thinking => "thinking",
        ClaudeState::ToolUse => "tool_use",
        ClaudeState::Unknown => "unknown",
    };

    Json(StatusResponse {
        session_id: state.session_id.clone(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        claude_state: claude_state.to_string(),
        telegram_chat_id: bot.chat_id.map(|c| c.0),
        debounce_ms: state.debounce_ms,
    })
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<ConfigResponse> {
    let bot = state.bot_state.lock().await;
    Json(ConfigResponse {
        session_id: state.session_id.clone(),
        debounce_ms: state.debounce_ms,
        mqtt_host: state.mqtt_host.clone(),
        mqtt_port: state.mqtt_port,
        telegram_connected: bot.chat_id.is_some(),
        allowed_chats: vec![],
    })
}

async fn get_session(State(state): State<Arc<AppState>>) -> Json<SessionResponse> {
    let bot = state.bot_state.lock().await;
    let claude_state = match bot.last_state {
        ClaudeState::Idle => "idle",
        ClaudeState::Thinking => "thinking",
        ClaudeState::ToolUse => "tool_use",
        ClaudeState::Unknown => "unknown",
    };

    Json(SessionResponse {
        session_id: state.session_id.clone(),
        claude_state: claude_state.to_string(),
        last_response: if bot.last_sent_response.is_empty() {
            None
        } else {
            Some(bot.last_sent_response.clone())
        },
    })
}

async fn send_to_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
) -> StatusCode {
    let bot = state.bot_state.lock().await;
    let mut data = req.text.into_bytes();
    data.push(b'\r');

    match bot
        .mqtt_client
        .publish(
            &format!("hermytt/{}/pty/in", state.session_id),
            rumqttc::QoS::AtMostOnce,
            false,
            data,
        )
        .await
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Announce grytti to hermytt's service registry
pub async fn announce_to_registry(registry_url: &str, api_port: u16) {
    let body = serde_json::json!({
        "name": "grytti",
        "role": "parser",
        "endpoint": format!("http://localhost:{}", api_port),
        "capabilities": ["pty-parse", "telegram-bridge", "claude-state"],
        "version": env!("CARGO_PKG_VERSION"),
    });

    match reqwest::Client::new()
        .post(&format!("{}/registry/announce", registry_url))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            tracing::info!(status = %resp.status(), "announced to hermytt registry");
        }
        Err(e) => {
            tracing::warn!("failed to announce to registry: {}", e);
        }
    }
}
