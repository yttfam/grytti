use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub mqtt: MqttConfig,
    #[serde(default = "default_api_config")]
    pub api: ApiConfig,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,

    // Single-session backwards compat
    pub session_id: Option<String>,
    pub telegram: Option<TelegramConfig>,

    // Multi-session
    #[serde(default)]
    pub sessions: Vec<SessionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    pub session_id: String,
    pub telegram: TelegramConfig,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
    /// Persisted chat ID so responses work immediately after restart
    pub chat_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MqttConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_chats: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_api_bind")]
    pub bind: String,
    #[serde(default = "default_api_port")]
    pub port: u16,
    pub hermytt_registry: Option<String>,
    pub hermytt_token: Option<String>,
    pub endpoint: Option<String>,
}

fn default_port() -> u16 {
    1883
}

fn default_client_id() -> String {
    format!("grytti-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

fn default_debounce() -> u64 {
    200
}

fn default_api_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_api_port() -> u16 {
    7780
}

fn default_api_config() -> ApiConfig {
    ApiConfig {
        bind: default_api_bind(),
        port: default_api_port(),
        hermytt_registry: None,
        hermytt_token: None,
        endpoint: None,
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        if let Ok(user) = std::env::var("GRYTTI_MQTT_USER") {
            config.mqtt.username = Some(user);
        }
        if let Ok(pass) = std::env::var("GRYTTI_MQTT_PASS") {
            config.mqtt.password = Some(pass);
        }

        Ok(config)
    }

    /// Resolve session configs — supports both single-session and multi-session formats
    pub fn resolved_sessions(&self) -> Vec<SessionConfig> {
        if !self.sessions.is_empty() {
            return self.sessions.clone();
        }
        // Single-session backwards compat
        if let (Some(ref sid), Some(ref tg)) = (&self.session_id, &self.telegram) {
            vec![SessionConfig {
                session_id: sid.clone(),
                telegram: tg.clone(),
                debounce_ms: self.debounce_ms,
                chat_id: None,
            }]
        } else {
            vec![]
        }
    }
}
