//! Lean Farm: a secure, horizontally scalable job runner for Lean theorem proving.
//!
//! The crate is organised around a small number of composable pieces:
//! - [`config::Config`] loads runtime configuration (YAML file + defaults).
//! - [`security::SecurityManager`] validates the sandbox/runtime posture.
//! - [`storage::StorageManager`] talks to S3-compatible object stores.
//! - [`lean::LeanCompiler`] turns a theorem plus [`ProofOptions`] into Lean code.
//! - [`job_runner::JobRunner`] orchestrates the worker pool and job lifecycle.

pub mod artifact;
pub mod config;
pub mod job_runner;
pub mod lean;
pub mod metrics;
pub mod proto;
pub mod sandbox;
pub mod security;
pub mod storage;

use std::time::Instant;
use tokio::sync::RwLock;
use tracing::info;

use crate::proto::spec_to_proof::v1::{LeanTheorem, ProofArtifact, ResourceUsage};

pub use crate::config::Config;

/// Shared boxed error type. It is `Send + Sync` so that fallible futures can be
/// driven on the multi-threaded Tokio runtime and moved across `tokio::spawn`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Lean Farm top-level orchestrator.
///
/// A `LeanFarm` is cheaply cloneable: every component either wraps its shared
/// state in an `Arc`/handle or is trivially copyable, so clones observe the same
/// configuration and backends. This is what lets the runner fan work out across
/// spawned tasks.
#[derive(Clone)]
pub struct LeanFarm {
    config: config::Config,
    job_runner: job_runner::JobRunner,
    /// Exposed so callers (and tests) can inspect the detected runtime posture.
    pub security_manager: security::SecurityManager,
    metrics_server: metrics::MetricsServer,
    start_time: Instant,
}

impl LeanFarm {
    pub async fn new(config: config::Config) -> Result<Self, BoxError> {
        let security_manager = security::SecurityManager::new(&config.security)?;
        let metrics = metrics::Metrics::new();
        let job_runner =
            job_runner::JobRunner::new(config.clone(), security_manager.clone(), metrics.clone())
                .await?;
        let metrics_server = metrics::MetricsServer::new(
            config.metrics.port,
            config.metrics.path.clone(),
            metrics,
        );

        Ok(Self {
            config,
            job_runner,
            security_manager,
            metrics_server,
            start_time: Instant::now(),
        })
    }

    /// Enqueue a job for processing by the worker pool.
    pub async fn submit(&self, job: ProofJob) -> Result<(), BoxError> {
        self.job_runner.enqueue(job).await
    }

    /// Synchronously run a single job to completion and return its result.
    ///
    /// This drives the same real proving path used by the worker pool (Lean
    /// sandbox execution, artifact upload, result persistence and reference
    /// mirroring) without going through the queue. It is the entrypoint used by
    /// the end-to-end proving test and by scripting/CLI callers that want a
    /// blocking prove.
    pub async fn prove(&self, job: ProofJob) -> Result<ProofResult, BoxError> {
        self.job_runner.run_job_to_completion(job).await
    }

    /// True when the container runtime required for real proving is reachable.
    pub async fn sandbox_available(&self) -> bool {
        self.job_runner.sandbox_available().await
    }

    /// Number of jobs currently waiting in the queue.
    pub async fn queue_size(&self) -> usize {
        self.job_runner.queue_size().await
    }

    /// Start the metrics server, health server and worker pool, and run until
    /// one of them returns (which, in normal operation, only happens on error
    /// or shutdown).
    pub async fn start(&self) -> Result<(), BoxError> {
        info!("Starting Lean Farm");

        self.security_manager.validate_environment().await?;
        info!("Security validation passed");

        // Each spawned task owns its own clone so the futures are `'static`.
        let metrics = self.metrics_server.clone();
        let jr_health = self.job_runner.clone();
        let jr_proc = self.job_runner.clone();

        let metrics_handle = tokio::spawn(async move { metrics.start().await });
        let health_handle = tokio::spawn(async move { jr_health.start_health_server().await });
        let job_handle = tokio::spawn(async move { jr_proc.start_processing().await });

        let (metrics_res, health_res, job_res) =
            tokio::join!(metrics_handle, health_handle, job_handle);

        // First unwrap the `JoinError` (task panics/cancellation), then the
        // inner task result.
        metrics_res??;
        health_res??;
        job_res??;

        Ok(())
    }

    pub fn get_uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub fn config(&self) -> &config::Config {
        &self.config
    }
}

/// A single Lean theorem proving job.
#[derive(Debug, Clone)]
pub struct ProofJob {
    pub id: String,
    pub theorem: LeanTheorem,
    pub options: ProofOptions,
    pub priority: JobPriority,
    pub created_at: Instant,
    pub deadline: Option<Instant>,
}

