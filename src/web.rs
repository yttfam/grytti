use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use futures::stream::StreamExt;
use futures::SinkExt;
use tokio::sync::mpsc;

use crate::api::GlobalState;

const CHAT_HTML: &str = include_str!("../static/chat.html");

pub fn router(state: Arc<GlobalState>) -> Router {
    Router::new()
        .route("/chat/:session_id", get(chat_page))
        .route("/chat", get(chat_index.clone()))
        .route("/chat/", get(chat_index))
        .route("/ws/:session_id", get(ws_handler))
        .with_state(state)
}

async fn chat_index(State(state): State<Arc<GlobalState>>) -> Html<String> {
    let sessions = state.sessions.lock().await;
    let mut html = String::from(
        "<html><body style='font-family:monospace;background:#0d1117;color:#c9d1d9;padding:2em'>\
         <h2 style='color:#58a6ff'>grytti sessions</h2><ul style='line-height:2'>",
    );
    for (key, ss) in sessions.iter() {
        let ms = ss.mutable.lock().await;
        let mode = if ss.tg_bot.lock().await.is_some() { "telegram" } else { "headless" };
        html.push_str(&format!(
            "<li><a href='/chat/{}' style='color:#7dd3fc'>{}</a> \
             <span style='color:#8b949e'>({})</span></li>",
            key, ms.session_id, mode
        ));
    }
    html.push_str("</ul></body></html>");
    Html(html)
}

async fn chat_page(Path(_session_id): Path<String>) -> Html<&'static str> {
    Html(CHAT_HTML)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<Arc<GlobalState>>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_ws(socket, session_id, state))
}

async fn handle_ws(socket: WebSocket, session_id: String, state: Arc<GlobalState>) {
    let ss = {
        let sessions = state.sessions.lock().await;
        sessions.get(&session_id).cloned()
    };

    let ss = match ss {
        Some(s) => s,
        None => return,
    };

    let (mut sender, mut receiver) = socket.split();

    // Channel for push messages to browser
    let (push_tx, mut push_rx) = mpsc::channel::<String>(64);

    // Send initial snapshot
    {
        let snap = ss.last_snapshot.lock().await.clone();
        if !snap.is_empty() {
            let screen = crate::claude::parse_screen(&snap);
            let br = ss.bridge.lock().await;
            let msg = serde_json::json!({
                "type": "snapshot",
                "state": state_str(&br.last_state),
                "process": proc_str(&br.last_process),
                "response": screen.response,
            });
            let _ = sender.send(Message::Text(msg.to_string().into())).await;
        }
    }

    // Push task: poll bridge events, not raw snapshots
    let ss_push = ss.clone();
    let push_handle = tokio::spawn(async move {
        let mut last_snap = String::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let snap = ss_push.last_snapshot.lock().await.clone();
            if snap == last_snap || snap.is_empty() {
                // No snapshot change — still send state for typing indicator
                let br = ss_push.bridge.lock().await;
                if br.last_state == crate::claude::ClaudeState::Thinking {
                    let msg = serde_json::json!({
                        "type": "state",
                        "state": "thinking",
                    });
                    if push_tx.send(msg.to_string()).await.is_err() { break; }
                }
                continue;
            }

            let screen = crate::claude::parse_screen(&snap);

            // Run through bridge for event filtering
            let mut br = ss_push.bridge.lock().await;
            let events = br.on_screen_update(&screen);
            let st = state_str(&br.last_state);
            let pr = proc_str(&br.last_process);
            drop(br);

            // Always send state updates
            let state_msg = serde_json::json!({
                "type": "state",
                "state": st,
                "process": pr,
            });
            if push_tx.send(state_msg.to_string()).await.is_err() { break; }

            // Only send response/shell output from bridge events
            for event in &events {
                let msg = match event {
                    crate::bridge::BridgeEvent::Response(text) => {
                        serde_json::json!({ "type": "response", "response": text })
                    }
                    crate::bridge::BridgeEvent::ShellOutput(text) => {
                        serde_json::json!({ "type": "response", "response": text })
                    }
                    crate::bridge::BridgeEvent::ProcessChanged(p) => {
                        serde_json::json!({ "type": "process", "process": proc_str(p) })
                    }
                    crate::bridge::BridgeEvent::Thinking => continue,
                };
                if push_tx.send(msg.to_string()).await.is_err() { break; }
            }

            last_snap = snap;
        }
    });

    let mqtt = state.mqtt_client.clone();
    let ss_recv = ss.clone();

    // Main loop: bridge push channel to sender, receive from browser
    loop {
        tokio::select! {
            msg = push_rx.recv() => {
                match msg {
                    Some(text) => {
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let sid = ss_recv.mutable.lock().await.session_id.clone();
                        let mut data = text.as_bytes().to_vec();
                        data.push(b'\r');
                        let _ = mqtt.publish(
                            &format!("hermytt/{}/pty/in", sid),
                            rumqttc::QoS::AtMostOnce,
                            false,
                            data,
                        ).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    push_handle.abort();
}

fn state_str(s: &crate::claude::ClaudeState) -> &'static str {
    match s {
        crate::claude::ClaudeState::Idle => "idle",
        crate::claude::ClaudeState::Thinking => "thinking",
        crate::claude::ClaudeState::NotLoggedIn => "not_logged_in",
        crate::claude::ClaudeState::LoginPrompt => "login_prompt",
        crate::claude::ClaudeState::ToolUse => "tool_use",
        crate::claude::ClaudeState::Unknown => "unknown",
    }
}

fn proc_str(p: &crate::claude::DetectedProcess) -> &'static str {
    match p {
        crate::claude::DetectedProcess::ClaudeCode => "claude_code",
        crate::claude::DetectedProcess::Shell => "shell",
        crate::claude::DetectedProcess::Unknown => "unknown",
    }
}
