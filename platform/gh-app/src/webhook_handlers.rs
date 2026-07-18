#![allow(non_snake_case)]
//! Source webhook handlers (Jira / Confluence / Google Docs) that translate
//! inbound edit events into drift-detection runs, with HMAC signature
//! verification, Redis-backed idempotency, and real alert transports.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::workflows::{DriftDetectionRequest, SpecDriftEngine};

// ---------------------------------------------------------------------------
// Jira / Confluence / Google Docs payload models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JiraWebhookPayload {
    pub timestamp: u64,
    pub webhookEvent: String,
    pub issue: Option<JiraIssue>,
    pub comment: Option<JiraComment>,
    pub user: Option<JiraUser>,
    pub changelog: Option<JiraChangelog>,
}

#[derive(Debug, Deserialize)]
pub struct JiraIssue {
    pub id: String,
    pub key: String,
    pub fields: JiraFields,
}

#[derive(Debug, Deserialize)]
pub struct JiraFields {
    pub summary: String,
    pub description: Option<String>,
    pub status: JiraStatus,
    pub updated: String,
}

#[derive(Debug, Deserialize)]
pub struct JiraStatus {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct JiraComment {
    pub id: String,
    pub body: String,
    pub updated: String,
}

#[derive(Debug, Deserialize)]
pub struct JiraUser {
    pub displayName: String,
    pub emailAddress: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JiraChangelog {
    pub items: Vec<JiraChangeItem>,
}

#[derive(Debug, Deserialize)]
pub struct JiraChangeItem {
    pub field: String,
    pub fieldtype: String,
    pub fromString: Option<String>,
    pub toString: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConfluenceWebhookPayload {
    pub timestamp: u64,
    pub eventType: String,
    pub page: Option<ConfluencePage>,
    pub space: Option<ConfluenceSpace>,
    pub user: Option<ConfluenceUser>,
}

#[derive(Debug, Deserialize)]
pub struct ConfluencePage {
    pub id: String,
    pub title: String,
    pub status: String,
    pub version: ConfluenceVersion,
    pub space: ConfluenceSpace,
    pub lastModified: ConfluenceDate,
}

#[derive(Debug, Deserialize)]
pub struct ConfluenceVersion {
    pub number: i32,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConfluenceSpace {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfluenceUser {
    pub displayName: String,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfluenceDate {
    pub iso8601: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleDocsWebhookPayload {
    pub state: String,
    pub resourceId: String,
    pub resourceUri: String,
    pub token: String,
    pub expiration: String,
}

// ---------------------------------------------------------------------------
// Webhook processor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub source_type: String,
    pub webhook_secret: String,
    pub drift_detection_enabled: bool,
    pub alert_channels: Vec<String>,
}

/// Processes inbound source webhooks and dispatches drift detection.
pub struct WebhookProcessor {
    engine: SpecDriftEngine,
    redis: Option<redis::Client>,
    source_configs: HashMap<String, SourceConfig>,
}

impl WebhookProcessor {
    pub fn new(engine: SpecDriftEngine, redis: Option<redis::Client>) -> Self {
        let mut source_configs = HashMap::new();
        for source in ["jira", "confluence", "gdocs"] {
            let env_key = format!("{}_WEBHOOK_SECRET", source.to_uppercase());
            source_configs.insert(
                source.to_string(),
                SourceConfig {
                    source_type: source.to_string(),
                    webhook_secret: std::env::var(&env_key).unwrap_or_default(),
                    drift_detection_enabled: true,
                    alert_channels: vec!["slack".to_string(), "email".to_string()],
                },
            );
        }
        Self { engine, redis, source_configs }
    }

    pub async fn process_jira_webhook(&self, payload: &str, signature: &str) -> Result<()> {
        self.verify_webhook_signature(payload, signature, "jira")?;
        let jira: JiraWebhookPayload = serde_json::from_str(payload)?;
        if let Some(issue) = jira.issue {
            let request = DriftDetectionRequest {
                document_id: format!("jira:{}", issue.key),
                source_system: "jira".to_string(),
                event_type: jira.webhookEvent,
                event_id: format!("jira-{}", jira.timestamp),
                timestamp: Utc::now(),
            };
            self.dispatch_drift_detection(request).await?;
        }
        Ok(())
    }

    pub async fn process_confluence_webhook(&self, payload: &str, signature: &str) -> Result<()> {
        self.verify_webhook_signature(payload, signature, "confluence")?;
        let confluence: ConfluenceWebhookPayload = serde_json::from_str(payload)?;
        if let Some(page) = confluence.page {
            let request = DriftDetectionRequest {
                document_id: format!("confluence:{}", page.id),
                source_system: "confluence".to_string(),
                event_type: confluence.eventType,
                event_id: format!("confluence-{}", confluence.timestamp),
                timestamp: Utc::now(),
            };
            self.dispatch_drift_detection(request).await?;
        }
        Ok(())
    }

    pub async fn process_gdocs_webhook(&self, payload: &str, signature: &str) -> Result<()> {
        self.verify_webhook_signature(payload, signature, "gdocs")?;
        let gdocs: GoogleDocsWebhookPayload = serde_json::from_str(payload)?;
        let request = DriftDetectionRequest {
            document_id: format!("gdocs:{}", gdocs.resourceId),
            source_system: "gdocs".to_string(),
            event_type: "document_updated".to_string(),
            event_id: format!("gdocs-{}", gdocs.token),
            timestamp: Utc::now(),
        };
        self.dispatch_drift_detection(request).await?;
        Ok(())
    }

    /// Idempotently dispatch a drift-detection run: events already marked
    /// processed in Redis are skipped.
    async fn dispatch_drift_detection(&self, request: DriftDetectionRequest) -> Result<()> {
        let key = Self::generate_idempotent_key(&request.source_system, &request.event_id);

        if self.check_idempotency(&key).await? {
            warn!("Duplicate event ignored (idempotency key {}): {}", key, request.document_id);
            return Ok(());
        }

        let response = self.engine.detect_drift(request).await?;
        info!(
            "Drift detection dispatched for {} (drift={})",
            response.document_id, response.drift_detected
        );

        self.mark_event_processed(&key).await?;
        Ok(())
    }

    /// Verify a webhook signature using constant-time HMAC-SHA256 over the raw
    /// request body. Signatures use the `sha256=<hex>` form; a missing secret
    /// is a hard failure rather than silent acceptance.
    fn verify_webhook_signature(&self, payload: &str, signature: &str, source: &str) -> Result<()> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let config = self
            .source_configs
            .get(source)
            .ok_or_else(|| anyhow::anyhow!("Unknown source: {}", source))?;

        if config.webhook_secret.is_empty() {
            return Err(anyhow::anyhow!(
                "Webhook secret for source '{}' is not configured; refusing to accept unsigned payload",
                source
            ));
        }

        let hex_signature = signature.strip_prefix("sha256=").unwrap_or(signature);
        let provided = hex::decode(hex_signature)
            .map_err(|_| anyhow::anyhow!("Malformed webhook signature for source '{}'", source))?;

        let mut mac = Hmac::<Sha256>::new_from_slice(config.webhook_secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to initialise HMAC for source '{}': {}", source, e))?;
        mac.update(payload.as_bytes());

        mac.verify_slice(&provided).map_err(|_| {
            warn!("Webhook signature verification failed for source: {}", source);
            anyhow::anyhow!("Invalid webhook signature for source '{}'", source)
        })?;

        info!("Webhook signature verified for source: {}", source);
        Ok(())
    }

    /// Pure idempotency key: SHA-256 of `source:event_id`.
    pub fn generate_idempotent_key(source: &str, event_id: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}", source, event_id).as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Returns `true` if the event was already processed. With no Redis
    /// configured, deduplication is unavailable and the event is processed.
    pub async fn check_idempotency(&self, key: &str) -> Result<bool> {
        use redis::AsyncCommands;
        match &self.redis {
            Some(client) => {
                let mut conn = client
                    .get_async_connection()
                    .await
                    .context("failed to connect to Redis for idempotency check")?;
                let exists: bool = conn.exists(format!("idem:{}", key)).await?;
                Ok(exists)
            }
            None => Ok(false),
        }
    }

    /// Mark an event as processed with a 24h TTL.
    pub async fn mark_event_processed(&self, key: &str) -> Result<()> {
        use redis::AsyncCommands;
        if let Some(client) = &self.redis {
            let mut conn = client
                .get_async_connection()
                .await
                .context("failed to connect to Redis to mark event processed")?;
            let _: () = conn.set_ex(format!("idem:{}", key), "1", 86_400).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Alerting
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait AlertChannel: Send + Sync {
    async fn send_alert(&self, message: &str, severity: &str) -> Result<()>;
}

/// Slack alert channel: posts to an incoming-webhook URL with bounded retries.
pub struct SlackAlertChannel {
    webhook_url: String,
    client: reqwest::Client,
}

impl SlackAlertChannel {
    pub fn new(webhook_url: String) -> Self {
        Self { webhook_url, client: reqwest::Client::new() }
    }
}

#[async_trait::async_trait]
impl AlertChannel for SlackAlertChannel {
    async fn send_alert(&self, message: &str, severity: &str) -> Result<()> {
        let body = serde_json::json!({ "text": format!("[{}] {}", severity.to_uppercase(), message) });

        let mut last_err = None;
        for attempt in 0..3u32 {
            match self.client.post(&self.webhook_url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) => last_err = Some(anyhow::anyhow!("Slack returned {}", resp.status())),
                Err(e) => last_err = Some(anyhow::anyhow!("Slack request failed: {}", e)),
            }
            // Exponential backoff: 200ms, 400ms.
            tokio::time::sleep(std::time::Duration::from_millis(200 * 2u64.pow(attempt))).await;
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Slack alert failed")))
    }
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: String,
}

/// SMTP email alert channel. The actual transport is compiled only with the
/// `email-smtp` feature (which pulls in `lettre`); without it, sending returns
/// a typed "not configured" error rather than silently succeeding.
pub struct EmailAlertChannel {
    #[allow(dead_code)]
    smtp_config: SmtpConfig,
}

impl EmailAlertChannel {
    pub fn new(smtp_config: SmtpConfig) -> Self {
        Self { smtp_config }
    }
}

#[async_trait::async_trait]
impl AlertChannel for EmailAlertChannel {
    #[cfg(feature = "email-smtp")]
    async fn send_alert(&self, message: &str, severity: &str) -> Result<()> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        let email = Message::builder()
            .from(self.smtp_config.from.parse().context("invalid from address")?)
            .to(self.smtp_config.to.parse().context("invalid to address")?)
            .subject(format!("[{}] Spec-to-Proof alert", severity.to_uppercase()))
            .body(message.to_string())
            .context("failed to build email")?;

        let creds = Credentials::new(
            self.smtp_config.username.clone(),
            self.smtp_config.password.clone(),
        );
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&self.smtp_config.host)
            .context("failed to build SMTP transport")?
            .port(self.smtp_config.port)
            .credentials(creds)
            .build();

        mailer.send(email).await.context("failed to send email")?;
        Ok(())
    }

    #[cfg(not(feature = "email-smtp"))]
    async fn send_alert(&self, _message: &str, _severity: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "email alerts are not configured: rebuild with the `email-smtp` feature to enable SMTP delivery"
        ))
    }
}

pub struct AlertManager {
    alert_channels: HashMap<String, Box<dyn AlertChannel>>,
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertManager {
    pub fn new() -> Self {
        let mut alert_channels: HashMap<String, Box<dyn AlertChannel>> = HashMap::new();

        if let Ok(webhook_url) = std::env::var("SLACK_WEBHOOK_URL") {
            alert_channels.insert("slack".to_string(), Box::new(SlackAlertChannel::new(webhook_url)));
        }

        if let (Ok(host), Ok(username), Ok(password)) = (
            std::env::var("SMTP_HOST"),
            std::env::var("SMTP_USERNAME"),
            std::env::var("SMTP_PASSWORD"),
        ) {
            let smtp_config = SmtpConfig {
                host,
                port: std::env::var("SMTP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(587),
                username,
                password,
                from: std::env::var("SMTP_FROM").unwrap_or_else(|_| "noreply@spec-to-proof.local".to_string()),
                to: std::env::var("SMTP_TO").unwrap_or_default(),
            };
            alert_channels.insert("email".to_string(), Box::new(EmailAlertChannel::new(smtp_config)));
        }

        Self { alert_channels }
    }

    pub async fn send_drift_alert(&self, document_id: &str, message: &str, severity: &str) -> Result<()> {
        let alert_message = format!("Drift Alert - Document: {} - {}", document_id, message);
        for (channel_name, channel) in &self.alert_channels {
            if let Err(e) = channel.send_alert(&alert_message, severity).await {
                error!("Failed to send alert via {}: {}", channel_name, e);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jira_webhook_payload_parses() {
        let payload = r#"{
            "timestamp": 1234567890,
            "webhookEvent": "jira:issue_updated",
            "issue": {
                "id": "12345",
                "key": "PROJ-123",
                "fields": {
                    "summary": "Test Issue",
                    "description": "Test Description",
                    "status": {"name": "In Progress"},
                    "updated": "2024-01-01T12:00:00Z"
                }
            }
        }"#;

        let parsed: JiraWebhookPayload = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.webhookEvent, "jira:issue_updated");
        let issue = parsed.issue.expect("issue should be present");
        assert_eq!(issue.key, "PROJ-123");
        assert_eq!(issue.fields.status.name, "In Progress");
    }

    #[test]
    fn test_idempotency_key_generation() {
        let key1 = WebhookProcessor::generate_idempotent_key("jira", "event-1");
        let key2 = WebhookProcessor::generate_idempotent_key("jira", "event-1");
        let key3 = WebhookProcessor::generate_idempotent_key("jira", "event-2");
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1.len(), 64);
    }

    #[tokio::test]
    async fn test_email_not_configured_without_feature() {
        let channel = EmailAlertChannel::new(SmtpConfig {
            host: "localhost".to_string(),
            port: 587,
            username: String::new(),
            password: String::new(),
            from: "a@b.c".to_string(),
            to: "d@e.f".to_string(),
        });
        let result = channel.send_alert("hi", "info").await;
        #[cfg(not(feature = "email-smtp"))]
        assert!(result.is_err());
        // With the feature enabled this would attempt a real SMTP connection,
        // which is covered by an integration test rather than here.
        let _ = result;
    }
}
