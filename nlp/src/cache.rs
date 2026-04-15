use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_dynamodb::types::{AttributeValue, ScalarAttributeType, BillingMode};
use base64::Engine;
use prost::Message;
use serde::{Deserialize, Serialize};
use crate::proto::nlp::v1::ExtractInvariantsResponse;
use crate::InvariantExtractionConfig;

pub struct DynamoCache {
    client: DynamoClient,
    table_name: String,
    ttl_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    cache_key: String,
    /// Base64-encoded `ExtractInvariantsResponse` protobuf.
    response_b64: String,
    created_at: u64,
    expires_at: u64,
}

impl DynamoCache {
    pub fn new(client: DynamoClient, config: &InvariantExtractionConfig) -> Self {
        Self {
            client,
            table_name: "spec-to-proof-nlp-cache".to_string(),
            ttl_seconds: config.cache_ttl_seconds,
        }
    }

    pub async fn get(&self, cache_key: &str) -> Result<Option<ExtractInvariantsResponse>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let response = self.client
            .get_item()
            .table_name(&self.table_name)
            .key("cache_key", AttributeValue::S(cache_key.to_string()))
            .send()
            .await?;

        if let Some(item) = response.item {
            if let (Some(cache_key_attr), Some(response_attr), Some(expires_at_attr)) = (
                item.get("cache_key"),
                item.get("response"),
                item.get("expires_at"),
            ) {
                if let (Some(cache_key_val), Some(expires_at_val)) = (
                    cache_key_attr.as_s().ok(),
                    expires_at_attr.as_n().ok(),
                ) {
                    if cache_key_val == cache_key {
                        if let Ok(expires_at) = expires_at_val.parse::<u64>() {
                            if expires_at > now {
                                // Cache hit and not expired
                                if let Ok(response_json) = response_attr.as_s() {
                                    if let Ok(cache_entry) = serde_json::from_str::<CacheEntry>(response_json) {
                                        if let Ok(bytes) =
                                            base64::engine::general_purpose::STANDARD.decode(&cache_entry.response_b64)
                                        {
                                            if let Ok(parsed) = ExtractInvariantsResponse::decode(&bytes[..]) {
                                                tracing::info!("Cache hit for key: {}", cache_key);
                                                return Ok(Some(parsed));
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Expired, delete it
                                self.delete(cache_key).await?;
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    pub async fn set(&self, cache_key: &str, response: &ExtractInvariantsResponse) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let expires_at = now + self.ttl_seconds;

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        let response_b64 = base64::engine::general_purpose::STANDARD.encode(&buf);

        let cache_entry = CacheEntry {
            cache_key: cache_key.to_string(),
            response_b64,
            created_at: now,
            expires_at,
        };

        let response_json = serde_json::to_string(&cache_entry)?;

        self.client
            .put_item()
            .table_name(&self.table_name)
            .item("cache_key", AttributeValue::S(cache_key.to_string()))
            .item("response", AttributeValue::S(response_json))
            .item("created_at", AttributeValue::N(now.to_string()))
            .item("expires_at", AttributeValue::N(expires_at.to_string()))
            .send()
            .await?;

        tracing::info!("Cached response for key: {}", cache_key);
        Ok(())
    }

    pub async fn delete(&self, cache_key: &str) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.table_name)
            .key("cache_key", AttributeValue::S(cache_key.to_string()))
            .send()
            .await?;

        tracing::info!("Deleted cache entry for key: {}", cache_key);
        Ok(())
    }

    pub async fn ensure_table_exists(&self) -> Result<()> {
        // Check if table exists
        match self.client
            .describe_table()
            .table_name(&self.table_name)
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!("Cache table {} already exists", self.table_name);
                return Ok(());
            }
            Err(_) => {
                // Table doesn't exist, create it
                tracing::info!("Creating cache table: {}", self.table_name);
            }
        }

        self.client
            .create_table()
            .table_name(&self.table_name)
            .attribute_definitions(
                aws_sdk_dynamodb::types::AttributeDefinition::builder()
                    .attribute_name("cache_key")
                    .attribute_type(ScalarAttributeType::S)
                    .build()?,
            )
            .key_schema(
                aws_sdk_dynamodb::types::KeySchemaElement::builder()
                    .attribute_name("cache_key")
                    .key_type(aws_sdk_dynamodb::types::KeyType::Hash)
                    .build()?,
            )
            .billing_mode(BillingMode::PayPerRequest)
            .send()
            .await?;

        // Wait for table to be active
        self.wait_for_table_active().await?;
        tracing::info!("Cache table {} created successfully", self.table_name);

        Ok(())
    }

    async fn wait_for_table_active(&self) -> Result<()> {
        let max_attempts = 30;
        let delay = Duration::from_secs(2);

        for attempt in 1..=max_attempts {
            match self.client
                .describe_table()
                .table_name(&self.table_name)
                .send()
                .await
            {
                Ok(response) => {
                    if let Some(table) = response.table {
                        if let Some(status) = table.table_status {
                            if status == aws_sdk_dynamodb::types::TableStatus::Active {
                                return Ok(());
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to describe table (attempt {}): {}", attempt, e);
                }
            }

            if attempt < max_attempts {
                tokio::time::sleep(delay).await;
            }
        }

        Err(anyhow!("Table did not become active within expected time"))
    }

    pub async fn cleanup_expired(&self) -> Result<u32> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let scan_response = self.client
            .scan()
            .table_name(&self.table_name)
            .filter_expression("expires_at < :now")
            .expression_attribute_values(":now", AttributeValue::N(now.to_string()))
            .send()
            .await?;

        let mut deleted_count = 0;

        if let Some(items) = scan_response.items {
            for item in items {
                if let Some(cache_key_attr) = item.get("cache_key") {
                    if let Ok(cache_key) = cache_key_attr.as_s() {
                        self.delete(cache_key).await?;
                        deleted_count += 1;
                    }
                }
            }
        }

        tracing::info!("Cleaned up {} expired cache entries", deleted_count);
        Ok(deleted_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::nlp::v1::{ExtractInvariantsResponse, TokenUsage, ProcessingMetadata};
    use prost::Message;

    #[tokio::test]
    async fn test_cache_entry_serialization() {
        let response = ExtractInvariantsResponse {
            invariants: vec![],
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                estimated_cost_usd: 0.00225,
            }),
            metadata: Some(ProcessingMetadata {
                processed_at: None,
                model_used: "claude-3-opus-20240229".to_string(),
                duration_ms: 1000,
                cached: false,
                cache_key: "test_key".to_string(),
            }),
        };

        let mut buf = Vec::new();
        response.encode(&mut buf).unwrap();
        let response_b64 = base64::engine::general_purpose::STANDARD.encode(&buf);

        let cache_entry = CacheEntry {
            cache_key: "test_key".to_string(),
            response_b64,
            created_at: 1234567890,
            expires_at: 1234567890 + 86400,
        };

        let serialized = serde_json::to_string(&cache_entry).unwrap();
        let deserialized: CacheEntry = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.cache_key, "test_key");
        assert_eq!(deserialized.created_at, 1234567890);
        assert_eq!(deserialized.expires_at, 1234567890 + 86400);
    }
} 