use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{error, info, instrument, warn};

use crate::artifact::{write_reference_to_dir, ProofArtifactReference};
use crate::config::Config;
use crate::metrics::Metrics;
use crate::proto::spec_to_proof::v1::{LeanTheorem, ProofArtifact, ProofStatus, ResourceUsage};
use crate::sandbox::{Sandbox, SandboxOutcome};
use crate::{
    lean::LeanCompiler, security::SecurityManager, storage::StorageManager, BoxError, JobQueue,
    ProofJob, ProofOptions, ProofResult,
};
use prost::Message;

/// Metadata key used to associate a job with the originating spec document, so
/// the emitted artifact reference can be keyed for the badge service.
const SPEC_DOCUMENT_ID_KEY: &str = "spec_document_id";

/// Orchestrates the worker pool and the lifecycle of a single proof job.
///
/// `JobRunner` is cloneable; clones share the metrics registry and the same
/// backend clients, but each holds its own `is_running` flag and an independent
/// view of the queue handle. The shared queue lives behind an `Arc` so all
/// clones (workers) enqueue/dequeue against the same queue.
#[derive(Clone)]
pub struct JobRunner {
    config: Config,
    /// Retained for sandbox validation hooks used by the container lifecycle.
    #[allow(dead_code)]
    security_manager: SecurityManager,
    storage_manager: StorageManager,
    lean_compiler: LeanCompiler,
    sandbox: Sandbox,
    job_queue: Arc<JobQueue>,
    metrics: Arc<Metrics>,
    worker_count: usize,
    max_job_duration: Duration,
    is_running: Arc<RwLock<bool>>,
}

impl JobRunner {
    pub async fn new(
        config: Config,
        security_manager: SecurityManager,
        metrics: Arc<Metrics>,
    ) -> Result<Self, BoxError> {
        let storage_manager = StorageManager::new(&config.storage).await?;
        let lean_compiler = LeanCompiler::new(&config.lean);
        let sandbox = Sandbox::from_config(&config);
        let job_queue = Arc::new(JobQueue::new(config.job.max_queue_size));
        let worker_count = config.job.worker_count;
        let max_job_duration = Duration::from_secs(config.job.max_job_duration_seconds);

        Ok(Self {
            config,
            security_manager,
            storage_manager,
            lean_compiler,
            sandbox,
            job_queue,
            metrics,
            worker_count,
            max_job_duration,
            is_running: Arc::new(RwLock::new(false)),
        })
    }

    /// Enqueue a job onto the shared queue.
    pub async fn enqueue(&self, job: ProofJob) -> Result<(), BoxError> {
        self.job_queue.enqueue(job).await
    }

    /// Number of jobs currently queued.
    pub async fn queue_size(&self) -> usize {
        self.job_queue.size().await
    }

    /// True when the container runtime required for real proving is reachable.
    pub async fn sandbox_available(&self) -> bool {
        self.sandbox.ensure_available().await.is_ok()
    }

    /// Run a single job to completion synchronously (process + persist), used by
    /// [`crate::LeanFarm::prove`] and the end-to-end test path.
    pub async fn run_job_to_completion(&self, job: ProofJob) -> Result<ProofResult, BoxError> {
        let result = self.process_job(job).await;
        self.handle_job_result(&result).await?;
        Ok(result)
    }

    pub async fn start_processing(&self) -> Result<(), BoxError> {
        info!("Starting job processing with {} workers", self.worker_count);

        {
            let mut is_running = self.is_running.write().await;
            *is_running = true;
        }

        let (tx, mut rx) = mpsc::channel(100);

        // Start worker pool
        let mut worker_handles = Vec::new();
        for worker_id in 0..self.worker_count {
            let tx = tx.clone();
            let job_runner = self.clone();
            let handle = tokio::spawn(async move {
                job_runner.worker_loop(worker_id, tx).await;
            });
            worker_handles.push(handle);
        }
        // Drop our own sender so the channel closes once all workers stop.
        drop(tx);

        // Process results
        while let Some(result) = rx.recv().await {
            if let Err(e) = self.handle_job_result(&result).await {
                error!("Failed to handle job result for {}: {}", result.job_id, e);
            }
        }

        // Wait for all workers to complete
        for handle in worker_handles {
            let _ = handle.await;
        }

        Ok(())
    }

    #[instrument(skip(self, tx))]
    async fn worker_loop(&self, worker_id: usize, tx: mpsc::Sender<ProofResult>) {
        info!("Worker {} started", worker_id);

        while *self.is_running.read().await {
            // Get next job from queue
            let job = match self.job_queue.dequeue().await {
                Some(job) => job,
                None => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };

            info!("Worker {} processing job {}", worker_id, job.id);

            // Process the job
            let result = self.process_job(job).await;

            // Send result back
            if let Err(e) = tx.send(result).await {
                error!("Failed to send job result: {}", e);
            }
        }

        info!("Worker {} stopped", worker_id);
    }

