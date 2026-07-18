//! Runtime configuration for the Lean Farm.
//!
//! Configuration is expressed as a single [`Config`] tree that can be loaded
//! from a YAML file. Every field has a sensible default so that a missing or
//! partial configuration file still yields a usable configuration (defaults are
//! filled in via `#[serde(default)]`).

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::security::SecurityConfig;
use crate::BoxError;

/// Top-level configuration tree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub security: SecurityConfig,
    pub storage: StorageConfig,
    pub lean: LeanConfig,
    pub sandbox: SandboxConfig,
    pub job: JobConfig,
    pub metrics: MetricsConfig,
}

impl Config {
    /// Load configuration from a YAML file.
    ///
    /// A missing file is treated as "use defaults" rather than an error, which
    /// keeps local/dev usage friction-free. A file that exists but cannot be
    /// parsed is a hard error, because silently ignoring a malformed operator
    /// configuration would be dangerous.
    pub async fn from_file(path: &Path) -> Result<Self, BoxError> {
        match tokio::fs::read_to_string(path).await {
            Ok(contents) => {
                let config: Config = serde_yaml::from_str(&contents)
                    .map_err(|e| format!("failed to parse config {}: {e}", path.display()))?;
                config.validate()?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    "config file {} not found; using built-in defaults",
                    path.display()
                );
                Ok(Config::default())
            }
            Err(e) => Err(format!("failed to read config {}: {e}", path.display()).into()),
        }
    }

    /// Validate cross-field invariants that serde cannot express.
    pub fn validate(&self) -> Result<(), BoxError> {
        if self.job.worker_count == 0 {
            return Err("job.worker_count must be greater than 0".into());
        }
        if self.job.max_queue_size == 0 {
            return Err("job.max_queue_size must be greater than 0".into());
        }
        if self.metrics.port == 0 {
            return Err("metrics.port must be greater than 0".into());
        }
        self.sandbox.validate()?;
        Ok(())
    }
}

/// Object storage configuration. `s3` is the source for read-only code bundles;
/// `minio` (any S3-compatible endpoint) receives proof artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct StorageConfig {
    pub s3: S3Settings,
    pub minio: MinioSettings,
    /// Optional local directory into which the farm mirrors a
    /// [`ProofArtifactReference`](crate::artifact::ProofArtifactReference)
    /// JSON record for each completed job, keyed by spec document id. This is
    /// the decoupled integration contract consumed by the GitHub App badge
    /// service (`DirectoryProofArtifactSource`). When unset, mirroring is
    /// skipped. In production the same records are written to the artifact
    /// bucket under the `references/` prefix.
    pub reference_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Settings {
    pub bucket: String,
    pub region: String,
    pub key_prefix: String,
    /// Optional custom endpoint (e.g. for LocalStack). `None` uses AWS.
    pub endpoint: Option<String>,
}

impl Default for S3Settings {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            region: "us-east-1".to_string(),
            key_prefix: "code-bundles".to_string(),
            endpoint: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MinioSettings {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub key_prefix: String,
    /// MinIO requires path-style addressing.
    pub force_path_style: bool,
}

impl Default for MinioSettings {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            bucket: String::new(),
            region: "us-east-1".to_string(),
            key_prefix: "proof-artifacts".to_string(),
            force_path_style: true,
        }
    }
}

/// Lean toolchain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LeanConfig {
    /// Toolchain version, embedded in generated sources for provenance and used
    /// to derive the container image tag when `image` is left at its default.
    pub lean_version: String,
    /// Fully-qualified container image that provides `lean`/`lake` on `PATH`
    /// with the Lean standard library baked in, so the default proving path
    /// runs fully offline (`--network=none`). Built from
    /// `lean-farm/lean-runner.Dockerfile` (there is no maintained official Lean
    /// image); the tag should track `proof/lean/lean-toolchain`.
    pub image: String,
    /// Import lines injected at the top of every generated proof source.
    ///
    /// Empty by default so generated proofs depend only on the Lean standard
    /// library and can be checked in the stock image without network access.
    /// Set to e.g. `["Mathlib"]` when running against a Mathlib-provisioned
    /// image.
    pub imports: Vec<String>,
    pub default_timeout_seconds: u64,
}

