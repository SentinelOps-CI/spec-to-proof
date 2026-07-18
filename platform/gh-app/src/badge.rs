use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::GitHubAppConfig;
use crate::github::GitHubClient;
use crate::proto::gh_app::v1::*;
use crate::sigstore::SigstoreClient;

/// Source of proof artifacts for a set of spec documents.
///
/// Production deployments back this with the proof farm / artifact store; the
/// in-memory implementation below is intended for tests and local development.
/// When no source is configured the badge manager fails loudly rather than
/// fabricating "proven" artifacts.
#[async_trait]
pub trait ProofArtifactSource: Send + Sync + std::fmt::Debug {
    async fn get_artifacts(
        &self,
        spec_document_ids: &[String],
    ) -> Result<Vec<ProofArtifactReference>>;
}

/// In-memory proof-artifact source for tests and local development.
#[derive(Debug, Default, Clone)]
pub struct InMemoryProofArtifacts {
    artifacts: HashMap<String, ProofArtifactReference>,
}

impl InMemoryProofArtifacts {
    pub fn new() -> Self {
        Self { artifacts: HashMap::new() }
    }

    pub fn insert(&mut self, spec_document_id: impl Into<String>, artifact: ProofArtifactReference) {
        self.artifacts.insert(spec_document_id.into(), artifact);
    }
}

#[async_trait]
impl ProofArtifactSource for InMemoryProofArtifacts {
    async fn get_artifacts(
        &self,
        spec_document_ids: &[String],
    ) -> Result<Vec<ProofArtifactReference>> {
        Ok(spec_document_ids
            .iter()
            .filter_map(|id| self.artifacts.get(id).cloned())
            .collect())
    }
}

/// Filesystem-backed proof-artifact source.
///
/// This is the concrete integration point with the Lean proof farm: the farm
/// writes one `ProofArtifactReference` JSON record per completed job into a
/// directory (its `storage.reference_dir`, or a mount synced from the artifact
/// bucket), keyed by spec document id. This source reads those records and
/// serves them to the badge manager, so a badge reflects *real* proof outcomes
/// produced by the farm rather than any fabricated status.
///
/// The JSON schema matches [`ProofArtifactReference`] exactly, which is the
/// same shape the farm serialises, giving a versioned, decoupled contract
/// between the two services without a shared binary dependency.
#[derive(Debug, Clone)]
pub struct DirectoryProofArtifactSource {
    dir: PathBuf,
}

impl DirectoryProofArtifactSource {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Load and parse every `*.json` artifact reference in the directory.
    /// A missing directory yields an empty set (no proofs produced yet);
    /// genuine I/O or parse errors are surfaced so misconfiguration is loud.
    async fn load_all(&self) -> Result<Vec<ProofArtifactReference>> {
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                warn!(
                    "proof reference directory {} does not exist yet; no artifacts",
                    self.dir.display()
                );
                return Ok(Vec::new());
            }
            Err(e) => {
                return Err(anyhow::Error::from(e).context(format!(
                    "reading proof reference directory {}",
                    self.dir.display()
                )))
            }
        };

        let mut references = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("iterating {}", self.dir.display()))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("reading artifact reference {}", path.display()))?;
            let reference: ProofArtifactReference = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing artifact reference {}", path.display()))?;
            references.push(reference);
        }
        Ok(references)
    }
}

