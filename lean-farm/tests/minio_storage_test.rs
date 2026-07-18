//! MinIO / S3-compatible artifact storage round-trip.
//!
//! Verifies the real object-store path used to persist proof artifacts. Gated
//! behind `E2E_MINIO=1` and requires a reachable S3-compatible endpoint plus
//! credentials. With `docker-compose.e2e.yml` running:
//!
//! ```powershell
//! $env:E2E_MINIO = "1"
//! $env:AWS_ACCESS_KEY_ID = "minioadmin"
//! $env:AWS_SECRET_ACCESS_KEY = "minioadmin"
//! $env:AWS_REGION = "us-east-1"
//! cargo test --test minio_storage_test -- --nocapture
//! ```

use lean_farm::config::{MinioSettings, StorageConfig};
use lean_farm::storage::StorageManager;

fn minio_enabled() -> bool {
    std::env::var("E2E_MINIO")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[tokio::test]
async fn minio_artifact_roundtrip() {
    if !minio_enabled() {
        eprintln!("skipping minio_artifact_roundtrip: set E2E_MINIO=1 to run");
        return;
    }

    let endpoint =
        std::env::var("MINIO_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
    let bucket = std::env::var("MINIO_BUCKET").unwrap_or_else(|_| "proof-artifacts".to_string());

    let config = StorageConfig {
        minio: MinioSettings {
            endpoint,
            bucket,
            region: "us-east-1".to_string(),
            key_prefix: "proof-artifacts".to_string(),
            force_path_style: true,
        },
        ..Default::default()
    };

    let manager = StorageManager::new(&config)
        .await
        .expect("storage manager initialises");

    let key = format!("proof-artifacts/e2e-{}.bin", uuid::Uuid::new_v4());
    let payload = b"lean-farm e2e artifact payload";

    manager
        .upload_to_minio(&key, payload)
        .await
        .expect("upload to MinIO succeeds");
}
