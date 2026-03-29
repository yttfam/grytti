use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put, delete};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::claude::{ClaudeState, DetectedProcess};
use crate::telegram::BotState;

/// Global state shared across API, main loop, and TG bots
pub struct GlobalState {
    pub sessions: Mutex<HashMap<String, Arc<SessionState>>>,
    pub mqtt_client: rumqttc::AsyncClient,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub start_time: std::time::Instant,
    pub config_path: std::path::PathBuf,
    /// Bot tokens per session key (for config persistence)
    pub bot_tokens: Mutex<HashMap<String, String>>,
}

/// Per-session state
pub struct SessionState {
    pub bot_state: Arc<Mutex<BotState>>,
    pub mutable: Mutex<MutableState>,
    pub messages_processed: AtomicU64,
    pub last_snapshot: Mutex<String>,
    pub login_flow: Mutex<crate::login::LoginFlow>,
    /// VTE parser + grid for this session (used by main loop)
    pub runtime: Mutex<SessionRuntime>,
    /// Telegram bot instance for sending messages from the main loop
    pub tg_bot: teloxide::Bot,
}

pub struct SessionRuntime {
    pub vte_parser: vte::Parser,
    pub performer: crate::parser::GridPerformer,
    pub last_update: tokio::time::Instant,
    pub last_published: String,
}

pub struct MutableState {
    pub session_id: String,
    pub debounce_ms: u64,
}

// --- Response types ---

#[derive(Serialize)]
struct SessionListEntry {
    session_id: String,
    claude_state: String,
    process: String,
    telegram_connected: bool,
    telegram_chat_id: Option<i64>,
    messages_processed: u64,
    debounce_ms: u64,
}

#[derive(Serialize)]
struct SessionsResponse {
    sessions: Vec<SessionListEntry>,
}

#[derive(Serialize)]
struct StatusResponse {
    session_id: String,
    uptime_secs: u64,
    claude_state: String,
    process: String,
    telegram_chat_id: Option<i64>,
    debounce_ms: u64,
    messages_processed: u64,
}

#[derive(Serialize)]
struct SessionDetailResponse {
    session_id: String,
    claude_state: String,
    process: String,
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

#[derive(Deserialize)]
struct CreateSessionRequest {
    session_id: String,
    bot_token: String,
    #[serde(default = "default_debounce")]
    debounce_ms: u64,
}

fn default_debounce() -> u64 {
    200
}

#[derive(Deserialize)]
struct SessionUpdate {
    session_id: Option<String>,
    debounce_ms: Option<u64>,
    #[allow(dead_code)]
    bot_token: Option<String>, // TODO: runtime bot token change
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn router(state: Arc<GlobalState>) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/:session_id", get(get_session_detail).put(put_session).delete(delete_session))
        .route("/sessions/:session_id/send", post(send_to_session))
        .route("/sessions/:session_id/snapshot", get(get_session_snapshot))
        // Legacy single-session routes (use first session)
        .route("/config", get(get_config).put(put_config))
        .route("/session", get(get_legacy_session))
        .route("/session/send", post(send_to_legacy_session))
        .route("/snapshot", get(get_legacy_snapshot))
        .with_state(state)
}

// --- Multi-session endpoints ---

async fn create_session(
    State(state): State<Arc<GlobalState>>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<OkResponse>, (StatusCode, Json<ErrorResponse>)> {
    let mut sessions = state.sessions.lock().await;
    if sessions.contains_key(&req.session_id) {
        return Err((StatusCode::CONFLICT, Json(ErrorResponse {
            error: format!("session {} already exists", req.session_id),
        })));
    }

    let bot_state = Arc::new(tokio::sync::Mutex::new(BotState {
        mqtt_client: state.mqtt_client.clone(),
        session_id: req.session_id.clone(),
        last_state: ClaudeState::Unknown,
        last_process: crate::claude::DetectedProcess::Unknown,
        last_sent_response: String::new(),
        chat_id: None,
    }));

    let tg_bot = teloxide::Bot::new(&req.bot_token);

    let ss = Arc::new(SessionState {
        bot_state: bot_state.clone(),
        mutable: tokio::sync::Mutex::new(MutableState {
            session_id: req.session_id.clone(),
            debounce_ms: req.debounce_ms,
        }),
        messages_processed: std::sync::atomic::AtomicU64::new(0),
        last_snapshot: tokio::sync::Mutex::new(String::new()),
        login_flow: tokio::sync::Mutex::new(crate::login::LoginFlow::new()),
        runtime: tokio::sync::Mutex::new(SessionRuntime {
            vte_parser: vte::Parser::new(),
            performer: crate::parser::GridPerformer::new(crate::grid::Grid::default()),
            last_update: tokio::time::Instant::now(),
            last_published: String::new(),
        }),
        tg_bot,
    });

    // Spawn TG bot for this session
    let tg_token = req.bot_token.clone();
    let bot_state_tg = bot_state.clone();
    let ss_tg = ss.clone();
    let mqtt_client_tg = state.mqtt_client.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::telegram::run_bot(&tg_token, bot_state_tg, ss_tg, mqtt_client_tg).await {
            tracing::error!("telegram bot error: {}", e);
        }
    });

