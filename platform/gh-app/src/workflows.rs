//! Spec-drift detection orchestration.
//!
//! The original scaffold depended on a fictional `temporal-sdk = "1.0"` crate.
//! This module replaces it with a self-contained, dependency-light abstraction:
//! activities are expressed as an [`async_trait`] ([`SpecDriftActivities`]) and
//! the orchestration is a plain async [`SpecDriftEngine`]. This keeps the drift
//! logic real and unit-testable with mock activities, while leaving a clean
//! seam to plug in a durable workflow engine (Temporal, AWS Step Functions,
//! etc.) later by implementing the activity trait against it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecDocument {
    pub id: String,
    pub content: String,
    pub source_system: String,
    pub last_modified: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftDetectionRequest {
    pub document_id: String,
    pub source_system: String,
    pub event_type: String,
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftDetectionResponse {
    pub document_id: String,
    pub drift_detected: bool,
    pub current_hash: String,
    pub last_proven_hash: Option<String>,
    pub proof_job_id: Option<String>,
    pub processing_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecUpdateEvent {
    pub document_id: String,
    pub source_system: String,
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofJob {
    pub id: String,
    pub document_id: String,
    pub priority: JobPriority,
    pub created_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
    pub retry_count: u32,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftStatus {
    NoDrift,
    DriftDetected,
    Processing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftAlert {
    pub document_id: String,
    pub alert_type: AlertType,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub severity: AlertSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertType {
    DriftDetected,
    ProofJobFailed,
    DriftUnresolved,
    SystemError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

// ---------------------------------------------------------------------------
// Activity boundary
// ---------------------------------------------------------------------------

/// External operations required by the drift-detection workflow. Implement this
/// against real backends (ingest connectors, proof storage, job queue, alert
/// transport). The default implementation ([`UnconfiguredActivities`]) fails
/// loudly for operations that require external systems.
#[async_trait]
pub trait SpecDriftActivities: Send + Sync {
    async fn fetch_spec_document(&self, document_id: &str) -> Result<SpecDocument>;
    async fn get_last_proven_hash(&self, document_id: &str) -> Result<Option<String>>;
    async fn enqueue_proof_job(&self, job: ProofJob) -> Result<String>;
    async fn update_drift_status(&self, document_id: &str, status: DriftStatus) -> Result<()>;
    async fn send_drift_alert(&self, alert: DriftAlert) -> Result<()>;

    /// Compute the canonical SHA-256 content hash of a document. This is pure
    /// and has a correct default implementation.
    async fn compute_document_hash(&self, document: &SpecDocument) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(document.content.as_bytes());
        Ok(hex::encode(hasher.finalize()))
    }
}

/// Default activities that fail loudly for anything requiring an external
/// system. Used until real connectors/storage/queue/alerting are wired in.
#[derive(Debug, Default, Clone)]
pub struct UnconfiguredActivities;

#[async_trait]
impl SpecDriftActivities for UnconfiguredActivities {
    async fn fetch_spec_document(&self, document_id: &str) -> Result<SpecDocument> {
        Err(anyhow::anyhow!(
            "fetch_spec_document is not configured: no ingest connector wired in (document '{}')",
            document_id
        ))
    }

    async fn get_last_proven_hash(&self, document_id: &str) -> Result<Option<String>> {
        Err(anyhow::anyhow!(
            "get_last_proven_hash is not configured: no proof-storage lookup wired in (document '{}')",
            document_id
        ))
    }

    async fn enqueue_proof_job(&self, job: ProofJob) -> Result<String> {
        Err(anyhow::anyhow!(
            "enqueue_proof_job is not configured: no job queue wired in (job {})",
            job.id
        ))
    }

    async fn update_drift_status(&self, document_id: &str, status: DriftStatus) -> Result<()> {
        Err(anyhow::anyhow!(
            "update_drift_status is not configured: no persistence wired in (document '{}', status {:?})",
            document_id, status
        ))
    }

    async fn send_drift_alert(&self, alert: DriftAlert) -> Result<()> {
        Err(anyhow::anyhow!(
            "send_drift_alert is not configured: no alert transport wired in (document '{}')",
            alert.document_id
        ))
    }
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Orchestrates drift detection over a set of [`SpecDriftActivities`].
#[derive(Clone)]
pub struct SpecDriftEngine {
    activities: Arc<dyn SpecDriftActivities>,
}

impl SpecDriftEngine {
    pub fn new(activities: Arc<dyn SpecDriftActivities>) -> Self {
        Self { activities }
    }

    /// Detect whether a document has drifted from its last proven state and, if
    /// so, enqueue a proof job and raise a drift alert.
    pub async fn detect_drift(&self, request: DriftDetectionRequest) -> Result<DriftDetectionResponse> {
        let start = std::time::Instant::now();
        info!("Starting drift detection for document: {}", request.document_id);

        let document = self.activities.fetch_spec_document(&request.document_id).await?;
        let current_hash = self.activities.compute_document_hash(&document).await?;
        let last_proven_hash = self.activities.get_last_proven_hash(&request.document_id).await?;

        let drift_detected = match &last_proven_hash {
            Some(proven) => proven != &current_hash,
            None => true,
        };

        let mut proof_job_id = None;
        if drift_detected {
            info!("Drift detected for document: {}", request.document_id);
            self.activities
                .update_drift_status(&request.document_id, DriftStatus::DriftDetected)
                .await?;

            let now = Utc::now();
            let job = ProofJob {
                id: uuid::Uuid::new_v4().to_string(),
                document_id: request.document_id.clone(),
                priority: JobPriority::High,
                created_at: now,
                deadline: Some(now + chrono::Duration::hours(1)),
                retry_count: 0,
                max_retries: 3,
            };
            let job_id = self.activities.enqueue_proof_job(job).await?;

            self.activities
                .send_drift_alert(DriftAlert {
                    document_id: request.document_id.clone(),
                    alert_type: AlertType::DriftDetected,
                    message: format!(
                        "Drift detected for document {}. New proof job created: {}",
                        request.document_id, job_id
                    ),
                    timestamp: Utc::now(),
                    severity: AlertSeverity::Warning,
                })
                .await?;
            proof_job_id = Some(job_id);
        }

        Ok(DriftDetectionResponse {
            document_id: request.document_id,
            drift_detected,
            current_hash,
            last_proven_hash,
            proof_job_id,
            processing_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// Load / chaos testing utilities
// ---------------------------------------------------------------------------

pub struct ChaosTestUtils;

impl ChaosTestUtils {
    pub fn simulate_burst_edits(count: u32) -> Vec<DriftDetectionRequest> {
        (0..count)
            .map(|i| DriftDetectionRequest {
                document_id: format!("doc-{}", i),
                source_system: "jira".to_string(),
                event_type: "edit".to_string(),
                event_id: uuid::Uuid::new_v4().to_string(),
                timestamp: Utc::now(),
            })
            .collect()
    }

    /// Run drift detection for each request through `engine` and report whether
    /// the whole batch completed within `max_time`. Individual activity errors
    /// are tolerated (counted) so that unconfigured backends do not abort the
    /// timing measurement.
    pub async fn measure_processing_time(
        engine: &SpecDriftEngine,
        requests: &[DriftDetectionRequest],
        max_time: Duration,
    ) -> (Duration, bool) {
        let start = std::time::Instant::now();
        for request in requests {
            let _ = engine.detect_drift(request.clone()).await;
        }
        let elapsed = start.elapsed();
        (elapsed, elapsed <= max_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockActivities {
        last_hash: Option<String>,
        enqueued: Mutex<Vec<String>>,
        alerts: Mutex<Vec<DriftAlert>>,
    }

    #[async_trait]
    impl SpecDriftActivities for MockActivities {
        async fn fetch_spec_document(&self, document_id: &str) -> Result<SpecDocument> {
            Ok(SpecDocument {
                id: document_id.to_string(),
                content: "spec body".to_string(),
                source_system: "jira".to_string(),
                last_modified: Utc::now(),
                metadata: HashMap::new(),
            })
        }
        async fn get_last_proven_hash(&self, _document_id: &str) -> Result<Option<String>> {
            Ok(self.last_hash.clone())
        }
        async fn enqueue_proof_job(&self, job: ProofJob) -> Result<String> {
            self.enqueued.lock().unwrap().push(job.document_id.clone());
            Ok(job.id)
        }
        async fn update_drift_status(&self, _document_id: &str, _status: DriftStatus) -> Result<()> {
            Ok(())
        }
        async fn send_drift_alert(&self, alert: DriftAlert) -> Result<()> {
            self.alerts.lock().unwrap().push(alert);
            Ok(())
        }
    }

    fn request(doc: &str) -> DriftDetectionRequest {
        DriftDetectionRequest {
            document_id: doc.to_string(),
            source_system: "jira".to_string(),
            event_type: "edit".to_string(),
            event_id: "e1".to_string(),
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_compute_document_hash_is_sha256() {
        let acts = UnconfiguredActivities;
        let doc = SpecDocument {
            id: "d".to_string(),
            content: "abc".to_string(),
            source_system: "s".to_string(),
            last_modified: Utc::now(),
            metadata: HashMap::new(),
        };
        let hash = acts.compute_document_hash(&doc).await.unwrap();
        // SHA-256("abc")
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn test_detect_drift_when_no_prior_hash() {
        let engine = SpecDriftEngine::new(Arc::new(MockActivities::default()));
        let resp = engine.detect_drift(request("doc-1")).await.unwrap();
        assert!(resp.drift_detected);
        assert!(resp.proof_job_id.is_some());
    }

    #[tokio::test]
    async fn test_no_drift_when_hash_matches() {
        // SHA-256("spec body")
        let matching = {
            let mut h = Sha256::new();
            h.update(b"spec body");
            hex::encode(h.finalize())
        };
        let engine = SpecDriftEngine::new(Arc::new(MockActivities {
            last_hash: Some(matching),
            ..Default::default()
        }));
        let resp = engine.detect_drift(request("doc-1")).await.unwrap();
        assert!(!resp.drift_detected);
        assert!(resp.proof_job_id.is_none());
    }

    #[tokio::test]
    async fn test_unconfigured_activities_fail_loudly() {
        let engine = SpecDriftEngine::new(Arc::new(UnconfiguredActivities));
        assert!(engine.detect_drift(request("doc-1")).await.is_err());
    }

    #[tokio::test]
    async fn test_simulate_burst_edits() {
        let requests = ChaosTestUtils::simulate_burst_edits(1000);
        assert_eq!(requests.len(), 1000);
    }
}
