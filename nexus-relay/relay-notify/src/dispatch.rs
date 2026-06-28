use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

use crate::apns::ApnsClient;
use crate::types::{NotificationPayload, NotifyConfig, NotifyEvent, NotifyTarget};

pub async fn run_dispatcher(mut rx: Receiver<NotifyEvent>, apns: Option<Arc<ApnsClient>>) {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    while let Some(event) = rx.recv().await {
        dispatch_event(&client, apns.as_deref(), event).await;
    }
}

async fn dispatch_event(client: &Client, apns: Option<&ApnsClient>, event: NotifyEvent) {
    let config = match &event.notify_config {
        Some(v) => match serde_json::from_value::<NotifyConfig>(v.clone()) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    ledger_id = %event.ledger_id,
                    error = %e,
                    "failed to parse notify_config, skipping notification"
                );
                return;
            }
        },
        None => {
            tracing::debug!(ledger_id = %event.ledger_id, "no notify_config, skipping notification");
            return;
        }
    };

    let preview = build_preview(&event.payload);
    let payload = NotificationPayload {
        ledger_id: event.ledger_id.to_string(),
        sequence: event.sequence,
        sender_id: event.sender_id.to_string(),
        sender_display_name: event.sender_display_name.clone(),
        msg_type: event.msg_type.clone(),
        correlation_id: event.correlation_id.map(|u| u.to_string()),
        preview,
    };

    let is_escalation = event.msg_type == "escalation";

    for target in &config.targets {
        match target {
            NotifyTarget::Webhook { url } => {
                let start = std::time::Instant::now();
                if let Err(e) = send_webhook(client, url, &payload).await {
                    tracing::warn!(
                        ledger_id = %event.ledger_id,
                        target = "webhook",
                        url = %url,
                        msg_type = %event.msg_type,
                        sequence = event.sequence,
                        duration_ms = start.elapsed().as_millis() as u64,
                        error = %e,
                        "notify_failed"
                    );
                } else {
                    tracing::info!(
                        ledger_id = %event.ledger_id,
                        target = "webhook",
                        url = %url,
                        msg_type = %event.msg_type,
                        sequence = event.sequence,
                        duration_ms = start.elapsed().as_millis() as u64,
                        "notify_delivered"
                    );
                }
            }
            NotifyTarget::Apns { device_token } => {
                if let Some(apns_client) = apns {
                    let start = std::time::Instant::now();
                    let token_short = &device_token[..device_token.len().min(8)];
                    match apns_client
                        .send_push(device_token, &payload, is_escalation)
                        .await
                    {
                        Ok(()) => {
                            tracing::info!(
                                ledger_id = %event.ledger_id,
                                target = "apns",
                                device_token_prefix = %token_short,
                                msg_type = %event.msg_type,
                                sequence = event.sequence,
                                is_escalation = is_escalation,
                                duration_ms = start.elapsed().as_millis() as u64,
                                "notify_delivered"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                ledger_id = %event.ledger_id,
                                target = "apns",
                                device_token_prefix = %token_short,
                                msg_type = %event.msg_type,
                                sequence = event.sequence,
                                is_escalation = is_escalation,
                                duration_ms = start.elapsed().as_millis() as u64,
                                error = %e,
                                "notify_failed"
                            );
                        }
                    }
                } else {
                    tracing::debug!(
                        ledger_id = %event.ledger_id,
                        "APNS not configured, skipping"
                    );
                }
            }
        }
    }
}

async fn send_webhook(
    client: &Client,
    url: &str,
    payload: &NotificationPayload,
) -> Result<(), reqwest::Error> {
    client.post(url).json(payload).send().await?;
    Ok(())
}

fn build_preview(payload: &serde_json::Value) -> String {
    if let Some(title) = payload.get("title").and_then(|v| v.as_str()) {
        return title.to_string();
    }
    let s = serde_json::to_string(payload).unwrap_or_default();
    s.chars().take(200).collect()
}