impl Default for LeanConfig {
    fn default() -> Self {
        Self {
            lean_version: "4.22.0".to_string(),
            image: "spec-to-proof/lean-runner:v4.22.0".to_string(),
            imports: Vec::new(),
            default_timeout_seconds: 300,
        }
    }
}

/// Container sandbox configuration for the real Lean execution path.
///
/// These settings map directly onto hardened `docker`/`runsc` (gVisor) run
/// flags applied by [`crate::sandbox::Sandbox`]. Defaults are deliberately
/// restrictive (no network, read-only root, dropped capabilities, non-root
/// user, capped CPU/memory/PIDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Container runtime: `docker` (runc) or `gvisor` (adds `--runtime=runsc`).
    pub runtime: SandboxRuntime,
    /// Docker network mode. `none` is the secure default and works offline for
    /// stdlib-only proofs.
    pub network: String,
    /// Working directory inside the container where the proof source is mounted.
    pub workdir: String,
    /// Maximum number of processes/threads the sandbox may spawn.
    pub pids_limit: u32,
    /// Size of the writable `tmpfs` (mounted at `/tmp` and used as `HOME`) in MiB.
    pub tmpfs_size_mb: u64,
    /// Mount the container root filesystem read-only.
    pub read_only_root: bool,
    /// Extra raw arguments appended to the `docker run` invocation (escape hatch
    /// for site-specific policy, e.g. a custom seccomp profile path).
    pub extra_run_args: Vec<String>,
}

/// Supported container runtimes for the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxRuntime {
    #[default]
    Docker,
    Gvisor,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            runtime: SandboxRuntime::Docker,
            network: "none".to_string(),
            workdir: "/work".to_string(),
            pids_limit: 256,
            tmpfs_size_mb: 512,
            read_only_root: true,
            extra_run_args: Vec::new(),
        }
    }
}

impl SandboxConfig {
    pub fn validate(&self) -> Result<(), BoxError> {
        if self.workdir.trim().is_empty() || !self.workdir.starts_with('/') {
            return Err("sandbox.workdir must be a non-empty absolute path".into());
        }
        if self.pids_limit == 0 {
            return Err("sandbox.pids_limit must be greater than 0".into());
        }
        if self.tmpfs_size_mb == 0 {
            return Err("sandbox.tmpfs_size_mb must be greater than 0".into());
        }
        if self.network.trim().is_empty() {
            return Err("sandbox.network must not be empty (use 'none' for isolation)".into());
        }
        Ok(())
    }
}

/// Worker pool / queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JobConfig {
    pub worker_count: usize,
    pub max_queue_size: usize,
    pub max_job_duration_seconds: u64,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            worker_count: 10,
            max_queue_size: 1000,
            max_job_duration_seconds: 300,
        }
    }
}

/// Metrics/health server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    pub port: u16,
    pub path: String,
    pub health_port: u16,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            port: 9090,
            path: "/metrics".to_string(),
            health_port: 8080,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[tokio::test]
    async fn missing_file_falls_back_to_defaults() {
        let path = std::env::temp_dir().join("lean-farm-nonexistent-config.yaml");
        let _ = tokio::fs::remove_file(&path).await;
        let config = Config::from_file(&path).await.unwrap();
        assert_eq!(config.job.worker_count, JobConfig::default().worker_count);
    }

    #[tokio::test]
    async fn parses_partial_yaml_and_fills_defaults() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("lean-farm-config-{}.yaml", uuid::Uuid::new_v4()));
        tokio::fs::write(
            &path,
            "job:\n  worker_count: 4\n  max_queue_size: 32\nmetrics:\n  port: 7000\n",
        )
        .await
        .unwrap();

        let config = Config::from_file(&path).await.unwrap();
        assert_eq!(config.job.worker_count, 4);
        assert_eq!(config.job.max_queue_size, 32);
        assert_eq!(config.metrics.port, 7000);
        // Untouched fields keep their defaults.
        assert_eq!(config.lean.lean_version, LeanConfig::default().lean_version);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[test]
    fn zero_worker_count_is_rejected() {
        let mut config = Config::default();
        config.job.worker_count = 0;
        assert!(config.validate().is_err());
    }
}
