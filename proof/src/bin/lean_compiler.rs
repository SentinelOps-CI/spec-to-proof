use std::error::Error;
use tonic::transport::Server;
use tracing::info;

use proof::{ProofConfig, ProofServiceImpl};
use proof::proto::proof::v1::proof_service_server::ProofServiceServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Spec-to-Proof Lean Compiler Service");

    // Load configuration
    let config = load_config()?;
    
    // Create the proof service
    let proof_service = ProofServiceImpl::new(config).await?;
    
    // Create gRPC server
    let addr = "[::1]:50051".parse()?;
    let svc = ProofServiceServer::new(proof_service);
    
    info!("Proof service listening on {}", addr);
    
    // Run the server
    Server::builder()
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}

fn load_config() -> Result<ProofConfig, Box<dyn Error>> {
    let config = ProofConfig {
        claude_api_key: std::env::var("CLAUDE_API_KEY")
            .unwrap_or_else(|_| "".to_string()),
        claude_model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-3-opus-20240229".to_string()),
        max_tokens: std::env::var("MAX_TOKENS")
            .unwrap_or_else(|_| "8000".to_string())
            .parse()
            .unwrap_or(8000),
        temperature: std::env::var("TEMPERATURE")
            .unwrap_or_else(|_| "0.0".to_string())
            .parse()
            .unwrap_or(0.0),
        max_retries: std::env::var("MAX_RETRIES")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .unwrap_or(3),
        retry_delay_ms: std::env::var("RETRY_DELAY_MS")
            .unwrap_or_else(|_| "1000".to_string())
            .parse()
            .unwrap_or(1000),
        cost_per_1k_tokens: std::env::var("COST_PER_1K_TOKENS")
            .unwrap_or_else(|_| "0.015".to_string())
            .parse()
            .unwrap_or(0.015),
        s3_bucket: std::env::var("S3_BUCKET")
            .unwrap_or_else(|_| "spec-to-proof-lean".to_string()),
        s3_region: std::env::var("S3_REGION")
            .unwrap_or_else(|_| "us-east-1".to_string()),
        s3_key_prefix: std::env::var("S3_KEY_PREFIX")
            .unwrap_or_else(|_| "theorems/".to_string()),
        kms_key_id: std::env::var("KMS_KEY_ID").ok(),
    };

    // Validate required configuration
    if config.claude_api_key.is_empty() {
        return Err("CLAUDE_API_KEY environment variable is required".into());
    }

    info!("Configuration loaded successfully");
    info!("Claude Model: {}", config.claude_model);
    info!("S3 Bucket: {}", config.s3_bucket);
    info!("S3 Region: {}", config.s3_region);
    info!("Temperature: {}", config.temperature);
    info!("Max Tokens: {}", config.max_tokens);

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        // Set required environment variable
        std::env::set_var("CLAUDE_API_KEY", "test-key");
        
        let config = load_config().unwrap();
        assert_eq!(config.claude_api_key, "test-key");
        assert_eq!(config.claude_model, "claude-3-opus-20240229");
        assert_eq!(config.temperature, 0.0);
        assert_eq!(config.max_tokens, 8000);
    }

    #[test]
    fn test_config_loading_missing_api_key() {
        // Remove the API key
        std::env::remove_var("CLAUDE_API_KEY");
        
        let result = load_config();
        match result {
            Err(e) => assert!(e.to_string().contains("CLAUDE_API_KEY")),
            Ok(_) => panic!("expected missing CLAUDE_API_KEY to fail"),
        }
    }
} 