    tracing::info!(session = %req.session_id, "session created via API");
    // Store bot token for persistence
    state.bot_tokens.lock().await.insert(req.session_id.clone(), req.bot_token);
    sessions.insert(req.session_id, ss);
    drop(sessions);
    persist_sessions(&state).await;

    Ok(Json(OkResponse { ok: true }))
}

async fn list_sessions(State(state): State<Arc<GlobalState>>) -> Json<SessionsResponse> {
    let sessions = state.sessions.lock().await;
    let mut entries = Vec::new();
    for (_, ss) in sessions.iter() {
        let bot = ss.bot_state.lock().await;
        let ms = ss.mutable.lock().await;
        entries.push(SessionListEntry {
            session_id: ms.session_id.clone(),
            claude_state: claude_state_str(&bot.last_state).to_string(),
            process: process_str(&bot.last_process).to_string(),
            telegram_connected: bot.chat_id.is_some(),
            telegram_chat_id: bot.chat_id.map(|c| c.0),
            messages_processed: ss.messages_processed.load(Ordering::Relaxed),
            debounce_ms: ms.debounce_ms,
        });
    }
    Json(SessionsResponse { sessions: entries })
}

async fn get_session_detail(
    State(state): State<Arc<GlobalState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetailResponse>, StatusCode> {
    let sessions = state.sessions.lock().await;
    let ss = sessions.get(&session_id).ok_or(StatusCode::NOT_FOUND)?;
    let bot = ss.bot_state.lock().await;
    let ms = ss.mutable.lock().await;
    Ok(Json(SessionDetailResponse {
        session_id: ms.session_id.clone(),
        claude_state: claude_state_str(&bot.last_state).to_string(),
        process: process_str(&bot.last_process).to_string(),
        last_response: if bot.last_sent_response.is_empty() {
            None
        } else {
            Some(bot.last_sent_response.clone())
        },
    }))
}

async fn put_session(
    State(state): State<Arc<GlobalState>>,
    Path(session_id): Path<String>,
    Json(update): Json<SessionUpdate>,
) -> Result<Json<OkResponse>, StatusCode> {
    let ss = {
        let sessions = state.sessions.lock().await;
        sessions.get(&session_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let mut ms = ss.mutable.lock().await;
    if let Some(ref sid) = update.session_id {
        tracing::info!(session = %session_id, new_session_id = %sid, "session_id updated via API");
        ms.session_id = sid.clone();
    }
    if let Some(d) = update.debounce_ms {
        tracing::info!(session = %session_id, debounce_ms = d, "debounce updated via API");
        ms.debounce_ms = d;
    }
    drop(ms);
    persist_sessions(&state).await;
    Ok(Json(OkResponse { ok: true }))
}

async fn delete_session(
    State(state): State<Arc<GlobalState>>,
    Path(session_id): Path<String>,
) -> Result<Json<OkResponse>, StatusCode> {
    let mut sessions = state.sessions.lock().await;
    if sessions.remove(&session_id).is_some() {
        tracing::info!(session = %session_id, "session removed via API");
        state.bot_tokens.lock().await.remove(&session_id);
        drop(sessions);
        persist_sessions(&state).await;
        Ok(Json(OkResponse { ok: true }))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn send_to_session(
    State(state): State<Arc<GlobalState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SendRequest>,
) -> StatusCode {
    let sessions = state.sessions.lock().await;
    let ss = match sessions.get(&session_id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND,
    };
    let ms = ss.mutable.lock().await;
    let mut data = req.text.into_bytes();
    data.push(b'\r');
    match state.mqtt_client
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

async fn get_session_snapshot(
    State(state): State<Arc<GlobalState>>,
    Path(session_id): Path<String>,
) -> Result<String, StatusCode> {
    let ss = {
        let sessions = state.sessions.lock().await;
        sessions.get(&session_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let snap = ss.last_snapshot.lock().await.clone();
    Ok(snap)
}

// --- Legacy single-session endpoints (first session) ---

async fn get_status(State(state): State<Arc<GlobalState>>) -> Json<StatusResponse> {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        let bot = ss.bot_state.lock().await;
        let ms = ss.mutable.lock().await;
        Json(StatusResponse {
            session_id: ms.session_id.clone(),
            uptime_secs: state.start_time.elapsed().as_secs(),
            claude_state: claude_state_str(&bot.last_state).to_string(),
            process: process_str(&bot.last_process).to_string(),
            telegram_chat_id: bot.chat_id.map(|c| c.0),
            debounce_ms: ms.debounce_ms,
            messages_processed: ss.messages_processed.load(Ordering::Relaxed),
        })
    } else {
        Json(StatusResponse {
            session_id: String::new(),
            uptime_secs: state.start_time.elapsed().as_secs(),
            claude_state: "no_sessions".to_string(),
            process: "unknown".to_string(),
            telegram_chat_id: None,
            debounce_ms: 0,
            messages_processed: 0,
        })
    }
}

async fn get_config(State(state): State<Arc<GlobalState>>) -> Json<serde_json::Value> {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        let bot = ss.bot_state.lock().await;
        let ms = ss.mutable.lock().await;
        Json(serde_json::json!({
            "session_id": ms.session_id,
            "debounce_ms": ms.debounce_ms,
            "mqtt_host": state.mqtt_host,
            "mqtt_port": state.mqtt_port,
            "telegram_connected": bot.chat_id.is_some(),
        }))
    } else {
        Json(serde_json::json!({"error": "no sessions"}))
    }
}

async fn put_config(
    State(state): State<Arc<GlobalState>>,
    Json(update): Json<ConfigUpdate>,
) -> StatusCode {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        let mut ms = ss.mutable.lock().await;
        if let Some(sid) = update.session_id {
            tracing::info!(session_id = %sid, "session_id updated via API");
            ms.session_id = sid;
        }
        if let Some(d) = update.debounce_ms {
            tracing::info!(debounce_ms = d, "debounce_ms updated via API");
            ms.debounce_ms = d;
        }
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_legacy_session(State(state): State<Arc<GlobalState>>) -> Json<serde_json::Value> {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        let bot = ss.bot_state.lock().await;
        let ms = ss.mutable.lock().await;
        Json(serde_json::json!({
            "session_id": ms.session_id,
            "claude_state": claude_state_str(&bot.last_state),
            "process": process_str(&bot.last_process),
            "last_response": if bot.last_sent_response.is_empty() { None } else { Some(&bot.last_sent_response) },
        }))
    } else {
        Json(serde_json::json!({"error": "no sessions"}))
    }
}

async fn send_to_legacy_session(
    State(state): State<Arc<GlobalState>>,
    Json(req): Json<SendRequest>,
) -> StatusCode {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        let ms = ss.mutable.lock().await;
        let mut data = req.text.into_bytes();
        data.push(b'\r');
        match state.mqtt_client
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
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_legacy_snapshot(State(state): State<Arc<GlobalState>>) -> String {
    let sessions = state.sessions.lock().await;
    if let Some((_, ss)) = sessions.iter().next() {
        ss.last_snapshot.lock().await.clone()
    } else {
        String::new()
    }
}

// --- Helpers ---

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

fn process_str(p: &DetectedProcess) -> &'static str {
    match p {
        DetectedProcess::ClaudeCode => "claude_code",
        DetectedProcess::Shell => "shell",
        DetectedProcess::Unknown => "unknown",
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

/// Persist current sessions to the config file so they survive restarts.
async fn persist_sessions(state: &GlobalState) {
    let sessions = state.sessions.lock().await;
    let tokens = state.bot_tokens.lock().await;

    let mut session_configs = Vec::new();
    for (key, ss) in sessions.iter() {
        let ms = ss.mutable.lock().await;
        let bot = ss.bot_state.lock().await;
        let token = tokens.get(key).cloned().unwrap_or_default();
        let chat_id = bot.chat_id.map(|c| c.0);
        session_configs.push(serde_json::json!({
            "session_id": ms.session_id,
            "bot_token": token,
            "debounce_ms": ms.debounce_ms,
            "chat_id": chat_id,
        }));
    }
    drop(sessions);

    // Read current config, update sessions section, write back
    let config_path = &state.config_path;
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to read config for persist: {}", e);
            return;
        }
    };

    // Parse, update sessions, rewrite
    if let Ok(mut doc) = content.parse::<toml::Table>() {
        // Remove old single-session fields
        doc.remove("session_id");
        doc.remove("telegram");

        // Build sessions array
        let mut arr = toml::value::Array::new();
        for sc in &session_configs {
            let mut table = toml::Table::new();
            table.insert("session_id".into(), toml::Value::String(sc["session_id"].as_str().unwrap_or("").to_string()));
            table.insert("debounce_ms".into(), toml::Value::Integer(sc["debounce_ms"].as_u64().unwrap_or(200) as i64));

            if let Some(cid) = sc["chat_id"].as_i64() {
                table.insert("chat_id".into(), toml::Value::Integer(cid));
            }

            let mut tg = toml::Table::new();
            tg.insert("bot_token".into(), toml::Value::String(sc["bot_token"].as_str().unwrap_or("").to_string()));
            table.insert("telegram".into(), toml::Value::Table(tg));

            arr.push(toml::Value::Table(table));
        }
        doc.insert("sessions".into(), toml::Value::Array(arr));

        match std::fs::write(config_path, toml::to_string_pretty(&doc).unwrap_or_default()) {
            Ok(_) => tracing::info!("persisted {} sessions to config", session_configs.len()),
            Err(e) => tracing::warn!("failed to write config: {}", e),
        }
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}