    /// Run the full proving pipeline for one job:
    /// generate self-contained Lean source → execute it in the hardened
    /// sandbox → build a structured artifact → upload it when storage is
    /// configured. Failures (compilation errors, unsolved goals, timeouts,
    /// missing runtime) are captured as a non-successful [`ProofResult`] with a
    /// structured error message rather than panicking.
    #[instrument(skip(self, job), fields(job_id = %job.id, theorem = %job.theorem.theorem_name))]
    async fn process_job(&self, job: ProofJob) -> ProofResult {
        let start_time = Instant::now();

        info!(
            "Processing job {} with theorem {}",
            job.id, job.theorem.theorem_name
        );

        // Check deadline before doing any work.
        if let Some(deadline) = job.deadline {
            if Instant::now() > deadline {
                return self.failed_result(
                    job,
                    start_time,
                    "Job deadline exceeded before execution".to_string(),
                );
            }
        }

        // 1. Generate the self-contained Lean source (deterministic, pure).
        let source = match self
            .lean_compiler
            .generate_proof(&job.theorem, &job.options)
            .await
        {
            Ok(source) => source,
            Err(e) => {
                return self.failed_result(
                    job,
                    start_time,
                    format!("failed to generate Lean source: {e}"),
                );
            }
        };

        // 2. Execute the source in the hardened sandbox with a bounded budget.
        let budget = self.time_budget(&job.options);
        let outcome = match self.sandbox.run_lean_source(&source, budget).await {
            Ok(outcome) => outcome,
            Err(e) => {
                // Runtime unavailable or a spawn error: this is an
                // infrastructure fault, surfaced verbatim.
                return self.failed_result(job, start_time, e.to_string());
            }
        };

        // 3. Build the structured artifact from the real execution outcome.
        let proof_artifact =
            self.artifact_from_outcome(&job.theorem, &job.options, &source, &outcome);
        let success = outcome.success;
        let error_message = outcome.failure_message();

        // 4. Upload the artifact when it represents a real proof and storage is
        //    configured (a `NotConfigured` sink is logged, not fatal).
        if success {
            if let Err(e) = self.upload_proof_artifact(&proof_artifact).await {
                warn!(
                    "Failed to upload proof artifact {}: {}",
                    proof_artifact.id, e
                );
            }
        }

        let resource_usage = proof_artifact.resource_usage.clone().unwrap_or_default();

        ProofResult {
            job_id: job.id,
            theorem: job.theorem,
            proof_artifact,
            duration_ms: start_time.elapsed().as_millis() as u64,
            success,
            error_message,
            resource_usage,
        }
    }

    /// Compute the effective time budget for a job: the smaller of the job's
    /// requested timeout and the runner-wide ceiling, and never zero.
    fn time_budget(&self, options: &ProofOptions) -> Duration {
        let requested = Duration::from_secs(options.timeout_seconds.max(1) as u64);
        requested
            .min(self.max_job_duration)
            .max(Duration::from_secs(1))
    }

    /// Build a [`ProofArtifact`] from a completed sandbox run. The artifact is
    /// content-addressed by the SHA-256 of the exact source that was checked,
    /// which keeps storage keys stable and deduplicated.
    fn artifact_from_outcome(
        &self,
        theorem: &LeanTheorem,
        options: &ProofOptions,
        source: &str,
        outcome: &SandboxOutcome,
    ) -> ProofArtifact {
        let duration_ms = outcome.duration.as_millis() as i64;
        let mut metadata = HashMap::new();
        if let Some(spec_doc) = options.metadata.get(SPEC_DOCUMENT_ID_KEY) {
            metadata.insert(SPEC_DOCUMENT_ID_KEY.to_string(), spec_doc.clone());
        }
        metadata.insert(
            "exit_code".to_string(),
            outcome
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string()),
        );
        metadata.insert("timed_out".to_string(), outcome.timed_out.to_string());
        metadata.insert("sandbox_image".to_string(), self.config.lean.image.clone());

