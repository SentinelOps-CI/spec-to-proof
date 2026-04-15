use anyhow::{Context, Result};
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, error};
use aws_config::BehaviorVersion;

use nlp::{
    NlpService, InvariantExtractionConfig,
    proto::nlp::v1::{
        nlp_service_server::{NlpService as NlpServiceTrait, NlpServiceServer},
        ExtractInvariantsRequest, ExtractInvariantsResponse,
        HealthCheckRequest, HealthCheckResponse,
    }
};

#[derive(Default)]
pub struct NlpServiceImpl {
    service: Option<NlpService>,
}

#[tonic::async_trait]
impl NlpServiceTrait for NlpServiceImpl {
    async fn extract_invariants(
        &self,
        request: Request<ExtractInvariantsRequest>,
    ) -> Result<Response<ExtractInvariantsResponse>, Status> {
        let request_inner = request.into_inner();
        
        info!("Processing invariant extraction request for document: {}", request_inner.document_id);
        
        if let Some(service) = &self.service {
            match service.extract_invariants(request_inner).await {
                Ok(response) => {
                    info!("Successfully extracted {} invariants", response.invariants.len());
                    Ok(Response::new(response))
                }
                Err(e) => {
                    error!("Failed to extract invariants: {}", e);
                    Err(Status::internal(format!("Extraction failed: {}", e)))
                }
            }
        } else {
            Err(Status::unavailable("Service not initialized"))
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        if let Some(service) = &self.service {
            match service.health_check(_request.into_inner()).await {
                Ok(response) => Ok(Response::new(response)),
                Err(e) => {
                    error!("Health check failed: {}", e);
                    Err(Status::internal("Health check failed"))
                }
            }
        } else {
            Err(Status::unavailable("Service not initialized"))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Spec-to-Proof NLP Service");

    // Load configuration
    let config = load_config()?;
    
    // Initialize AWS SDK
    let aws_config = aws_config::defaults(BehaviorVersion::latest())
        .load()
        .await;

    // Initialize DynamoDB client
    let dynamo_client = aws_sdk_dynamodb::Client::new(&aws_config);

    // Initialize NLP service
    let nlp_service = NlpService::new(config, dynamo_client).await?;

    // Ensure cache table exists
    nlp_service.ensure_cache_table_exists().await?;

    // Create service implementation
    let service_impl = NlpServiceImpl {
        service: Some(nlp_service),
    };

    // Start gRPC server
    let addr = "[::1]:50051".parse()?;
    info!("NLP Service listening on {}", addr);

    Server::builder()
        .add_service(NlpServiceServer::new(service_impl))
        .serve(addr)
        .await?;

    Ok(())
}

fn load_config() -> Result<InvariantExtractionConfig> {
    let claude_api_key = std::env::var("CLAUDE_API_KEY")
        .context("CLAUDE_API_KEY environment variable is required")?;

    let config = InvariantExtractionConfig {
        claude_api_key,
        claude_model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-3-opus-20240229".to_string()),
        max_tokens: std::env::var("MAX_TOKENS")
            .unwrap_or_else(|_| "4000".to_string())
            .parse()
            .unwrap_or(4000),
        temperature: std::env::var("TEMPERATURE")
            .unwrap_or_else(|_| "0.0".to_string())
            .parse()
            .unwrap_or(0.0),
        cache_ttl_seconds: std::env::var("CACHE_TTL_SECONDS")
            .unwrap_or_else(|_| "86400".to_string())
            .parse()
            .unwrap_or(86400),
        max_retries: std::env::var("MAX_RETRIES")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .unwrap_or(3),
        retry_delay_ms: std::env::var("RETRY_DELAY_MS")
            .unwrap_or_else(|_| "1000".to_string())
            .parse()
            .unwrap_or(1000),
        confidence_threshold: std::env::var("CONFIDENCE_THRESHOLD")
            .unwrap_or_else(|_| "0.5".to_string())
            .parse()
            .unwrap_or(0.5),
        cost_per_1k_tokens: std::env::var("COST_PER_1K_TOKENS")
            .unwrap_or_else(|_| "0.015".to_string())
            .parse()
            .unwrap_or(0.015),
    };

    info!("Loaded configuration: {:?}", config);
    Ok(config)
} 