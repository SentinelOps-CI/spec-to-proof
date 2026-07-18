use std::sync::Arc;

use ingest::{
    ConnectorConfig, IngestionConnector, OAuth2Token,
    connectors::JiraConnector,
    publisher::DocumentPublisher,
    secrets::SecretsManager,
};
use aws_sdk_secretsmanager::Client as SecretsClient;
use aws_sdk_kms::Client as KmsClient;
use tokio::signal;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Jira connector...");

    // Load configuration from environment variables
    let config = load_config()?;

    // Initialize AWS clients
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let secrets_client = SecretsClient::new(&aws_config);
    let kms_client = KmsClient::new(&aws_config);

    // Initialize secrets manager
    let secrets_manager = SecretsManager::new(
        secrets_client.clone(),
        kms_client,
        std::env::var("KMS_KEY_ID").unwrap_or_else(|_| "alias/spec-to-proof".to_string()),
    );

    // Initialize the document publisher. With the `nats-jetstream` feature the
    // connector publishes to a real NATS JetStream cluster; without it we fall
    // back to an in-memory publisher so the binary still runs (documents are
    // not delivered anywhere durable, which is logged prominently).
    let publisher = build_publisher().await?;

    // Initialize Jira connector
    let mut jira_connector = JiraConnector::new(config.clone());

    // Initialize ingestion connector
    let ingestion_connector = IngestionConnector::new(
        config.clone(),
        secrets_client,
        publisher,
    ).await?;

    info!("Jira connector initialized successfully");

    // Main polling loop
    loop {
        match poll_and_publish(&mut jira_connector, &ingestion_connector, &secrets_manager).await {
            Ok(_) => {
                info!("Successfully polled and published Jira documents");
            }
            Err(e) => {
                error!("Error polling Jira documents: {}", e);
            }
        }

        // Wait for next poll interval or shutdown signal
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(config.poll_interval_seconds)) => {
                continue;
            }
            _ = signal::ctrl_c() => {
                info!("Received shutdown signal, stopping Jira connector");
                break;
            }
        }
    }

    info!("Jira connector stopped");
    Ok(())
}

/// Build the document publisher based on the compiled feature set.
async fn build_publisher() -> Result<Arc<dyn DocumentPublisher>, Box<dyn std::error::Error>> {
    #[cfg(feature = "nats-jetstream")]
    {
        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
        info!("Connecting to NATS JetStream at {}", nats_url);
        let publisher = ingest::publisher::NatsJetStreamPublisher::connect(&nats_url)
            .await
            .map_err(|e| format!("failed to connect to NATS JetStream at {}: {}", nats_url, e))?;
        Ok(Arc::new(publisher))
    }
    #[cfg(not(feature = "nats-jetstream"))]
    {
        tracing::warn!(
            "built without the `nats-jetstream` feature: using an in-memory publisher; \
             ingested documents will NOT be delivered to NATS. Rebuild with \
             `--features nats-jetstream` for production use."
        );
        Ok(Arc::new(ingest::publisher::InMemoryPublisher::new()))
    }
}

fn load_config() -> Result<ConnectorConfig, Box<dyn std::error::Error>> {
    let source_system = std::env::var("SOURCE_SYSTEM").unwrap_or_else(|_| "jira".to_string());
    let base_url = std::env::var("JIRA_BASE_URL")
        .map_err(|_| "JIRA_BASE_URL environment variable is required")?;
    let rate_limit_per_minute = std::env::var("RATE_LIMIT_PER_MINUTE")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<u32>()?;
    let batch_size = std::env::var("BATCH_SIZE")
        .unwrap_or_else(|_| "50".to_string())
        .parse::<usize>()?;
    let poll_interval_seconds = std::env::var("POLL_INTERVAL_SECONDS")
        .unwrap_or_else(|_| "300".to_string())
        .parse::<u64>()?;
    let secrets_arn = std::env::var("SECRETS_ARN")
        .map_err(|_| "SECRETS_ARN environment variable is required")?;

    Ok(ConnectorConfig {
        source_system,
        base_url,
        rate_limit_per_minute,
        batch_size,
        poll_interval_seconds,
        secrets_arn,
    })
}

async fn poll_and_publish(
    jira_connector: &mut JiraConnector,
    ingestion_connector: &IngestionConnector,
    secrets_manager: &SecretsManager,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get OAuth2 token from secrets manager
    let token_key = "jira-oauth-token";
    let oauth_credentials = secrets_manager.retrieve_oauth2_credentials(token_key).await?;

    // For now, we'll use a mock token. In production, you'd implement OAuth2 token refresh
    let token = OAuth2Token {
        access_token: "mock_access_token".to_string(),
        refresh_token: oauth_credentials.client_secret,
        expires_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0),
        token_type: "Bearer".to_string(),
    };

    // Poll documents from Jira
    let documents = jira_connector.poll_documents(&token).await?;

    if documents.is_empty() {
        info!("No new documents found in Jira");
        return Ok(());
    }

    // Publish documents to the configured transport.
    let document_count = documents.len();
    for document in documents {
        let document_id = document.id.clone();
        match ingestion_connector.publish_document(document).await {
            Ok(_) => {
                info!("Published document {}", document_id);
            }
            Err(e) => {
                error!("Failed to publish document {}: {}", document_id, e);
            }
        }
    }

    info!("Successfully processed {} documents from Jira", document_count);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config() {
        std::env::set_var("JIRA_BASE_URL", "https://example.atlassian.net");
        std::env::set_var("SECRETS_ARN", "arn:aws:secretsmanager:us-east-1:123456789012:secret:jira-oauth");

        let config = load_config().unwrap();
        assert_eq!(config.source_system, "jira");
        assert_eq!(config.base_url, "https://example.atlassian.net");
        assert_eq!(config.rate_limit_per_minute, 100);
        assert_eq!(config.batch_size, 50);
        assert_eq!(config.poll_interval_seconds, 300);
        assert_eq!(config.secrets_arn, "arn:aws:secretsmanager:us-east-1:123456789012:secret:jira-oauth");
    }

    #[test]
    fn test_load_config_missing_required() {
        std::env::remove_var("JIRA_BASE_URL");
        std::env::remove_var("SECRETS_ARN");

        let result = load_config();
        assert!(result.is_err());
    }
} 