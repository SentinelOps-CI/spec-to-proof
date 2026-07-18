use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use aws_sdk_secretsmanager::Client as SecretsClient;
use chrono::{DateTime, Utc};
use prost::Message;
use crate::proto::spec_to_proof::v1::SpecDocument;
use crate::publisher::DocumentPublisher;
use prost_types::Timestamp;

pub mod proto;
pub mod connectors;
pub mod secrets;
pub mod rate_limiter;
pub mod backoff;
pub mod publisher;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    pub source_system: String,
    pub base_url: String,
    pub rate_limit_per_minute: u32,
    pub batch_size: usize,
    pub poll_interval_seconds: u64,
    pub secrets_arn: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at_unix: i64,
    pub token_type: String,
}

pub struct IngestionConnector {
    config: ConnectorConfig,
    secrets_client: SecretsClient,
    publisher: Arc<dyn DocumentPublisher>,
    token_cache: RwLock<HashMap<String, OAuth2Token>>,
    rate_limiter: rate_limiter::RateLimiter,
}

impl IngestionConnector {
    pub async fn new(
        config: ConnectorConfig,
        secrets_client: SecretsClient,
        publisher: Arc<dyn DocumentPublisher>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let rate_limiter = rate_limiter::RateLimiter::new(
            config.rate_limit_per_minute,
            Duration::from_secs(60),
        );

        Ok(Self {
            config,
            secrets_client,
            publisher,
            token_cache: RwLock::new(HashMap::new()),
            rate_limiter,
        })
    }

    pub async fn start_polling(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut interval = tokio::time::interval(
            Duration::from_secs(self.config.poll_interval_seconds)
        );

        loop {
            interval.tick().await;
            
            match self.poll_documents().await {
                Ok(documents) => {
                    for doc in documents {
                        self.publish_document(doc).await?;
                    }
                }
                Err(e) => {
                    tracing::error!("Error polling documents: {}", e);
                    // Exponential backoff will be handled by the connector
                }
            }
        }
    }

    async fn poll_documents(&self) -> Result<Vec<SpecDocument>, Box<dyn std::error::Error>> {
        // The generic service does not poll any source directly; a
        // source-specific connector (Jira/Confluence/Google Docs) supplies the
        // polling implementation. Return a clear error instead of `todo!()` so
        // that a misconfigured deployment fails cleanly rather than panicking
        // inside the `start_polling` loop.
        Err(format!(
            "poll_documents is not implemented for the generic ingest service (source_system='{}'); use a source-specific connector",
            self.config.source_system
        )
        .into())
    }

    pub async fn publish_document(&self, document: SpecDocument) -> Result<(), Box<dyn std::error::Error>> {
        let subject = format!("spec-documents.{}", self.config.source_system);

        let payload = document.encode_to_vec();

        self.publisher
            .publish(&subject, &payload)
            .await
            .map_err(|e| format!("Failed to publish document: {}", e))?;

        tracing::info!(
            "Published document {} to subject {}",
            document.id,
            subject
        );

        Ok(())
    }

    /// Access the rate limiter (used by the connector's polling logic and by
    /// tests that assert on rate-limit accounting).
    pub fn rate_limiter(&self) -> &rate_limiter::RateLimiter {
        &self.rate_limiter
    }

    pub async fn refresh_token(&self, token_key: &str) -> Result<OAuth2Token, Box<dyn std::error::Error>> {
        let mut cache = self.token_cache.write().await;
        
        if let Some(token) = cache.get(token_key) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if now < token.expires_at_unix {
                return Ok(token.clone());
            }
        }

        // Fetch fresh token from AWS Secrets Manager
        let secret_value = self.secrets_client
            .get_secret_value()
            .secret_id(token_key)
            .send()
            .await?;

        let token_data: OAuth2Token = serde_json::from_str(
            secret_value.secret_string()
                .ok_or("No secret string found")?
        )?;

        cache.insert(token_key.to_string(), token_data.clone());
        Ok(token_data)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub source_id: String,
    pub title: String,
    pub url: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub version: i32,
    pub status: String,
    pub metadata: HashMap<String, String>,
}

impl From<DocumentMetadata> for SpecDocument {
    fn from(metadata: DocumentMetadata) -> Self {
        SpecDocument {
            id: format!("{}-{}", metadata.source_id, metadata.version),
            content_sha256: String::new(), // Will be computed
            source_system: String::new(), // Will be set by connector
            source_id: metadata.source_id,
            title: metadata.title,
            content: String::new(), // Will be fetched separately
            url: metadata.url,
            author: metadata.author,
            created_at: Some(Timestamp {
                seconds: metadata.created_at.timestamp(),
                nanos: metadata.created_at.timestamp_subsec_nanos() as i32,
            }),
            modified_at: Some(Timestamp {
                seconds: metadata.modified_at.timestamp(),
                nanos: metadata.modified_at.timestamp_subsec_nanos() as i32,
            }),
            metadata: metadata.metadata,
            version: metadata.version,
            status: 0, // Will be mapped from string
        }
    }
} 