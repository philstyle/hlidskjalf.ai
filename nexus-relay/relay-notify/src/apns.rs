use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::NotificationPayload;

const TOKEN_REFRESH_SECS: u64 = 50 * 60; // refresh after 50 minutes (tokens valid for 1 hour)

#[derive(Clone)]
pub struct ApnsConfig {
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
    pub sandbox: bool,
    encoding_key: EncodingKey,
}

impl ApnsConfig {
    pub fn from_env() -> Option<Self> {
        let key_path = std::env::var("APNS_KEY_PATH").ok()?;
        let key_id = std::env::var("APNS_KEY_ID").ok()?;
        let team_id = std::env::var("APNS_TEAM_ID").ok()?;
        let topic =
            std::env::var("APNS_TOPIC").unwrap_or_else(|_| "com.skynexus.nexuscomms".to_string());
        let sandbox = std::env::var("APNS_SANDBOX")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let pem = std::fs::read(&key_path)
            .inspect_err(
                |e| tracing::error!(path = %key_path, error = %e, "failed to read APNS key file"),
            )
            .ok()?;

        let encoding_key = EncodingKey::from_ec_pem(&pem)
            .inspect_err(|e| tracing::error!(error = %e, "failed to parse APNS P8 key"))
            .ok()?;

        tracing::info!(
            key_id = %key_id,
            team_id = %team_id,
            topic = %topic,
            sandbox = sandbox,
            "APNS configured"
        );

        Some(ApnsConfig {
            key_id,
            team_id,
            topic,
            sandbox,
            encoding_key,
        })
    }

    fn base_url(&self) -> &str {
        if self.sandbox {
            "https://api.sandbox.push.apple.com"
        } else {
            "https://api.push.apple.com"
        }
    }
}

pub struct ApnsClient {
    config: ApnsConfig,
    client: Client,
    cached_token: Mutex<Option<CachedToken>>,
}

struct CachedToken {
    jwt: String,
    issued_at: u64,
}

impl ApnsClient {
    pub fn new(config: ApnsConfig) -> Self {
        let client = Client::builder()
            .http2_prior_knowledge()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        ApnsClient {
            config,
            client,
            cached_token: Mutex::new(None),
        }
    }

    fn get_token(&self) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();

        // Check cache
        {
            let cache = self.cached_token.lock().unwrap();
            if let Some(ref cached) = *cache
                && now - cached.issued_at < TOKEN_REFRESH_SECS
            {
                return Ok(cached.jwt.clone());
            }
        }

        // Generate new token
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());

        let claims = ApnsClaims {
            iss: &self.config.team_id,
            iat: now,
        };

        let jwt = jsonwebtoken::encode(&header, &claims, &self.config.encoding_key)
            .map_err(|e| format!("JWT signing failed: {}", e))?;

        let mut cache = self.cached_token.lock().unwrap();
        *cache = Some(CachedToken {
            jwt: jwt.clone(),
            issued_at: now,
        });

        Ok(jwt)
    }

    pub async fn send_push(
        &self,
        device_token: &str,
        payload: &NotificationPayload,
        is_escalation: bool,
    ) -> Result<(), String> {
        let token = self.get_token()?;
        let url = format!("{}/3/device/{}", self.config.base_url(), device_token);

        let title = format!("From {}", payload.sender_display_name);
        let body = if payload.preview.is_empty() {
            format!("[{}]", payload.msg_type)
        } else {
            payload.preview.clone()
        };

        let apns_payload = ApnsPayload {
            aps: ApsPayload {
                alert: ApsAlert { title, body },
                sound: "default".to_string(),
                badge: 1,
                interruption_level: if is_escalation {
                    Some("time-sensitive".to_string())
                } else {
                    None
                },
            },
            entry_id: &payload.ledger_id,
            sequence: payload.sequence,
            msg_type: &payload.msg_type,
        };

        let response = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {}", token))
            .header("apns-topic", &self.config.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", if is_escalation { "10" } else { "5" })
            .json(&apns_payload)
            .send()
            .await
            .map_err(|e| format!("APNS request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "no body".to_string());
            return Err(format!("APNS returned {}: {}", status, body));
        }

        Ok(())
    }
}

#[derive(Serialize)]
struct ApnsClaims<'a> {
    iss: &'a str,
    iat: u64,
}

#[derive(Serialize)]
struct ApnsPayload<'a> {
    aps: ApsPayload,
    entry_id: &'a str,
    sequence: i64,
    msg_type: &'a str,
}

#[derive(Serialize)]
struct ApsPayload {
    alert: ApsAlert,
    sound: String,
    badge: u32,
    #[serde(rename = "interruption-level", skip_serializing_if = "Option::is_none")]
    interruption_level: Option<String>,
}

#[derive(Serialize)]
struct ApsAlert {
    title: String,
    body: String,
}
