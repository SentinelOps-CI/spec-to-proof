pub mod claude_client;
pub mod compiler;
pub mod s3_storage;
pub mod prompts;
pub mod proto;

use std::collections::HashMap;
use std::error::Error;
use std::pin::Pin;
use std::time::{Duration, Instant};
use futures_util::Stream;
use tonic::{Request, Response, Status};

use crate::proto::proof::v1::proof_service_server::ProofService as ProofServiceTrait;
use crate::proto::proof::v1::*;
use crate::proto::spec_to_proof::v1::*;

#[derive(Clone)]
pub struct ProofConfig {
    pub claude_api_key: String,
    pub claude_model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub cost_per_1k_tokens: f64,
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_key_prefix: String,
    pub kms_key_id: Option<String>,
}

impl Default for ProofConfig {
    fn default() -> Self {
        Self {
            claude_api_key: String::new(),
            claude_model: "claude-3-opus-20240229".to_string(),
            max_tokens: 8000,
            temperature: 0.0, // Deterministic generation
            max_retries: 3,
            retry_delay_ms: 1000,
            cost_per_1k_tokens: 0.015, // Claude 3 Opus pricing
            s3_bucket: "spec-to-proof-lean".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_key_prefix: "theorems/".to_string(),
            kms_key_id: None,
        }
    }
}

pub struct ProofServiceImpl {
    config: ProofConfig,
    #[allow(dead_code)] // Wired for future direct Claude calls alongside the compiler.
    claude_client: claude_client::ClaudeClient,
    compiler: compiler::LeanCompiler,
    s3_storage: s3_storage::S3Storage,
    start_time: Instant,
}

impl ProofServiceImpl {
    pub async fn new(config: ProofConfig) -> Result<Self, Box<dyn Error>> {
        let claude_client = claude_client::ClaudeClient::new(&config.claude_api_key, &config.claude_model);
        let compiler = compiler::LeanCompiler::new(&config);
        let s3_storage = s3_storage::S3Storage::new(&config).await?;

        Ok(Self {
            config,
            claude_client,
            compiler,
            s3_storage,
            start_time: Instant::now(),
        })
    }

