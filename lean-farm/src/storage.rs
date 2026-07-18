//! Object-storage integration.
//!
//! Two logical buckets are used: an S3 source bucket holding read-only code
//! bundles, and an S3-compatible (e.g. MinIO) bucket that receives proof
//! artifacts and job-result summaries. Both are accessed with the AWS SDK; the
//! MinIO client simply targets a custom endpoint with path-style addressing.
//!
//! When a bucket is not configured, operations fail fast with
//! [`LeanFarmError::NotConfigured`] rather than panicking or silently
//! succeeding, so a misconfigured deployment is obvious.

use std::path::Path;

use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

use crate::config::StorageConfig;
use crate::{BoxError, LeanFarmError, ProofResult};

#[derive(Clone, Debug)]
pub struct StorageManager {
    s3_client: Client,
    minio_client: Client,
    config: StorageConfig,
}

impl StorageManager {
    pub async fn new(config: &StorageConfig) -> Result<Self, BoxError> {
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;

        // Source S3 client (optionally pointed at a custom endpoint).
        let mut s3_builder = aws_sdk_s3::config::Builder::from(&shared)
            .region(Region::new(config.s3.region.clone()));
        if let Some(endpoint) = &config.s3.endpoint {
            s3_builder = s3_builder.endpoint_url(endpoint.clone());
        }
        let s3_client = Client::from_conf(s3_builder.build());

        // MinIO / S3-compatible client for artifact writes.
        let mut minio_builder = aws_sdk_s3::config::Builder::from(&shared)
            .region(Region::new(config.minio.region.clone()))
            .force_path_style(config.minio.force_path_style);
        if !config.minio.endpoint.is_empty() {
            minio_builder = minio_builder.endpoint_url(config.minio.endpoint.clone());
        }
        let minio_client = Client::from_conf(minio_builder.build());

        Ok(Self {
            s3_client,
            minio_client,
            config: config.clone(),
        })
    }

    /// Download an object from the source S3 bucket to a local path.
    pub async fn download_from_s3(&self, key: &str, local_path: &Path) -> Result<(), BoxError> {
        if self.config.s3.bucket.is_empty() {
            return Err(LeanFarmError::NotConfigured(
                "storage.s3.bucket is not set; cannot download code bundle".to_string(),
            )
            .into());
        }

        let output = self
            .s3_client
            .get_object()
            .bucket(&self.config.s3.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| LeanFarmError::Storage(format!("s3 get_object {key} failed: {e}")))?;

        let bytes = output
            .body
            .collect()
            .await
            .map_err(|e| LeanFarmError::Storage(format!("reading s3 body for {key}: {e}")))?
            .into_bytes();

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| LeanFarmError::Storage(format!("creating {}: {e}", parent.display())))?;
        }
        tokio::fs::write(local_path, &bytes)
            .await
            .map_err(|e| LeanFarmError::Storage(format!("writing {}: {e}", local_path.display())))?;

        Ok(())
    }

    /// Upload bytes to the MinIO/S3-compatible artifact bucket.
    pub async fn upload_to_minio(&self, key: &str, data: &[u8]) -> Result<(), BoxError> {
        if self.config.minio.bucket.is_empty() {
            return Err(LeanFarmError::NotConfigured(
                "storage.minio.bucket is not set; cannot upload artifact".to_string(),
            )
            .into());
        }

        self.minio_client
            .put_object()
            .bucket(&self.config.minio.bucket)
            .key(key)
            .body(ByteStream::from(data.to_vec()))
            .send()
            .await
            .map_err(|e| LeanFarmError::Storage(format!("minio put_object {key} failed: {e}")))?;

        Ok(())
    }

    /// Persist a compact JSON summary of a job result. When no artifact bucket
    /// is configured this is a no-op (logged), so result handling never fails
    /// solely because persistence is disabled.
    pub async fn store_job_result(&self, result: &ProofResult) -> Result<(), BoxError> {
        let summary = serde_json::json!({
            "job_id": result.job_id,
            "theorem_id": result.theorem.id,
            "proof_artifact_id": result.proof_artifact.id,
            "success": result.success,
            "duration_ms": result.duration_ms,
            "error_message": result.error_message,
        });
        let payload = serde_json::to_vec(&summary)?;

        if self.config.minio.bucket.is_empty() {
            tracing::info!(
                "no artifact bucket configured; skipping persistence of job {} ({} bytes)",
                result.job_id,
                payload.len()
            );
            return Ok(());
        }

        let key = format!("{}/results/{}.json", self.config.minio.key_prefix, result.job_id);
        self.upload_to_minio(&key, &payload).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::spec_to_proof::v1::{LeanTheorem, ProofArtifact, ResourceUsage};

    async fn manager() -> StorageManager {
        StorageManager::new(&StorageConfig::default()).await.unwrap()
    }

    fn dummy_result() -> ProofResult {
        ProofResult {
            job_id: "job-1".to_string(),
            theorem: LeanTheorem::default(),
            proof_artifact: ProofArtifact::default(),
            duration_ms: 10,
            success: true,
            error_message: None,
            resource_usage: ResourceUsage::default(),
        }
    }

    #[tokio::test]
    async fn download_without_bucket_is_not_configured() {
        let mgr = manager().await;
        let path = std::env::temp_dir().join("lean-farm-download-test");
        let err = mgr.download_from_s3("k", &path).await.unwrap_err();
        assert!(err.to_string().contains("not configured") || err.to_string().contains("not set"));
    }

    #[tokio::test]
    async fn upload_without_bucket_is_not_configured() {
        let mgr = manager().await;
        let err = mgr.upload_to_minio("k", b"data").await.unwrap_err();
        assert!(err.to_string().contains("not configured") || err.to_string().contains("not set"));
    }

    #[tokio::test]
    async fn store_result_without_bucket_is_noop() {
        let mgr = manager().await;
        // No bucket configured -> Ok without touching the network.
        mgr.store_job_result(&dummy_result()).await.unwrap();
    }
}
