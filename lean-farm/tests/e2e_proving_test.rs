//! End-to-end proving tests exercising the *real* Lean execution path.
//!
//! These tests run an actual Lean toolchain inside a hardened container, so
//! they require Docker (or a compatible runtime) and network access to pull the
//! `leanprover/lean4` image on first run. To keep CI green on machines without
//! Docker they are gated behind the `E2E_DOCKER=1` environment variable and are
//! otherwise skipped with an explanatory message.
//!
//! Run locally with:
//!
//! ```powershell
//! $env:E2E_DOCKER = "1"; cargo test --test e2e_proving_test -- --nocapture
//! ```
//!
//! The default configuration generates stdlib-only proofs and runs with
//! `--network=none`, so no Mathlib or registry access is needed once the base
//! image is present.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use lean_farm::proto::spec_to_proof::v1::{LeanTheorem, TheoremStatus};
use lean_farm::{Config, JobPriority, LeanFarm, ProofJob, ProofOptions};

fn docker_e2e_enabled() -> bool {
    std::env::var("E2E_DOCKER")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn theorem(name: &str, code: &str) -> LeanTheorem {
    LeanTheorem {
        id: format!("test-{name}"),
        content_sha256: sha256::digest(code),
        theorem_name: name.to_string(),
        lean_code: code.to_string(),
        source_invariant_id: format!("INV-{name}"),
        generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        status: TheoremStatus::Generated as i32,
        compilation_errors: Vec::new(),
        proof_strategy: "decide".to_string(),
        metadata: HashMap::new(),
    }
}

fn job(id: &str, theorem: LeanTheorem, strategy: &str, spec_doc: &str) -> ProofJob {
    let mut metadata = HashMap::new();
    metadata.insert("spec_document_id".to_string(), spec_doc.to_string());
    ProofJob {
        id: id.to_string(),
        theorem,
        options: ProofOptions {
            max_attempts: 1,
            timeout_seconds: 120,
            proof_strategy: strategy.to_string(),
            confidence_threshold: 0.8,
            resource_limits: None,
            metadata,
        },
        priority: JobPriority::Normal,
        created_at: Instant::now(),
        deadline: Some(Instant::now() + Duration::from_secs(600)),
    }
}

async fn farm() -> LeanFarm {
    LeanFarm::new(Config::default())
        .await
        .expect("farm initialises with defaults")
}

async fn farm_with_reference_dir(dir: &std::path::Path) -> LeanFarm {
    let mut config = Config::default();
    config.storage.reference_dir = Some(dir.to_string_lossy().into_owned());
    LeanFarm::new(config)
        .await
        .expect("farm initialises with reference dir")
}

#[tokio::test]
async fn e2e_valid_theorem_is_proven() {
    if !docker_e2e_enabled() {
        eprintln!("skipping e2e_valid_theorem_is_proven: set E2E_DOCKER=1 to run");
        return;
    }
    let farm = farm().await;
    assert!(
        farm.sandbox_available().await,
        "E2E_DOCKER=1 but the container runtime is not reachable"
    );

    // A true, self-contained proposition provable with `decide` over the Lean
    // standard library (no Mathlib required).
    let job = job(
        "valid",
        theorem("e2e_valid", "theorem e2e_valid : 1 + 1 = 2"),
        "decide",
        "DOC-VALID",
    );

    let result = farm.prove(job).await.expect("prove completes");
    assert!(
        result.success,
        "expected proof to succeed; error = {:?}, logs = {:?}",
        result.error_message, result.proof_artifact.logs
    );
    assert_eq!(
        result.proof_artifact.status,
        lean_farm::proto::spec_to_proof::v1::ProofStatus::Success as i32
    );
    assert!(result.error_message.is_none());
    assert!(!result.proof_artifact.content_sha256.is_empty());
}

#[tokio::test]
async fn e2e_false_theorem_fails_with_diagnostics() {
    if !docker_e2e_enabled() {
        eprintln!("skipping e2e_false_theorem_fails_with_diagnostics: set E2E_DOCKER=1 to run");
        return;
    }
    let farm = farm().await;
    assert!(farm.sandbox_available().await);

    // A false proposition: `decide` must reject it, so the sandbox exits
    // non-zero and the job is reported as failed with real diagnostics.
    let job = job(
        "false",
        theorem("e2e_false", "theorem e2e_false : 1 + 1 = 3"),
        "decide",
        "DOC-FALSE",
    );

    let result = farm.prove(job).await.expect("prove completes");
    assert!(
        !result.success,
        "a false theorem must not be reported proven"
    );
    assert!(
        result.error_message.is_some(),
        "a failed proof must carry a structured error message"
    );
    assert_eq!(
        result.proof_artifact.status,
        lean_farm::proto::spec_to_proof::v1::ProofStatus::Failed as i32
    );
}

#[tokio::test]
async fn e2e_proof_emits_badge_reference() {
    if !docker_e2e_enabled() {
        eprintln!("skipping e2e_proof_emits_badge_reference: set E2E_DOCKER=1 to run");
        return;
    }
    let dir = std::env::temp_dir().join(format!("lean-farm-refs-{}", uuid::Uuid::new_v4()));
    let farm = farm_with_reference_dir(&dir).await;
    assert!(farm.sandbox_available().await);

    let job = job(
        "ref",
        theorem("e2e_ref", "theorem e2e_ref : 2 = 2"),
        "rfl",
        "DOC-REF",
    );
    let result = farm.prove(job).await.expect("prove completes");
    assert!(result.success, "error = {:?}", result.error_message);

    // The farm must have mirrored a ProofArtifactReference keyed by the spec
    // document id — this is exactly what the GitHub App badge service consumes.
    let reference_path = dir.join("DOC-REF.json");
    let contents = tokio::fs::read_to_string(&reference_path)
        .await
        .unwrap_or_else(|e| panic!("reference {} not written: {e}", reference_path.display()));
    assert!(
        contents.contains("\"status\": \"proven\""),
        "got: {contents}"
    );
    assert!(
        contents.contains("\"spec_document_id\": \"DOC-REF\""),
        "got: {contents}"
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