        ProofArtifact {
            id: format!("proof-{}", uuid::Uuid::new_v4()),
            content_sha256: sha256::digest(source),
            theorem_id: theorem.id.clone(),
            invariant_id: theorem.source_invariant_id.clone(),
            status: if outcome.success {
                ProofStatus::Success as i32
            } else {
                ProofStatus::Failed as i32
            },
            attempted_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            duration_ms,
            output: outcome.stdout.clone(),
            logs: outcome.stderr.lines().map(|l| l.to_string()).collect(),
            resource_usage: Some(ResourceUsage {
                // The sandbox is the CPU-bound worker here, so wall-clock time
                // is a faithful proxy for CPU seconds consumed by the run.
                cpu_seconds: outcome.duration.as_secs_f64(),
                memory_bytes: self.config.security.resource_limits.memory_limit as i64,
                disk_bytes: source.len() as i64,
                network_bytes: 0,
            }),
            proof_strategy: options.proof_strategy.clone(),
            // Confidence is only meaningful once Lean has actually checked the
            // proof: certainty on success, none on failure.
            confidence_score: if outcome.success { 1.0 } else { 0.0 },
            metadata,
        }
    }

    /// Construct a failed [`ProofResult`] with a structured message. Centralised
    /// so every early-return path is consistent.
    fn failed_result(&self, job: ProofJob, start_time: Instant, message: String) -> ProofResult {
        warn!("Job {} failed: {}", job.id, message);
        let mut artifact = ProofArtifact {
            id: format!("proof-{}", uuid::Uuid::new_v4()),
            theorem_id: job.theorem.id.clone(),
            invariant_id: job.theorem.source_invariant_id.clone(),
            status: ProofStatus::Failed as i32,
            attempted_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            proof_strategy: job.options.proof_strategy.clone(),
            ..Default::default()
        };
        if let Some(spec_doc) = job.options.metadata.get(SPEC_DOCUMENT_ID_KEY) {
            artifact
                .metadata
                .insert(SPEC_DOCUMENT_ID_KEY.to_string(), spec_doc.clone());
        }
        ProofResult {
            job_id: job.id,
            theorem: job.theorem,
            proof_artifact: artifact,
            duration_ms: start_time.elapsed().as_millis() as u64,
            success: false,
            error_message: Some(message),
            resource_usage: ResourceUsage::default(),
        }
    }

    async fn upload_proof_artifact(&self, proof_artifact: &ProofArtifact) -> Result<(), BoxError> {
        let artifact_key = format!(
            "{}/{}",
            self.config.storage.minio.key_prefix, proof_artifact.id
        );

        info!(
            "Uploading proof artifact to artifact store: {}",
            artifact_key
        );

        // Serialize proof artifact to protobuf
        let artifact_bytes = proof_artifact.encode_to_vec();

        self.storage_manager
            .upload_to_minio(&artifact_key, &artifact_bytes)
            .await?;

        info!("Successfully uploaded proof artifact {}", proof_artifact.id);
        Ok(())
    }

    async fn handle_job_result(&self, result: &ProofResult) -> Result<(), BoxError> {
        if result.success {
            info!(
                "Job {} completed successfully in {}ms",
                result.job_id, result.duration_ms
            );
        } else {
            warn!("Job {} failed: {:?}", result.job_id, result.error_message);
        }

        // Update metrics
        self.update_job_metrics(result).await;

        // Store the compact result summary in persistent storage (no-op when no
        // artifact bucket is configured).
        self.storage_manager.store_job_result(result).await?;

        // Emit the cross-service artifact reference consumed by the badge
        // service. This is best-effort: a mirroring failure must not fail the
        // job, so it is logged rather than propagated.
        self.emit_artifact_reference(result).await;

        Ok(())
    }

    /// Write the [`ProofArtifactReference`] for a completed job to the local
    /// mirror directory (when configured) and to the artifact bucket under the
    /// `references/` prefix (when configured).
    async fn emit_artifact_reference(&self, result: &ProofResult) {
        let spec_doc = result
            .proof_artifact
            .metadata
            .get(SPEC_DOCUMENT_ID_KEY)
            .map(String::as_str);
        let reference = ProofArtifactReference::from_result(result, spec_doc);

        if let Some(dir) = &self.config.storage.reference_dir {
            match write_reference_to_dir(std::path::Path::new(dir), &reference).await {
                Ok(path) => info!("Mirrored artifact reference to {}", path.display()),
                Err(e) => warn!("Failed to mirror artifact reference: {}", e),
            }
        }

        if !self.config.storage.minio.bucket.is_empty() {
            match reference.to_json() {
                Ok(payload) => {
                    let key = format!(
                        "{}/references/{}",
                        self.config.storage.minio.key_prefix,
                        reference.file_name()
                    );
                    if let Err(e) = self.storage_manager.upload_to_minio(&key, &payload).await {
                        warn!("Failed to upload artifact reference to store: {}", e);
                    }
                }
                Err(e) => warn!("Failed to serialise artifact reference: {}", e),
            }
        }
    }

    async fn update_job_metrics(&self, result: &ProofResult) {
        self.metrics.record_job(result.success, result.duration_ms);
    }

    pub async fn start_health_server(&self) -> Result<(), BoxError> {
        use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
        use serde_json::json;

        let port = self.config.metrics.health_port;
        let queue = self.job_queue.clone();

        let app = Router::new()
            .route("/health", get(|| async { StatusCode::OK }))
            .route("/ready", get(|| async { StatusCode::OK }))
            .route(
                "/status",
                get(|State(queue): State<Arc<JobQueue>>| async move {
                    Json(json!({ "queue_size": queue.size().await }))
                }),
            )
            .with_state(queue);

        let addr = format!("0.0.0.0:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("failed to bind health server on {addr}: {e}"))?;
        info!("health server listening on {addr}");

        axum::serve(listener, app)
            .await
            .map_err(|e| format!("health server error: {e}"))?;
        Ok(())
    }

    pub async fn stop(&self) {
        info!("Stopping job runner");
        let mut is_running = self.is_running.write().await;
        *is_running = false;
    }
}
