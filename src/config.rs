use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub telegram: TelegramConfig,
    #[serde(default = "default_api_config")]
    pub api: ApiConfig,
    /// Session ID to bridge (single session for now)
    pub session_id: String,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
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

#[derive(Debug, Deserialize)]
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
    /// Hermytt registry URL for service announcement
    pub hermytt_registry: Option<String>,
    /// How hermytt can reach grytti's API (e.g. "http://10.11.0.5:7780")
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
        if let Ok(token) = std::env::var("GRYTTI_TG_TOKEN") {
            config.telegram.bot_token = token;
        }

        Ok(config)
    }
}
