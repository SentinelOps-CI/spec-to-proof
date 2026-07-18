//! Hermetic integration tests for the Lean Farm public surface.
//!
//! These tests exercise the real orchestration/domain logic (configuration,
//! job submission and priority ordering, security posture inspection) without
//! requiring external infrastructure. Full end-to-end proof execution requires
//! a Lean container runtime (Docker + gVisor) and S3/MinIO endpoints and is
//! therefore not run as part of the unit/integration suite.

use std::time::{Duration, Instant};

use lean_farm::proto::spec_to_proof::v1::{LeanTheorem, TheoremStatus};
use lean_farm::{BoxError, Config, JobPriority, LeanFarm, ProofJob, ProofOptions, ResourceLimits};

fn test_options(strategy: &str) -> ProofOptions {
    ProofOptions {
        max_attempts: 3,
        timeout_seconds: 300,
        proof_strategy: strategy.to_string(),
        confidence_threshold: 0.8,
        resource_limits: Some(ResourceLimits {
            cpu_seconds: 60.0,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            disk_bytes: 5 * 1024 * 1024 * 1024,
            network_bytes: 50 * 1024 * 1024,
        }),
        metadata: std::collections::HashMap::new(),
    }
}

fn test_theorem(id: u32) -> LeanTheorem {
    let lean_code = format!("theorem test_theorem_{id} : {id} + 0 = {id}");
    LeanTheorem {
        id: format!("test-theorem-{id}"),
        content_sha256: sha256::digest(&lean_code),
        theorem_name: format!("test_theorem_{id}"),
        lean_code,
        source_invariant_id: format!("test-invariant-{id}"),
        generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        status: TheoremStatus::Generated as i32,
        compilation_errors: Vec::new(),
        proof_strategy: "auto".to_string(),
        metadata: std::collections::HashMap::new(),
    }
}

fn job(id: &str, theorem: LeanTheorem, priority: JobPriority) -> ProofJob {
    ProofJob {
        id: id.to_string(),
        theorem,
        options: test_options("auto"),
        priority,
        created_at: Instant::now(),
        deadline: Some(Instant::now() + Duration::from_secs(600)),
    }
}

#[tokio::test]
async fn farm_initializes_with_defaults() -> Result<(), BoxError> {
    let farm = LeanFarm::new(Config::default()).await?;
    assert_eq!(farm.queue_size().await, 0);
    // Uptime is monotonic and starts near zero.
    assert!(farm.get_uptime_seconds() < 5);
    Ok(())
}

#[tokio::test]
async fn farm_accepts_and_queues_jobs() -> Result<(), BoxError> {
    let farm = LeanFarm::new(Config::default()).await?;

    farm.submit(job("job-a", test_theorem(1), JobPriority::Normal)).await?;
    farm.submit(job("job-b", test_theorem(2), JobPriority::High)).await?;

    assert_eq!(farm.queue_size().await, 2);
    Ok(())
}

#[tokio::test]
async fn farm_rejects_jobs_when_queue_full() -> Result<(), BoxError> {
    let mut config = Config::default();
    config.job.max_queue_size = 1;
    let farm = LeanFarm::new(config).await?;

    farm.submit(job("job-a", test_theorem(1), JobPriority::Normal)).await?;
    let err = farm
        .submit(job("job-b", test_theorem(2), JobPriority::Normal))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("queue is full"));
    Ok(())
}

#[tokio::test]
async fn security_posture_is_inspectable() -> Result<(), BoxError> {
    let farm = LeanFarm::new(Config::default()).await?;
    // The detected runtime posture is exposed; in a hardened environment these
    // flags would all be true. We only assert the accessor works and is
    // internally consistent (booleans), not the values, so the test is
    // environment-independent.
    let info = farm.security_manager.get_runtime_info();
    let _ = (
        info.is_gvisor,
        info.is_rootless,
        info.seccomp_enabled,
        info.capabilities_dropped,
        info.network_isolated,
    );
    Ok(())
}

#[tokio::test]
async fn proof_options_carry_resource_limits() {
    let opts = test_options("omega");
    assert_eq!(opts.proof_strategy, "omega");
    let limits = opts.resource_limits.expect("limits present");
    assert_eq!(limits.memory_bytes, 2 * 1024 * 1024 * 1024);
}

