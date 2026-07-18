use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use reqwest::{Client, header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT, ACCEPT}};
use serde::{Deserialize, Serialize};
use jsonwebtoken::{encode, Header, EncodingKey, Algorithm};
use chrono::{Utc, Duration as ChronoDuration};
use anyhow::{Result, Context};
use tokio::sync::Mutex;
use tracing::info;

use crate::config::GitHubAppConfig;
use crate::proto::gh_app::v1::*;

/// Thin GitHub REST client. Caches are behind `Arc<Mutex<..>>` so the client is
/// cheaply cloneable and shareable behind an `Arc` while remaining `&self`.
#[derive(Debug, Clone)]
pub struct GitHubClient {
    config: GitHubAppConfig,
    http_client: Client,
    jwt_cache: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    installation_token_cache: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JWTPayload {
    iss: String,  // App ID
    iat: i64,     // Issued at
    exp: i64,     // Expiration time
}

#[derive(Debug, Serialize, Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
    permissions: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StatusRequest {
    state: String,
    target_url: Option<String>,
    description: Option<String>,
    context: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequest {
    id: String,
    number: String,
    title: String,
    state: String,
    head: CommitRef,
    base: CommitRef,
    user: User,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CommitRef {
    sha: String,
    ref_field: String,
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: String,
    login: String,
    #[serde(rename = "type")]
    user_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Repository {
    id: String,
    name: String,
    full_name: String,
    owner: User,
    private: bool,
    default_branch: String,
}

impl GitHubClient {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("spec-to-proof-gh-app"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github.v3+json"));
        
        let http_client = Client::builder()
            .timeout(Duration::from_secs(config.request_timeout))
            .default_headers(headers)
            .build()
            .context("Failed to create HTTP client")?;
        
        Ok(Self {
            config: config.clone(),
            http_client,
            jwt_cache: Arc::new(Mutex::new(HashMap::new())),
            installation_token_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    
    pub async fn create_jwt(&self) -> Result<String> {
        {
            let cache = self.jwt_cache.lock().await;
            if let Some((token, created_at)) = cache.get("jwt") {
                // App JWTs are valid for 10 minutes; serve cached until 9.
                if created_at.elapsed() < Duration::from_secs(540) {
                    return Ok(token.clone());
                }
            }
        }

        if self.config.private_key.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "GitHub App private key is not configured; cannot mint JWT"
            ));
        }

        let now = Utc::now();
        let exp = now + ChronoDuration::minutes(10); // JWT expires in 10 minutes
        
        let payload = JWTPayload {
            iss: self.config.app_id.clone(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
        };
        
        let token = encode(
            &Header::new(Algorithm::RS256),
            &payload,
            &EncodingKey::from_rsa_pem(self.config.private_key.as_bytes())
                .context("Failed to parse RSA private key")?
        )?;
        
        self.jwt_cache.lock().await.insert("jwt".to_string(), (token.clone(), Instant::now()));
        Ok(token)
    }
    
    pub async fn get_installation_token(&self, installation_id: &str) -> Result<String> {
        {
            let cache = self.installation_token_cache.lock().await;
            if let Some((token, created_at)) = cache.get(installation_id) {
                if created_at.elapsed() < Duration::from_secs(3300) {
                    return Ok(token.clone());
                }
            }
        }
        
        // Get JWT for authentication
        let jwt = self.create_jwt().await?;
        
        let url = format!("{}/app/installations/{}/access_tokens", 
            self.config.base_url, installation_id);
        
        let response = self.http_client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", jwt))
            .send()
            .await
            .context("Failed to get installation token")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get installation token: {}", error_text));
        }
        
        let token_response: InstallationTokenResponse = response.json().await
            .context("Failed to parse installation token response")?;
        
        self.installation_token_cache.lock().await.insert(
            installation_id.to_string(), 
            (token_response.token.clone(), Instant::now())
        );
        
        Ok(token_response.token)
    }
    
    pub async fn update_commit_status(
        &self,
        repo: &str,
        sha: &str,
        status: BadgeStatus,
        context: &str,
        description: &str,
        target_url: Option<&str>,
    ) -> Result<()> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let state = match status {
            BadgeStatus::BadgeStatusPending => "pending",
            BadgeStatus::BadgeStatusSuccess => "success",
            BadgeStatus::BadgeStatusFailure => "failure",
            BadgeStatus::BadgeStatusError => "error",
            _ => "error",
        };
        
        let status_request = StatusRequest {
            state: state.to_string(),
            target_url: target_url.map(|s| s.to_string()),
            description: Some(description.to_string()),
            context: context.to_string(),
        };
        
        let url = format!("{}/repos/{}/statuses/{}", 
            self.config.base_url, repo, sha);
        
        let response = self.http_client
            .post(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .json(&status_request)
            .send()
            .await
            .context("Failed to update commit status")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to update commit status: {}", error_text));
        }
        
        info!("Updated commit status for {}@{}: {:?}", repo, sha, status);
        Ok(())
    }
    
    pub async fn get_pull_request(&self, repo: &str, pr_number: &str) -> Result<PullRequest> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let url = format!("{}/repos/{}/pulls/{}", 
            self.config.base_url, repo, pr_number);
        
        let response = self.http_client
            .get(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .send()
            .await
            .context("Failed to get pull request")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get pull request: {}", error_text));
        }
        
        let pr: PullRequest = response.json().await
            .context("Failed to parse pull request response")?;
        
        Ok(pr)
    }
    
    pub async fn get_repository(&self, repo: &str) -> Result<Repository> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let url = format!("{}/repos/{}", self.config.base_url, repo);
        
        let response = self.http_client
            .get(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .send()
            .await
            .context("Failed to get repository")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get repository: {}", error_text));
        }
        
        let repository: Repository = response.json().await
            .context("Failed to parse repository response")?;
        
        Ok(repository)
    }
    
    /// Resolve a numeric GitHub repository ID to its `owner/repo` full name
    /// via `GET /repositories/{id}`.
    pub async fn get_repository_full_name_by_id(&self, repository_id: &str) -> Result<String> {
        let token = self.get_installation_token(&self.config.installation_id).await?;
        let url = format!("{}/repositories/{}", self.config.base_url, repository_id);

        let response = self.http_client
            .get(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .send()
            .await
            .context("Failed to resolve repository by id")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to resolve repository by id: {}", error_text));
        }

        let value: serde_json::Value = response.json().await
            .context("Failed to parse repository response")?;
        value.get("full_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("repository {} has no full_name", repository_id))
    }

    pub async fn get_commit(&self, repo: &str, sha: &str) -> Result<serde_json::Value> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let url = format!("{}/repos/{}/commits/{}", 
            self.config.base_url, repo, sha);
        
        let response = self.http_client
            .get(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .send()
            .await
            .context("Failed to get commit")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get commit: {}", error_text));
        }
        
        let commit: serde_json::Value = response.json().await
            .context("Failed to parse commit response")?;
        
        Ok(commit)
    }
    
    pub async fn get_changed_files(&self, repo: &str, pr_number: &str) -> Result<Vec<String>> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let url = format!("{}/repos/{}/pulls/{}/files", 
            self.config.base_url, repo, pr_number);
        
        let response = self.http_client
            .get(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .send()
            .await
            .context("Failed to get changed files")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get changed files: {}", error_text));
        }
        
        let files: Vec<serde_json::Value> = response.json().await
            .context("Failed to parse changed files response")?;
        
        let file_names: Vec<String> = files
            .into_iter()
            .filter_map(|file| file.get("filename").and_then(|f| f.as_str()).map(|s| s.to_string()))
            .collect();
        
        Ok(file_names)
    }
    
