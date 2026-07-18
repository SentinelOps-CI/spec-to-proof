//! Cross-service proof-artifact reference records.
//!
//! The farm produces protobuf [`ProofArtifact`](crate::proto::spec_to_proof::v1::ProofArtifact)s,
//! but downstream consumers (notably the GitHub App badge service) work with a
//! smaller, stable, JSON reference shape keyed by *spec document id*. This
//! module defines that reference and the mapping from a completed
//! [`ProofResult`](crate::ProofResult), and provides a directory/S3 sink so the
//! two services integrate through a serialized contract rather than a shared
//! binary dependency.
//!
//! The JSON field names deliberately match the GitHub App's
//! `ProofArtifactReference` so a written record can be consumed verbatim by its
//! `DirectoryProofArtifactSource`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{BoxError, LeanFarmError, ProofResult};

/// Stable, serialization-friendly reference to a proof artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProofArtifactReference {
    pub artifact_id: String,
    pub spec_document_id: String,
    pub content_hash: String,
    pub proof_hash: String,
    #[serde(default)]
    pub rekor_entry_id: String,
    #[serde(default)]
    pub fulcio_certificate: String,
    pub proven_at: Option<DateTime<Utc>>,
    /// One of `proven`, `failed`, `pending`.
    pub status: String,
    #[serde(default)]
    pub error_message: String,
}

impl ProofArtifactReference {
    /// Build a reference from a completed job result.
    ///
    /// The spec-document id is taken from the job option metadata key
    /// `spec_document_id` when present (set by the orchestrator that knows the
    /// document → invariant → theorem chain); otherwise it falls back to the
    /// theorem's `source_invariant_id`, which is always populated.
    pub fn from_result(result: &ProofResult, spec_document_id: Option<&str>) -> Self {
        let spec_document_id = spec_document_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| result.theorem.source_invariant_id.clone());

        Self {
            artifact_id: result.proof_artifact.id.clone(),
            spec_document_id,
            content_hash: result.theorem.content_sha256.clone(),
            proof_hash: result.proof_artifact.content_sha256.clone(),
            rekor_entry_id: String::new(),
            fulcio_certificate: String::new(),
            proven_at: result.success.then(Utc::now),
            status: if result.success {
                "proven".to_string()
            } else {
                "failed".to_string()
            },
            error_message: result.error_message.clone().unwrap_or_default(),
        }
    }

    /// Deterministic object key/file name for this reference.
    pub fn file_name(&self) -> String {
        // spec_document_id can contain characters unsafe for a path segment;
        // sanitise to a conservative set.
        let safe: String = self
            .spec_document_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{safe}.json")
    }

    pub fn to_json(&self) -> Result<Vec<u8>, BoxError> {
        serde_json::to_vec_pretty(self).map_err(|e| {
            LeanFarmError::Storage(format!("serialising artifact reference: {e}")).into()
        })
    }
}

/// Persist a reference to a local mirror directory. Used as the decoupled
/// integration point with the badge service in local/dev deployments and as a
/// durable mirror alongside the S3 artifact bucket in production.
pub async fn write_reference_to_dir(
    dir: &std::path::Path,
    reference: &ProofArtifactReference,
) -> Result<std::path::PathBuf, BoxError> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| {
        LeanFarmError::Storage(format!("creating reference dir {}: {e}", dir.display()))
    })?;
    let path = dir.join(reference.file_name());
    let payload = reference.to_json()?;
    tokio::fs::write(&path, &payload).await.map_err(|e| {
        LeanFarmError::Storage(format!("writing reference {}: {e}", path.display()))
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::spec_to_proof::v1::{LeanTheorem, ProofArtifact, ResourceUsage};

    fn result(success: bool) -> ProofResult {
        ProofResult {
            job_id: "job-1".to_string(),
            theorem: LeanTheorem {
                content_sha256: "theoremhash".to_string(),
                source_invariant_id: "INV-1".to_string(),
                ..Default::default()
            },
            proof_artifact: ProofArtifact {
                id: "proof-1".to_string(),
                content_sha256: "proofhash".to_string(),
                ..Default::default()
            },
            duration_ms: 42,
            success,
            error_message: if success {
                None
            } else {
                Some("boom".to_string())
            },
            resource_usage: ResourceUsage::default(),
        }
    }

    #[test]
    fn maps_successful_result_to_proven() {
        let r = ProofArtifactReference::from_result(&result(true), Some("DOC-9"));
        assert_eq!(r.status, "proven");
        assert_eq!(r.spec_document_id, "DOC-9");
        assert_eq!(r.proof_hash, "proofhash");
        assert!(r.proven_at.is_some());
        assert!(r.error_message.is_empty());
    }

    #[test]
    fn maps_failed_result_to_failed_with_error() {
        let r = ProofArtifactReference::from_result(&result(false), None);
        assert_eq!(r.status, "failed");
        // Falls back to the invariant id when no spec document id is provided.
        assert_eq!(r.spec_document_id, "INV-1");
        assert_eq!(r.error_message, "boom");
        assert!(r.proven_at.is_none());
    }

    #[test]
    fn file_name_is_sanitised() {
        let mut r = ProofArtifactReference::from_result(&result(true), Some("DOC/../9 x"));
        r.spec_document_id = "DOC/../9 x".to_string();
        // '/', '.', '.', '/', ' ' each map to '_'.
        assert_eq!(r.file_name(), "DOC____9_x.json");
    }

    #[test]
    fn json_roundtrips() {
        let r = ProofArtifactReference::from_result(&result(true), Some("DOC-9"));
        let bytes = r.to_json().unwrap();
        let back: ProofArtifactReference = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r, back);
    }
}
