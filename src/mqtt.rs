use anyhow::Result;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::Config;

#[derive(Debug)]
pub enum Message {
    Pty(PtyMessage),
    Meta(MetaMessage),
}

#[derive(Debug)]
pub struct PtyMessage {
    pub session_id: String,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct MetaMessage {
    pub session_id: String,
    pub resize: Option<(usize, usize)>, // (cols, rows)
}

pub fn connect(
    config: &Config,
) -> Result<(
    AsyncClient,
    EventLoop,
    mpsc::Receiver<Message>,
    mpsc::Sender<Message>,
)> {
    let mut opts = MqttOptions::new(&config.mqtt.client_id, &config.mqtt.host, config.mqtt.port);
    opts.set_keep_alive(Duration::from_secs(30));
    if let (Some(user), Some(pass)) = (&config.mqtt.username, &config.mqtt.password) {
        opts.set_credentials(user, pass);
    }

    let (client, eventloop) = AsyncClient::new(opts, 256);
    let (tx, rx) = mpsc::channel(1024);

    Ok((client, eventloop, rx, tx))
}

/// Subscribe to PTY output topics.
pub async fn subscribe(client: &AsyncClient, sessions: &[String]) -> Result<()> {
    if sessions.is_empty() {
        client
            .subscribe("hermytt/+/pty/out", QoS::AtMostOnce)
            .await?;
        tracing::info!("subscribed to hermytt/+/pty/out");
    } else {
        for session in sessions {
            let topic = format!("hermytt/{}/pty/out", session);
            client.subscribe(&topic, QoS::AtMostOnce).await?;
            tracing::info!("subscribed to {}", topic);
        }
    }
    Ok(())
}

/// Subscribe to meta topics (resize events).
pub async fn subscribe_meta(client: &AsyncClient, sessions: &[String]) -> Result<()> {
    if sessions.is_empty() {
        client
            .subscribe("hermytt/+/meta", QoS::AtMostOnce)
            .await?;
        tracing::info!("subscribed to hermytt/+/meta");
    } else {
        for session in sessions {
            let topic = format!("hermytt/{}/meta", session);
            client.subscribe(&topic, QoS::AtMostOnce).await?;
            tracing::info!("subscribed to {}", topic);
        }
    }
    Ok(())
}

/// Run the MQTT event loop, dispatching messages to the channel.
pub async fn run_eventloop(mut eventloop: EventLoop, tx: mpsc::Sender<Message>) -> Result<()> {
    loop {
        let event = eventloop.poll().await?;
        if let rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish)) = event {
            let msg = parse_topic(&publish.topic, publish.payload.to_vec());
            if let Some(msg) = msg {
                if tx.send(msg).await.is_err() {
                    tracing::warn!("receiver dropped, stopping mqtt loop");
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Send raw bytes to a session's PTY stdin.
pub async fn send_stdin(client: &AsyncClient, session_id: &str, data: &[u8]) -> Result<()> {
    let topic = format!("hermytt/{}/pty/in", session_id);
    client
        .publish(&topic, QoS::AtMostOnce, false, data)
        .await?;
    tracing::debug!(session = %session_id, bytes = data.len(), "sent stdin");
    Ok(())
}

/// Send a line of text to a session's PTY stdin (appends newline).
pub async fn send_line(client: &AsyncClient, session_id: &str, line: &str) -> Result<()> {
    let mut data = line.as_bytes().to_vec();
    data.push(b'\n');
    send_stdin(client, session_id, &data).await
}

fn parse_topic(topic: &str, payload: Vec<u8>) -> Option<Message> {
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() < 2 || parts[0] != "hermytt" {
        return None;
    }
    let session_id = parts[1].to_string();

    match parts.get(2..) {
        Some(&["pty", "out"]) => Some(Message::Pty(PtyMessage {
            session_id,
            payload,
        })),
        Some(&["meta"]) => {
            // Parse meta payload as JSON for resize events
            let resize = serde_json::from_slice::<serde_json::Value>(&payload)
                .ok()
                .and_then(|v| {
                    let cols = v.get("cols")?.as_u64()? as usize;
                    let rows = v.get("rows")?.as_u64()? as usize;
                    Some((cols, rows))
                });
            Some(Message::Meta(MetaMessage {
                session_id,
                resize,
            }))
        }
        _ => None,
    }
}