    pub async fn create_check_run(
        &self,
        repo: &str,
        name: &str,
        head_sha: &str,
        status: &str,
        conclusion: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<()> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let mut check_run = serde_json::json!({
            "name": name,
            "head_sha": head_sha,
            "status": status,
        });
        
        if let Some(conclusion) = conclusion {
            check_run["conclusion"] = serde_json::Value::String(conclusion.to_string());
        }
        
        if let Some(output) = output {
            check_run["output"] = output;
        }
        
        let url = format!("{}/repos/{}/check-runs", 
            self.config.base_url, repo);
        
        let response = self.http_client
            .post(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .json(&check_run)
            .send()
            .await
            .context("Failed to create check run")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to create check run: {}", error_text));
        }
        
        info!("Created check run {} for {}@{}", name, repo, head_sha);
        Ok(())
    }
    
    pub async fn update_check_run(
        &self,
        repo: &str,
        check_run_id: &str,
        status: &str,
        conclusion: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<()> {
        let installation_id = &self.config.installation_id;
        let token = self.get_installation_token(installation_id).await?;
        
        let mut check_run = serde_json::json!({
            "status": status,
        });
        
        if let Some(conclusion) = conclusion {
            check_run["conclusion"] = serde_json::Value::String(conclusion.to_string());
        }
        
        if let Some(output) = output {
            check_run["output"] = output;
        }
        
        let url = format!("{}/repos/{}/check-runs/{}", 
            self.config.base_url, repo, check_run_id);
        
        let response = self.http_client
            .patch(&url)
            .header(AUTHORIZATION, format!("token {}", token))
            .json(&check_run)
            .send()
            .await
            .context("Failed to update check run")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to update check run: {}", error_text));
        }
        
        info!("Updated check run {} for {}", check_run_id, repo);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_github_client_creation() {
        let config = GitHubAppConfig::default();
        let client = GitHubClient::new(&config).await;
        assert!(client.is_ok());
    }
    
    #[test]
    fn test_jwt_payload_serialization() {
        let payload = JWTPayload {
            iss: "12345".to_string(),
            iat: 1234567890,
            exp: 1234567890 + 600, // 10 minutes later
        };
        
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: JWTPayload = serde_json::from_str(&json).unwrap();
        
        assert_eq!(payload.iss, deserialized.iss);
        assert_eq!(payload.iat, deserialized.iat);
        assert_eq!(payload.exp, deserialized.exp);
    }
} 