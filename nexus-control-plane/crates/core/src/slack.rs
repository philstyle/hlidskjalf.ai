use std::process::Command;

/// Slack notification configuration, read from environment variables.
pub struct SlackConfig {
    pub bot_token: String,
    pub channel: String,
}

impl SlackConfig {
    /// Read Slack config from environment. Returns None if token/channel not set.
    pub fn from_env() -> Option<Self> {
        // Explicit opt-out
        if std::env::var("SLACK_IDLE_ENABLED").ok().as_deref() == Some("false") {
            return None;
        }

        let bot_token = std::env::var("SLACK_BOT_TOKEN").ok()?;
        let channel = std::env::var("SLACK_CHANNEL").ok()?;

        if bot_token.is_empty() || channel.is_empty() {
            return None;
        }

        Some(Self { bot_token, channel })
    }

    pub fn from_settings(settings: &crate::settings::SettingsStore) -> Option<Self> {
        if !settings.get_bool("slack_idle_enabled").unwrap_or(true) {
            return None;
        }

        let bot_token = settings.get_str("slack_bot_token")?;
        let channel = settings.get_str("slack_channel")?;

        if bot_token.is_empty() || channel.is_empty() {
            return None;
        }

        Some(Self { bot_token, channel })
    }

    /// Send a Slack notification for an idle session.
    /// Uses curl (always available on macOS) to avoid adding an HTTPS client dependency.
    pub fn send_idle_notification(
        &self,
        card_name: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let text = format!(":{}: *{}* is waiting for input", emoji, card_name);
        let body = serde_json::json!({
            "channel": self.channel,
            "text": text,
        });

        let output = Command::new("curl")
            .args([
                "-s",
                "-S",
                "-X",
                "POST",
                "https://slack.com/api/chat.postMessage",
                "-H",
                &format!("Authorization: Bearer {}", self.bot_token),
                "-H",
                "Content-Type: application/json",
                "-d",
                &body.to_string(),
            ])
            .output()
            .map_err(|e| format!("curl failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Slack API error: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Check Slack API response for ok field
        if let Ok(resp) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let err = resp
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(format!("Slack API returned error: {}", err));
            }
        }

        Ok(())
    }
}
