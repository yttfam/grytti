use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::claude::ClaudeState;
use crate::telegram::BotState;

/// Shared app state for the API
pub struct AppState {
    pub bot_state: Arc<Mutex<BotState>>,
    pub mutable: Mutex<MutableState>,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub start_time: std::time::Instant,
    pub messages_processed: AtomicU64,
    pub last_snapshot: Mutex<String>,
    pub login_flow: Mutex<crate::login::LoginFlow>,
}

pub struct MutableState {
    pub session_id: String,
    pub debounce_ms: u64,
}

#[derive(Serialize)]
struct StatusResponse {
    session_id: String,
    uptime_secs: u64,
    claude_state: String,
    telegram_chat_id: Option<i64>,
    debounce_ms: u64,
    messages_processed: u64,
}

#[derive(Serialize)]
struct ConfigResponse {
    session_id: String,
    debounce_ms: u64,
    mqtt_host: String,
    mqtt_port: u16,
    telegram_connected: bool,
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

#[derive(Deserialize)]
struct ConfigUpdate {
    session_id: Option<String>,
    debounce_ms: Option<u64>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/config", get(get_config).put(put_config))
        .route("/session", get(get_session))
        .route("/session/send", post(send_to_session))
        .route("/snapshot", get(get_snapshot))
        .with_state(state)
}

async fn get_snapshot(State(state): State<Arc<AppState>>) -> String {
    state.last_snapshot.lock().await.clone()
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let bot = state.bot_state.lock().await;
    let ms = state.mutable.lock().await;
    let claude_state = claude_state_str(&bot.last_state);

    Json(StatusResponse {
        session_id: ms.session_id.clone(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        claude_state: claude_state.to_string(),
        telegram_chat_id: bot.chat_id.map(|c| c.0),
        debounce_ms: ms.debounce_ms,
        messages_processed: state.messages_processed.load(Ordering::Relaxed),
    })
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<ConfigResponse> {
    let bot = state.bot_state.lock().await;
    let ms = state.mutable.lock().await;
    Json(ConfigResponse {
        session_id: ms.session_id.clone(),
        debounce_ms: ms.debounce_ms,
        mqtt_host: state.mqtt_host.clone(),
        mqtt_port: state.mqtt_port,
        telegram_connected: bot.chat_id.is_some(),
    })
}

async fn put_config(
    State(state): State<Arc<AppState>>,
    Json(update): Json<ConfigUpdate>,
) -> StatusCode {
    let mut ms = state.mutable.lock().await;
    if let Some(sid) = update.session_id {
        tracing::info!(session_id = %sid, "session_id updated via API");
        ms.session_id = sid;
    }
    if let Some(d) = update.debounce_ms {
        tracing::info!(debounce_ms = d, "debounce_ms updated via API");
        ms.debounce_ms = d;
    }
    StatusCode::OK
}

async fn get_session(State(state): State<Arc<AppState>>) -> Json<SessionResponse> {
    let bot = state.bot_state.lock().await;
    let ms = state.mutable.lock().await;
    let claude_state = claude_state_str(&bot.last_state);

    Json(SessionResponse {
        session_id: ms.session_id.clone(),
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
    let ms = state.mutable.lock().await;
    let mut data = req.text.into_bytes();
    data.push(b'\r');

    match bot
        .mqtt_client
        .publish(
            &format!("hermytt/{}/pty/in", ms.session_id),
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

fn claude_state_str(state: &ClaudeState) -> &'static str {
    match state {
        ClaudeState::Idle => "idle",
        ClaudeState::Thinking => "thinking",
        ClaudeState::ToolUse => "tool_use",
        ClaudeState::NotLoggedIn => "not_logged_in",
        ClaudeState::LoginPrompt => "login_prompt",
        ClaudeState::Unknown => "unknown",
    }
}

/// Announce grytti to hermytt's service registry
pub async fn announce_to_registry(registry_url: &str, endpoint: &str, token: Option<&str>) {
    let body = serde_json::json!({
        "name": "grytti",
        "role": "parser",
        "endpoint": endpoint,
        "meta": {
            "host": hostname(),
            "version": env!("CARGO_PKG_VERSION"),
        }
    });

    let client = reqwest::Client::new();
    let mut req = client.post(&format!("{}/registry/announce", registry_url));
    if let Some(t) = token {
        req = req.header("X-Hermytt-Key", t);
    }
    match req.json(&body).send().await {
        Ok(resp) => {
            tracing::info!(status = %resp.status(), "announced to hermytt registry");
        }
        Err(e) => {
            tracing::warn!("failed to announce to registry: {}", e);
        }
    }
}

/// Run heartbeat loop — re-announce every 15s
pub async fn heartbeat_loop(registry_url: String, endpoint: String, token: Option<String>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        interval.tick().await;
        announce_to_registry(&registry_url, &endpoint, token.as_deref()).await;
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}