/// Options controlling how a proof is generated for a job.
///
/// This is a Lean-Farm domain type (distinct from the gRPC `ProofOptions`
/// message) because the farm cares about scheduling/resource concerns
/// (`resource_limits`, `confidence_threshold`) that are not part of the wire
/// contract used by the proof-generation service.
#[derive(Debug, Clone)]
pub struct ProofOptions {
    pub max_attempts: u32,
    pub timeout_seconds: u32,
    pub proof_strategy: String,
    pub confidence_threshold: f64,
    pub resource_limits: Option<ResourceLimits>,
    pub metadata: std::collections::HashMap<String, String>,
}

impl Default for ProofOptions {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            timeout_seconds: 300,
            proof_strategy: "auto".to_string(),
            confidence_threshold: 0.8,
            resource_limits: None,
            metadata: std::collections::HashMap::new(),
        }
    }
}

/// Per-job resource ceilings applied when running a proof.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceLimits {
    pub cpu_seconds: f64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub network_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl From<i32> for JobPriority {
    fn from(value: i32) -> Self {
        match value {
            0 => JobPriority::Low,
            1 => JobPriority::Normal,
            2 => JobPriority::High,
            3 => JobPriority::Critical,
            _ => JobPriority::Normal,
        }
    }
}

/// Outcome of a proof job.
#[derive(Debug, Clone)]
pub struct ProofResult {
    pub job_id: String,
    pub theorem: LeanTheorem,
    pub proof_artifact: ProofArtifact,
    pub duration_ms: u64,
    pub success: bool,
    pub error_message: Option<String>,
    pub resource_usage: ResourceUsage,
}

/// Priority-ordered job queue with a bounded capacity.
#[derive(Debug)]
pub struct JobQueue {
    jobs: RwLock<Vec<ProofJob>>,
    max_queue_size: usize,
}

impl JobQueue {
    pub fn new(max_queue_size: usize) -> Self {
        Self {
            jobs: RwLock::new(Vec::new()),
            max_queue_size,
        }
    }

    /// Enqueue a job. Returns [`LeanFarmError::ResourceLimit`] if the queue is
    /// at capacity. Jobs are kept sorted so that the highest priority job is at
    /// the end of the vector (and therefore popped first by [`Self::dequeue`]).
    pub async fn enqueue(&self, job: ProofJob) -> Result<(), BoxError> {
        let mut jobs = self.jobs.write().await;

        if jobs.len() >= self.max_queue_size {
            return Err(LeanFarmError::ResourceLimit("Job queue is full".to_string()).into());
        }

        let job_id = job.id.clone();
        let priority = job.priority;
        jobs.push(job);
        jobs.sort_by(|a, b| a.priority.cmp(&b.priority));

        info!("Job {} enqueued with priority {:?}", job_id, priority);
        Ok(())
    }

    pub async fn dequeue(&self) -> Option<ProofJob> {
        let mut jobs = self.jobs.write().await;
        jobs.pop()
    }

    pub async fn size(&self) -> usize {
        self.jobs.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.jobs.read().await.is_empty()
    }
}

/// Error types for the Lean Farm.
#[derive(Debug, thiserror::Error)]
pub enum LeanFarmError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Security validation failed: {0}")]
    Security(String),

    #[error("Job execution failed: {0}")]
    JobExecution(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Lean compilation failed: {0}")]
    LeanCompilation(String),

    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Not configured: {0}")]
    NotConfigured(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::spec_to_proof::v1::LeanTheorem;

    fn sample_job(id: &str, priority: JobPriority) -> ProofJob {
        ProofJob {
            id: id.to_string(),
            theorem: LeanTheorem::default(),
            options: ProofOptions::default(),
            priority,
            created_at: Instant::now(),
            deadline: None,
        }
    }

    #[tokio::test]
    async fn queue_pops_highest_priority_first() {
        let queue = JobQueue::new(16);
        queue.enqueue(sample_job("low", JobPriority::Low)).await.unwrap();
        queue.enqueue(sample_job("crit", JobPriority::Critical)).await.unwrap();
        queue.enqueue(sample_job("normal", JobPriority::Normal)).await.unwrap();

        assert_eq!(queue.size().await, 3);
        assert_eq!(queue.dequeue().await.unwrap().id, "crit");
        assert_eq!(queue.dequeue().await.unwrap().id, "normal");
        assert_eq!(queue.dequeue().await.unwrap().id, "low");
        assert!(queue.is_empty().await);
    }

    #[tokio::test]
    async fn queue_rejects_when_full() {
        let queue = JobQueue::new(1);
        queue.enqueue(sample_job("a", JobPriority::Normal)).await.unwrap();
        let err = queue
            .enqueue(sample_job("b", JobPriority::Normal))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("queue is full"));
    }

    #[test]
    fn job_priority_from_i32_is_total() {
        assert_eq!(JobPriority::from(0), JobPriority::Low);
        assert_eq!(JobPriority::from(3), JobPriority::Critical);
        assert_eq!(JobPriority::from(99), JobPriority::Normal);
    }
}