#[async_trait]
impl ProofArtifactSource for DirectoryProofArtifactSource {
    async fn get_artifacts(
        &self,
        spec_document_ids: &[String],
    ) -> Result<Vec<ProofArtifactReference>> {
        let wanted: HashSet<&str> = spec_document_ids.iter().map(String::as_str).collect();
        let all = self.load_all().await?;
        Ok(all
            .into_iter()
            .filter(|reference| wanted.contains(reference.spec_document_id.as_str()))
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct BadgeManager {
    config: GitHubAppConfig,
    github_client: GitHubClient,
    sigstore_client: SigstoreClient,
    proof_source: Option<Arc<dyn ProofArtifactSource>>,
    badge_cache: Arc<Mutex<HashMap<String, (BadgeStatusResponse, Instant)>>>,
    /// In-memory history of badge updates keyed by `repo:pr`.
    history: Arc<Mutex<HashMap<String, Vec<BadgeStatusResponse>>>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BadgeStatistics {
    pub total_badges: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub pending_count: u64,
    pub error_count: u64,
    pub average_verification_time_ms: u64,
}

impl BadgeManager {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            github_client: GitHubClient::new(config).await?,
            sigstore_client: SigstoreClient::new(config).await?,
            proof_source: None,
            badge_cache: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Attach a proof-artifact source (proof farm adapter or in-memory store).
    pub fn with_proof_source(mut self, source: Arc<dyn ProofArtifactSource>) -> Self {
        self.proof_source = Some(source);
        self
    }

    pub async fn update_badge_status(&self, request: BadgeStatusRequest) -> Result<BadgeStatusResponse> {
        let cache_key = format!(
            "{}_{}_{}",
            request.repository_id, request.pull_request_id, request.commit_sha
        );

        {
            let cache = self.badge_cache.lock().await;
            if let Some((cached, created_at)) = cache.get(&cache_key) {
                if created_at.elapsed() < Duration::from_secs(300) {
                    return Ok(cached.clone());
                }
            }
        }

        info!(
            "Updating badge status for repo={}, pr={}, commit={}",
            request.repository_id, request.pull_request_id, request.commit_sha
        );

        let repo = self.resolve_repository(&request.repository_id).await?;
        let _pr_number = self.resolve_pull_request(&request.pull_request_id)?;

        let proof_artifacts = self.get_proof_artifacts(&request.spec_document_ids).await?;
        let badge_status = self.determine_badge_status(&proof_artifacts);
        let sigstore_entries = self.get_sigstore_entries(&proof_artifacts).await?;

        let response = BadgeStatusResponse {
            status: badge_status,
            message: self.get_badge_message(badge_status, &proof_artifacts),
            target_url: self.get_badge_target_url(&request, &proof_artifacts),
            description: self.get_badge_description(badge_status, &proof_artifacts),
            context: self.config.badge_context.clone(),
            proof_artifacts,
            sigstore_entries,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };

        self.update_github_status(&repo, &request.commit_sha, &response).await?;

        {
            let mut cache = self.badge_cache.lock().await;
            cache.insert(cache_key, (response.clone(), Instant::now()));
        }
        {
            let history_key = format!("{}:{}", request.repository_id, request.pull_request_id);
            let mut history = self.history.lock().await;
            history.entry(history_key).or_default().push(response.clone());
        }

        info!("Updated badge status: {:?} for {}@{}", badge_status, repo, request.commit_sha);
        Ok(response)
    }

    async fn get_proof_artifacts(
        &self,
        spec_document_ids: &[String],
    ) -> Result<Vec<ProofArtifactReference>> {
        match &self.proof_source {
            Some(source) => source.get_artifacts(spec_document_ids).await,
            None => Err(anyhow::anyhow!(
                "proof artifact source is not configured; refusing to report a badge status \
                 without real proof artifacts (configure a ProofArtifactSource)"
            )),
        }
    }

    fn determine_badge_status(&self, artifacts: &[ProofArtifactReference]) -> BadgeStatus {
        if artifacts.is_empty() {
            return BadgeStatus::BadgeStatusPending;
        }

        let mut all_proven = true;
        let mut any_failed = false;
        for artifact in artifacts {
            match artifact.status.as_str() {
                "proven" => {}
                "failed" => {
                    any_failed = true;
                    all_proven = false;
                }
                "pending" => all_proven = false,
                other => {
                    warn!("Unknown artifact status: {}", other);
                    all_proven = false;
                }
            }
        }

        if any_failed {
            BadgeStatus::BadgeStatusFailure
        } else if all_proven {
            BadgeStatus::BadgeStatusSuccess
        } else {
            BadgeStatus::BadgeStatusPending
        }
    }

    async fn get_sigstore_entries(
        &self,
        artifacts: &[ProofArtifactReference],
    ) -> Result<Vec<SigstoreEntry>> {
        let mut entries = Vec::new();
        for artifact in artifacts {
            if artifact.rekor_entry_id.is_empty() {
                continue;
            }
            let entry = self.sigstore_client.get_entry(&artifact.rekor_entry_id).await?;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn get_badge_message(&self, status: BadgeStatus, artifacts: &[ProofArtifactReference]) -> String {
        match status {
            BadgeStatus::BadgeStatusPending => {
                if artifacts.is_empty() {
                    "No spec documents found".to_string()
                } else {
                    format!("Verifying {} spec document(s)...", artifacts.len())
                }
            }
            BadgeStatus::BadgeStatusSuccess => {
                format!("All {} spec document(s) verified", artifacts.len())
            }
            BadgeStatus::BadgeStatusFailure => {
                let failed = artifacts.iter().filter(|a| a.status == "failed").count();
                format!("{} spec document(s) failed verification", failed)
            }
            BadgeStatus::BadgeStatusError => "Error during verification".to_string(),
            BadgeStatus::BadgeStatusUnspecified => "Unknown status".to_string(),
        }
    }

    fn get_badge_target_url(
        &self,
        _request: &BadgeStatusRequest,
        artifacts: &[ProofArtifactReference],
    ) -> String {
        if artifacts.is_empty() {
            self.config.badge_target_url.clone()
        } else {
            let ids: Vec<String> = artifacts.iter().map(|a| a.artifact_id.clone()).collect();
            format!("{}?artifacts={}", self.config.badge_target_url, ids.join(","))
        }
    }

    fn get_badge_description(&self, status: BadgeStatus, artifacts: &[ProofArtifactReference]) -> String {
        match status {
            BadgeStatus::BadgeStatusPending => "Spec-to-Proof verification in progress".to_string(),
            BadgeStatus::BadgeStatusSuccess => {
                format!("Spec-to-Proof verification passed ({} artifacts)", artifacts.len())
            }
            BadgeStatus::BadgeStatusFailure => {
                let failed = artifacts.iter().filter(|a| a.status == "failed").count();
                format!("Spec-to-Proof verification failed ({} failed)", failed)
            }
            BadgeStatus::BadgeStatusError => "Spec-to-Proof verification error".to_string(),
            BadgeStatus::BadgeStatusUnspecified => "Spec-to-Proof verification".to_string(),
        }
    }

    async fn update_github_status(
        &self,
        repo: &str,
        commit_sha: &str,
        response: &BadgeStatusResponse,
    ) -> Result<()> {
        self.github_client
            .update_commit_status(
                repo,
                commit_sha,
                response.status,
                &response.context,
                &response.description,
                Some(&response.target_url),
            )
            .await
    }

    /// Resolve a repository identifier to an `owner/repo` slug.
    ///
    /// Accepts an `owner/repo` slug directly, or a numeric GitHub repository ID
    /// which is resolved to its full name via the REST API.
    async fn resolve_repository(&self, repository_id: &str) -> Result<String> {
        if repository_id.contains('/') {
            return Ok(repository_id.to_string());
        }
        if repository_id.chars().all(|c| c.is_ascii_digit()) && !repository_id.is_empty() {
            return self
                .github_client
                .get_repository_full_name_by_id(repository_id)
                .await
                .with_context(|| format!("failed to resolve repository id {}", repository_id));
        }
        Err(anyhow::anyhow!(
            "cannot resolve repository identifier '{}': expected 'owner/repo' or a numeric id",
            repository_id
        ))
    }

    /// Resolve a pull-request identifier to a PR number.
    fn resolve_pull_request(&self, pull_request_id: &str) -> Result<String> {
        if pull_request_id.is_empty() {
            return Ok(String::new());
        }
        if pull_request_id.chars().all(|c| c.is_ascii_digit()) {
            return Ok(pull_request_id.to_string());
        }
        Err(anyhow::anyhow!(
            "cannot resolve pull-request identifier '{}': expected a numeric PR number",
            pull_request_id
        ))
    }

    pub async fn get_badge_history(&self, repo: &str, pr: &str) -> Result<Vec<BadgeStatusResponse>> {
        let key = format!("{}:{}", repo, pr);
        Ok(self.history.lock().await.get(&key).cloned().unwrap_or_default())
    }

    pub async fn invalidate_badge_cache(&self, repo: &str, pr: &str, commit_sha: &str) -> Result<()> {
        let cache_key = format!("{}_{}_{}", repo, pr, commit_sha);
        self.badge_cache.lock().await.remove(&cache_key);
        info!("Invalidated badge cache for {}@{}", repo, commit_sha);
        Ok(())
    }

    pub async fn get_badge_statistics(&self) -> Result<BadgeStatistics> {
        let mut stats = BadgeStatistics {
            total_badges: 0,
            success_count: 0,
            failure_count: 0,
            pending_count: 0,
            error_count: 0,
            average_verification_time_ms: 0,
        };

        let cache = self.badge_cache.lock().await;
        for (response, _) in cache.values() {
            stats.total_badges += 1;
            match response.status {
                BadgeStatus::BadgeStatusSuccess => stats.success_count += 1,
                BadgeStatus::BadgeStatusFailure => stats.failure_count += 1,
                BadgeStatus::BadgeStatusPending => stats.pending_count += 1,
                BadgeStatus::BadgeStatusError => stats.error_count += 1,
                BadgeStatus::BadgeStatusUnspecified => {}
            }
        }
        Ok(stats)
    }
}

/// Badge verification service backed by a [`ProofArtifactSource`].
pub struct BadgeVerificationService {
    proof_source: Arc<dyn ProofArtifactSource>,
}

impl BadgeVerificationService {
    pub fn new(proof_source: Arc<dyn ProofArtifactSource>) -> Self {
        Self { proof_source }
    }

    pub async fn verify_spec_documents(
        &self,
        spec_document_ids: Vec<String>,
    ) -> Result<Vec<ProofArtifactReference>> {
        self.proof_source.get_artifacts(&spec_document_ids).await
    }

    pub async fn verify_single_document(
        &self,
        spec_document_id: &str,
    ) -> Result<ProofArtifactReference> {
        self.proof_source
            .get_artifacts(std::slice::from_ref(&spec_document_id.to_string()))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no proof artifact for document {}", spec_document_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proven_artifact(doc: &str) -> ProofArtifactReference {
        ProofArtifactReference {
            artifact_id: "1".to_string(),
            spec_document_id: doc.to_string(),
            content_hash: "sha256:abc".to_string(),
            proof_hash: "sha256:def".to_string(),
            rekor_entry_id: String::new(),
            fulcio_certificate: "cert".to_string(),
            proven_at: Some(Utc::now()),
            status: "proven".to_string(),
            error_message: String::new(),
        }
    }

    #[tokio::test]
    async fn test_badge_manager_creation() {
        assert!(BadgeManager::new(&GitHubAppConfig::default()).await.is_ok());
    }

    #[tokio::test]
    async fn test_badge_status_determination() {
        let manager = BadgeManager::new(&GitHubAppConfig::default()).await.unwrap();
        assert_eq!(manager.determine_badge_status(&[]), BadgeStatus::BadgeStatusPending);
        assert_eq!(
            manager.determine_badge_status(&[proven_artifact("DOC-1")]),
            BadgeStatus::BadgeStatusSuccess
        );

        let mut failed = proven_artifact("DOC-1");
        failed.status = "failed".to_string();
        assert_eq!(
            manager.determine_badge_status(&[failed]),
            BadgeStatus::BadgeStatusFailure
        );
    }

    #[tokio::test]
    async fn test_badge_message_generation() {
        let manager = BadgeManager::new(&GitHubAppConfig::default()).await.unwrap();
        let message =
            manager.get_badge_message(BadgeStatus::BadgeStatusSuccess, &[proven_artifact("DOC-1")]);
        assert!(message.contains("All 1 spec document(s) verified"));
    }

    #[tokio::test]
    async fn test_missing_proof_source_fails_loudly() {
        let manager = BadgeManager::new(&GitHubAppConfig::default()).await.unwrap();
        assert!(manager.get_proof_artifacts(&["DOC-1".to_string()]).await.is_err());
    }

    #[tokio::test]
    async fn test_directory_proof_source_reads_farm_records() {
        // Simulate what the Lean farm writes: one JSON reference per document.
        let dir = std::env::temp_dir().join(format!("gh-app-refs-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let proven = proven_artifact("DOC-1");
        let mut failed = proven_artifact("DOC-2");
        failed.status = "failed".to_string();
        failed.error_message = "unsolved goals".to_string();

        for artifact in [&proven, &failed] {
            let path = dir.join(format!("{}.json", artifact.spec_document_id));
            tokio::fs::write(&path, serde_json::to_vec(artifact).unwrap())
                .await
                .unwrap();
        }
        // A stray non-json file must be ignored.
        tokio::fs::write(dir.join("README.txt"), b"ignore me").await.unwrap();

        let source = DirectoryProofArtifactSource::new(&dir);
        let got = source
            .get_artifacts(&["DOC-1".to_string(), "DOC-2".to_string(), "DOC-404".to_string()])
            .await
            .unwrap();

        assert_eq!(got.len(), 2);
        let manager = BadgeManager::new(&GitHubAppConfig::default())
            .await
            .unwrap()
            .with_proof_source(Arc::new(source));
        // DOC-2 failed, so the overall badge is a failure.
        let artifacts = manager
            .get_proof_artifacts(&["DOC-1".to_string(), "DOC-2".to_string()])
            .await
            .unwrap();
        assert_eq!(manager.determine_badge_status(&artifacts), BadgeStatus::BadgeStatusFailure);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_directory_proof_source_missing_dir_is_empty() {
        let dir = std::env::temp_dir().join(format!("gh-app-missing-{}", uuid::Uuid::new_v4()));
        let source = DirectoryProofArtifactSource::new(&dir);
        let got = source.get_artifacts(&["DOC-1".to_string()]).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn test_in_memory_proof_source() {
        let mut store = InMemoryProofArtifacts::new();
        store.insert("DOC-1", proven_artifact("DOC-1"));
        let manager = BadgeManager::new(&GitHubAppConfig::default())
            .await
            .unwrap()
            .with_proof_source(Arc::new(store));
        let artifacts = manager.get_proof_artifacts(&["DOC-1".to_string()]).await.unwrap();
        assert_eq!(artifacts.len(), 1);
    }

    #[tokio::test]
    async fn test_resolve_repository_slug_passthrough() {
        let manager = BadgeManager::new(&GitHubAppConfig::default()).await.unwrap();
        assert_eq!(manager.resolve_repository("octocat/hello").await.unwrap(), "octocat/hello");
        assert!(manager.resolve_pull_request("42").is_ok());
        assert!(manager.resolve_pull_request("not-a-number").is_err());
    }
}
