pub mod claude_client;
pub mod extractor;
pub mod post_processor;
pub mod cache;
pub mod pii_redactor;
pub mod prompts;
pub mod proto;

use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use aws_sdk_dynamodb::Client as DynamoClient;

use crate::proto::nlp::v1::{
    ExtractInvariantsRequest, ExtractInvariantsResponse, ExtractedInvariant,
    ProcessingMetadata, ExtractionMetadata,
    HealthCheckRequest, HealthCheckResponse,
};

use crate::claude_client::ClaudeClient;
use crate::extractor::InvariantExtractor;
use crate::cache::DynamoCache;
use crate::pii_redactor::PiiRedactor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantExtractionConfig {
    pub claude_api_key: String,
    pub claude_model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub cache_ttl_seconds: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub confidence_threshold: f64,
    pub cost_per_1k_tokens: f64,
}

impl Default for InvariantExtractionConfig {
    fn default() -> Self {
        Self {
            claude_api_key: String::new(),
            claude_model: "claude-3-opus-20240229".to_string(),
            max_tokens: 4000,
            temperature: 0.0,
            cache_ttl_seconds: 86400, // 24 hours
            max_retries: 3,
            retry_delay_ms: 1000,
            confidence_threshold: 0.5,
            cost_per_1k_tokens: 0.015, // Claude 3 Opus pricing
        }
    }
}

pub struct NlpService {
    config: InvariantExtractionConfig,
    /// Reserved for future direct Claude calls from this service layer.
    #[allow(dead_code)]
    claude_client: ClaudeClient,
    extractor: InvariantExtractor,
    cache: DynamoCache,
    pii_redactor: PiiRedactor,
    post_processor: post_processor::PostProcessor,
}

impl NlpService {
    pub async fn new(
        config: InvariantExtractionConfig,
        dynamo_client: DynamoClient,
    ) -> Result<Self> {
        let claude_client = ClaudeClient::new(&config.claude_api_key, &config.claude_model);
        let extractor = InvariantExtractor::new(&config);
        let cache = DynamoCache::new(dynamo_client, &config);
        let pii_redactor = PiiRedactor::new();
        let post_processor = post_processor::PostProcessor::new();

        Ok(Self {
            config,
            claude_client,
            extractor,
            cache,
            pii_redactor,
            post_processor,
        })
    }

    pub async fn extract_invariants(
        &self,
        request: ExtractInvariantsRequest,
    ) -> Result<ExtractInvariantsResponse> {
        let start_time = Instant::now();
        
        // Generate cache key from document content
        let cache_key = self.generate_cache_key(&request);
        
        // Check cache first
        if let Some(cached_response) = self.cache.get(&cache_key).await? {
            tracing::info!("Serving invariant extraction from cache for document {}", request.document_id);
            return Ok(self.add_metadata(cached_response, start_time, true, &cache_key));
        }

        // Redact PII from content
        let (redacted_content, pii_detected, redacted_fields) = 
            self.pii_redactor.redact(&request.content);

        // Extract invariants using Claude
        let extraction_result = self.extractor
            .extract_invariants(&request, &redacted_content)
            .await?;

        // Post-process invariants
        let processed_invariants = self.post_processor
            .process_invariants(extraction_result.invariants)
            .await?;

        // Filter by confidence threshold
        let filtered_invariants: Vec<ExtractedInvariant> = processed_invariants
            .into_iter()
            .filter(|inv| inv.confidence_score >= self.config.confidence_threshold)
            .collect();

        // Create response
        let response = ExtractInvariantsResponse {
            invariants: filtered_invariants,
            token_usage: extraction_result.token_usage,
            metadata: None,
        };

        // Cache the result
        self.cache.set(&cache_key, &response).await?;

        let final_response = self.add_metadata(
            response, 
            start_time, 
            false, 
            &cache_key
        );

        // Add extraction metadata
        let mut response_with_metadata = final_response;
        for invariant in &mut response_with_metadata.invariants {
            invariant.extraction_metadata = Some(ExtractionMetadata {
                prompt_version: "1.0.0".to_string(),
                post_processing_rules: vec![
                    "variable_normalization".to_string(),
                    "unit_standardization".to_string(),
                    "confidence_filtering".to_string(),
                ],
                retry_count: 0,
                pii_detected,
                redacted_fields: redacted_fields.clone(),
            });
        }

        Ok(response_with_metadata)
    }

    pub async fn ensure_cache_table_exists(&self) -> Result<()> {
        self.cache.ensure_table_exists().await
    }

    pub async fn health_check(
        &self,
        _request: HealthCheckRequest,
    ) -> Result<HealthCheckResponse> {
        Ok(HealthCheckResponse {
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        })
    }

    fn generate_cache_key(&self, request: &ExtractInvariantsRequest) -> String {
        use sha2::{Sha256, Digest};
        
        let content = format!(
            "{}:{}:{}:{}",
            request.document_id,
            request.content,
            request.title,
            request.source_system
        );
        
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("invariant_extraction:{}", hex::encode(hasher.finalize()))
    }

    fn add_metadata(
        &self,
        mut response: ExtractInvariantsResponse,
        start_time: Instant,
        cached: bool,
        cache_key: &str,
    ) -> ExtractInvariantsResponse {
        response.metadata = Some(ProcessingMetadata {
            processed_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            model_used: self.config.claude_model.clone(),
            duration_ms: start_time.elapsed().as_millis() as i64,
            cached,
            cache_key: cache_key.to_string(),
        });
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_nlp_service_creation() {
        let _config = InvariantExtractionConfig::default();
        // This would need a mock DynamoDB client in a real test
        // let service = NlpService::new(config, mock_dynamo_client).await;
        // assert!(service.is_ok());
    }
} 