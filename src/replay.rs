use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::api;
use crate::bridge::Bridge;
use crate::claude;
use crate::grid::Grid;
use crate::parser::GridPerformer;
use crate::telegram::TgState;
use crate::login::LoginFlow;

/// Run grytti in replay mode — feed a .cast file through the pipeline and serve the web UI.
pub async fn run(cast_path: &str, port: u16) -> Result<()> {
    let content = std::fs::read_to_string(cast_path)?;
    let mut lines = content.lines();

    let header_str = lines.next().ok_or_else(|| anyhow::anyhow!("empty cast file"))?;
    let header: serde_json::Value = serde_json::from_str(header_str)?;
    let cols = header["width"].as_u64().unwrap_or(80) as usize;
    let rows = header["height"].as_u64().unwrap_or(24) as usize;

    tracing::info!(cols, rows, "replay: loaded {}", cast_path);

    // Collect all output events
    let mut events: Vec<(f64, Vec<u8>)> = Vec::new();
    for line in lines {
        let event: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = event[0].as_f64().unwrap_or(0.0);
        let etype = event[1].as_str().unwrap_or("");
        if etype == "o" {
            if let Some(data) = event[2].as_str() {
                events.push((ts, data.as_bytes().to_vec()));
            }
        }
    }

    tracing::info!("replay: {} output events over {:.1}s", events.len(),
        events.last().map(|(t, _)| *t).unwrap_or(0.0));

    let session_id = "replay";

    // Build a fake MQTT client — we won't actually connect
    // Use a dummy for TgState
    let tg_state = Arc::new(Mutex::new(TgState {
        mqtt_client: rumqttc::AsyncClient::new(
            rumqttc::MqttOptions::new("dummy", "127.0.0.1", 1883), 1
        ).0,
        session_id: session_id.to_string(),
        chat_id: None,
    }));

    let ss = Arc::new(api::SessionState {
        tg_state,
        bridge: Mutex::new(Bridge::new()),
        mutable: Mutex::new(api::MutableState {
            session_id: session_id.to_string(),
            debounce_ms: 200,
        }),
        messages_processed: AtomicU64::new(0),
        last_snapshot: Mutex::new(String::new()),
        login_flow: Mutex::new(LoginFlow::new()),
        runtime: Mutex::new(api::SessionRuntime {
            vte_parser: vte::Parser::new(),
            performer: GridPerformer::new(Grid::new(cols, rows)),
            last_update: Instant::now(),
            last_published: String::new(),
        }),
        tg_bot: Mutex::new(None),
    });

    let mut session_states: HashMap<String, Arc<api::SessionState>> = HashMap::new();
    session_states.insert(session_id.to_string(), ss.clone());

    let global_state = Arc::new(api::GlobalState {
        sessions: Mutex::new(session_states),
        mqtt_client: rumqttc::AsyncClient::new(
            rumqttc::MqttOptions::new("dummy2", "127.0.0.1", 1883), 1
        ).0,
        mqtt_host: "replay".to_string(),
        mqtt_port: 0,
        start_time: std::time::Instant::now(),
        config_path: std::path::PathBuf::from("/dev/null"),
        bot_tokens: Mutex::new(HashMap::new()),
    });

    // Serve web UI
    let bind = format!("0.0.0.0:{}", port);
    let router = crate::web::router(global_state.clone());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("replay: web UI at http://localhost:{}/chat/{}", port, session_id);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("web server error: {}", e);
        }
    });

    // Wait a moment for browser to connect
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Replay events at recorded speed
    let start_time = events.first().map(|(t, _)| *t).unwrap_or(0.0);
    let replay_start = Instant::now();

    for (ts, data) in &events {
        // Wait until the right time
        let elapsed = ts - start_time;
        let target = Duration::from_secs_f64(elapsed);
        let now = replay_start.elapsed();
        if target > now {
            tokio::time::sleep(target - now).await;
        }

        // Feed through parser
        {
            let mut rt = ss.runtime.lock().await;
            let api::SessionRuntime { ref mut vte_parser, ref mut performer, .. } = *rt;
            GridPerformer::feed(vte_parser, performer, data);
            rt.last_update = Instant::now();
        }

        // Update snapshot + bridge
        {
            let mut rt = ss.runtime.lock().await;
            let snapshot = rt.performer.grid.snapshot();
            if !snapshot.is_empty()
                && snapshot != rt.last_published
                && !claude::is_spinner_only_change(&rt.last_published, &snapshot)
            {
                let screen = claude::parse_screen(&snapshot);

                let mut br = ss.bridge.lock().await;
                br.on_screen_update(&screen);
                drop(br);

                *ss.last_snapshot.lock().await = snapshot.clone();
                rt.last_published = snapshot;
            }
        }

        ss.messages_processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    tracing::info!("replay: finished, web UI still running");

    // Keep serving
    tokio::signal::ctrl_c().await?;
    Ok(())
}