    pub async fn compile_invariant_set(
        &self,
        invariant_set: &InvariantSet,
        options: &CompilationOptions,
    ) -> Result<Vec<LeanTheorem>, Box<dyn Error>> {
        let start_time = Instant::now();
        
        tracing::info!("Compiling invariant set {} with {} invariants", 
            invariant_set.id, invariant_set.invariants.len());

        let mut theorems = Vec::new();
        let mut total_input_tokens = 0;
        let mut total_output_tokens = 0;

        for invariant in &invariant_set.invariants {
            let theorem = self.compiler.compile_invariant_to_theorem(invariant, options).await?;
            
            // Track token usage
            if let Some(token_usage) = &theorem.metadata.get("token_usage") {
                if let Ok(usage) = serde_json::from_str::<HashMap<String, u32>>(token_usage) {
                    total_input_tokens += usage.get("input_tokens").unwrap_or(&0);
                    total_output_tokens += usage.get("output_tokens").unwrap_or(&0);
                }
            }
            
            theorems.push(theorem);
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let estimated_cost = self.estimate_cost(total_input_tokens, total_output_tokens);

        tracing::info!("Compiled {} theorems in {}ms, cost: ${:.4}", 
            theorems.len(), duration_ms, estimated_cost);

        Ok(theorems)
    }

    pub async fn generate_proof(
        &self,
        theorem: &LeanTheorem,
        options: &ProofOptions,
    ) -> Result<(LeanTheorem, ProofArtifact), Box<dyn Error>> {
        let start_time = Instant::now();
        
        tracing::info!("Generating proof for theorem {}", theorem.theorem_name);

        let mut attempts = 0;
        let mut last_error: Option<String> = None;

        while attempts < options.max_attempts {
            attempts += 1;
            
            match self
                .compiler
                .generate_proof(theorem, options)
                .await
                .map_err(|e| e.to_string())
            {
                Ok((mut proven_theorem, mut proof_artifact)) => {
                    let duration_ms = start_time.elapsed().as_millis() as u64;
                    
                    tracing::info!("Proof generated successfully in {}ms after {} attempts", 
                        duration_ms, attempts);

                    // Record the real number of attempts taken by the retry loop
                    // (the compiler only sees a single invocation). Both the
                    // theorem metadata and the artifact duration are updated so
                    // downstream consumers observe the end-to-end cost.
                    proven_theorem
                        .metadata
                        .insert("attempts".to_string(), attempts.to_string());
                    proof_artifact.duration_ms = duration_ms as i64;

                    return Ok((proven_theorem, proof_artifact));
                }
                Err(err_msg) => {
                    last_error = Some(err_msg.clone());

                    if attempts < options.max_attempts {
                        let delay_ms = self.config.retry_delay_ms.saturating_mul(2_u64.pow(
                            attempts.saturating_sub(1),
                        ));
                        let delay = Duration::from_millis(delay_ms);
                        tracing::warn!("Proof attempt {} failed, retrying in {:?}: {}", 
                            attempts, delay, err_msg);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All proof attempts failed".to_string()).into())
    }

    pub async fn stream_lean_code(
        &self,
        theorem: &LeanTheorem,
        s3_config: &S3Config,
        versioning: &VersioningOptions,
    ) -> Result<StreamLeanCodeResponse, Box<dyn Error>> {
        let start_time = Instant::now();
        
        tracing::info!("Streaming Lean code for theorem {} to S3", theorem.theorem_name);

        // Generate version based on strategy
        let version = match versioning.strategy() {
            VersioningStrategy::Hash => theorem.content_sha256.clone(),
            VersioningStrategy::Timestamp => {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string()
            }
            VersioningStrategy::Sequential => {
                // In a real implementation, this would use a counter service
                "1".to_string()
            }
            VersioningStrategy::Custom => versioning.custom_version.clone(),
            _ => theorem.content_sha256.clone(),
        };

        // Upload to S3
        let s3_location = self.s3_storage.upload_theorem(theorem, &version, s3_config).await?;
        
        let duration_ms = start_time.elapsed().as_millis() as u64;
        let file_size = theorem.lean_code.len() as u64;

        let metadata = UploadMetadata {
            file_size,
            content_type: "text/plain".to_string(),
            checksum: theorem.content_sha256.clone(),
            uploaded_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        };

        tracing::info!("Successfully uploaded theorem to {} in {}ms", s3_location, duration_ms);

        Ok(StreamLeanCodeResponse {
            status: UploadStatus::Completed as i32,
            s3_location,
            version,
            metadata: Some(metadata),
        })
    }

    fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        let total_tokens = input_tokens.saturating_add(output_tokens);
        (total_tokens as f64 / 1000.0) * self.config.cost_per_1k_tokens
    }

    /// Aggregate the per-theorem token counters that the compiler records in
    /// `LeanTheorem::metadata` into `(input_tokens, output_tokens)`.
    ///
    /// `key_prefix` is `""` for the compilation path (keys `input_tokens` /
    /// `output_tokens`) and `"proof_"` for the proof-generation path (keys
    /// `proof_input_tokens` / `proof_output_tokens`).
    fn aggregate_tokens<'a, I>(theorems: I, key_prefix: &str) -> (u32, u32)
    where
        I: IntoIterator<Item = &'a LeanTheorem>,
    {
        let input_key = format!("{}input_tokens", key_prefix);
        let output_key = format!("{}output_tokens", key_prefix);
        let mut input = 0u32;
        let mut output = 0u32;
        for theorem in theorems {
            if let Some(value) = theorem.metadata.get(&input_key) {
                input = input.saturating_add(value.parse().unwrap_or(0));
            }
            if let Some(value) = theorem.metadata.get(&output_key) {
                output = output.saturating_add(value.parse().unwrap_or(0));
            }
        }
        (input, output)
    }

    fn get_uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

#[tonic::async_trait]
impl ProofServiceTrait for ProofServiceImpl {
    type StreamLeanCodeStream =
        Pin<Box<dyn Stream<Item = Result<StreamLeanCodeResponse, Status>> + Send + 'static>>;

    async fn compile_invariant_set(
        &self,
        request: Request<CompileInvariantSetRequest>,
    ) -> Result<Response<CompileInvariantSetResponse>, Status> {
        let req = request.into_inner();
        let start_time = Instant::now();

        match self.compile_invariant_set(&req.invariant_set.unwrap(), &req.options.unwrap()).await {
            Ok(theorems) => {
                let duration_ms = start_time.elapsed().as_millis() as u64;

                let (input_tokens, output_tokens) = Self::aggregate_tokens(&theorems, "");
                let estimated_cost = self.estimate_cost(input_tokens, output_tokens);

                let metadata = CompilationMetadata {
                    duration_ms,
                    token_usage: Some(TokenUsage {
                        input_tokens,
                        output_tokens,
                        total_tokens: input_tokens.saturating_add(output_tokens),
                    }),
                    estimated_cost,
                    compiled_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                };

                let response = CompileInvariantSetResponse {
                    theorems,
                    metadata: Some(metadata),
                    errors: Vec::new(),
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                tracing::error!("Failed to compile invariant set: {}", e);
                Err(Status::internal(format!("Compilation failed: {}", e)))
            }
        }
    }

    async fn generate_proof(
        &self,
        request: Request<GenerateProofRequest>,
    ) -> Result<Response<GenerateProofResponse>, Status> {
        let req = request.into_inner();
        let start_time = Instant::now();

        match self.generate_proof(&req.theorem.unwrap(), &req.options.unwrap()).await {
            Ok((theorem, proof_artifact)) => {
                let duration_ms = start_time.elapsed().as_millis() as u64;

                let (input_tokens, output_tokens) = Self::aggregate_tokens(
                    std::iter::once(&theorem),
                    "proof_",
                );
                let estimated_cost = self.estimate_cost(input_tokens, output_tokens);
                let attempts = theorem
                    .metadata
                    .get("attempts")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);

                let metadata = ProofMetadata {
                    duration_ms,
                    token_usage: Some(TokenUsage {
                        input_tokens,
                        output_tokens,
                        total_tokens: input_tokens.saturating_add(output_tokens),
                    }),
                    estimated_cost,
                    attempts,
                    generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                };

                let response = GenerateProofResponse {
                    theorem: Some(theorem),
                    proof_artifact: Some(proof_artifact),
                    metadata: Some(metadata),
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                tracing::error!("Failed to generate proof: {}", e);
                Err(Status::internal(format!("Proof generation failed: {}", e)))
            }
        }
    }

    async fn stream_lean_code(
        &self,
        request: Request<StreamLeanCodeRequest>,
    ) -> Result<Response<Self::StreamLeanCodeStream>, Status> {
        let req = request.into_inner();

        match self.stream_lean_code(&req.theorem.unwrap(), &req.s3_config.unwrap(), &req.versioning.unwrap()).await {
            Ok(response) => {
                let stream: Self::StreamLeanCodeStream = Box::pin(tokio_stream::once(Ok(response)));
                Ok(Response::new(stream))
            }
            Err(e) => {
                tracing::error!("Failed to stream Lean code: {}", e);
                Err(Status::internal(format!("Streaming failed: {}", e)))
            }
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let response = HealthCheckResponse {
            status: ServiceStatus::Healthy as i32,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: self.get_uptime_seconds(),
            checked_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        };

        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theorem_with_tokens(pairs: &[(&str, &str)]) -> LeanTheorem {
        let mut metadata = HashMap::new();
        for (key, value) in pairs {
            metadata.insert(key.to_string(), value.to_string());
        }
        LeanTheorem {
            id: "t".to_string(),
            content_sha256: String::new(),
            theorem_name: "t".to_string(),
            lean_code: String::new(),
            source_invariant_id: String::new(),
            generated_at: None,
            status: 0,
            compilation_errors: Vec::new(),
            proof_strategy: String::new(),
            metadata,
        }
    }

    #[test]
    fn aggregate_tokens_sums_compilation_counters() {
        let theorems = vec![
            theorem_with_tokens(&[("input_tokens", "100"), ("output_tokens", "40")]),
            theorem_with_tokens(&[("input_tokens", "25"), ("output_tokens", "5")]),
        ];

        let (input, output) = ProofServiceImpl::aggregate_tokens(&theorems, "");
        assert_eq!(input, 125);
        assert_eq!(output, 45);
    }

    #[test]
    fn aggregate_tokens_respects_prefix_and_missing_keys() {
        // Only the prefixed keys should be counted; malformed/missing entries
        // are treated as zero rather than panicking.
        let theorems = vec![
            theorem_with_tokens(&[("proof_input_tokens", "10"), ("input_tokens", "999")]),
            theorem_with_tokens(&[("proof_output_tokens", "not-a-number")]),
        ];

        let (input, output) = ProofServiceImpl::aggregate_tokens(&theorems, "proof_");
        assert_eq!(input, 10);
        assert_eq!(output, 0);
    }
}