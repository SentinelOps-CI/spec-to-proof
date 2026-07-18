use std::collections::HashMap;
use std::time::Instant;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use anyhow::{Result, Context};
use tracing::{info, warn};

use crate::config::GitHubAppConfig;
use crate::proto::gh_app::v1::*;

pub struct WebhookProcessor {
    config: GitHubAppConfig,
    event_handlers: HashMap<String, Box<dyn EventHandler + Send + Sync>>,
    #[allow(dead_code)]
    signature_cache: HashMap<String, Instant>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub action: Option<String>,
    pub pull_request: Option<PullRequestPayload>,
    pub repository: Option<RepositoryPayload>,
    pub sender: Option<UserPayload>,
    pub installation: Option<InstallationPayload>,
    pub commits: Option<Vec<CommitPayload>>,
    pub head_commit: Option<CommitPayload>,
    pub ref_field: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub created: Option<bool>,
    pub deleted: Option<bool>,
    pub forced: Option<bool>,
    pub base_ref: Option<String>,
    pub compare: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequestPayload {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub draft: bool,
    pub merged: bool,
    pub mergeable: Option<bool>,
    pub mergeable_state: String,
    pub head: CommitRefPayload,
    pub base: CommitRefPayload,
    pub user: UserPayload,
    pub assignees: Vec<UserPayload>,
    pub requested_reviewers: Vec<UserPayload>,
    pub labels: Vec<LabelPayload>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepositoryPayload {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub owner: UserPayload,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserPayload {
    pub id: u64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub user_type: String,
    pub site_admin: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationPayload {
    pub id: u64,
    pub account: UserPayload,
    pub repository_selection: String,
    pub permissions: HashMap<String, String>,
    pub events: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitPayload {
    pub id: String,
    pub tree_id: String,
    pub message: String,
    pub timestamp: String,
    pub author: CommitAuthorPayload,
    pub committer: CommitAuthorPayload,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitAuthorPayload {
    pub name: String,
    pub email: String,
    pub username: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitRefPayload {
    pub label: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    pub user: UserPayload,
    pub repo: RepositoryPayload,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LabelPayload {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub color: String,
    pub default: bool,
}

#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, payload: &WebhookPayload) -> Result<ProcessWebhookResponse>;
}

impl WebhookProcessor {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        let mut processor = Self {
            config: config.clone(),
            event_handlers: HashMap::new(),
            signature_cache: HashMap::new(),
        };
        
        // Register event handlers
        processor.register_event_handlers().await?;
        
        Ok(processor)
    }
    
    async fn register_event_handlers(&mut self) -> Result<()> {
        // Register pull request handler
        self.event_handlers.insert(
            "pull_request".to_string(),
            Box::new(PullRequestHandler::new(&self.config).await?),
        );
        
        // Register push handler
        self.event_handlers.insert(
            "push".to_string(),
            Box::new(PushHandler::new(&self.config).await?),
        );
        
        // Register status handler
        self.event_handlers.insert(
            "status".to_string(),
            Box::new(StatusHandler::new(&self.config).await?),
        );
        
        info!("Registered {} event handlers", self.event_handlers.len());
        Ok(())
    }
    
    pub async fn verify_webhook(&self, request: WebhookVerificationRequest) -> Result<WebhookVerificationResponse> {
        let signature = request.signature;
        let payload = request.payload;
        let webhook_secret = request.webhook_secret;
        
        // Check signature format
        if !signature.starts_with("sha256=") {
            return Ok(WebhookVerificationResponse {
                valid: false,
                error_message: "Invalid signature format".to_string(),
                event_type: "".to_string(),
                delivery_id: "".to_string(),
            });
        }
        
        // Extract signature hash
        let signature_hash = &signature[7..];
        
        // Calculate expected signature
        let mut mac = Hmac::<Sha256>::new_from_slice(webhook_secret.as_bytes())
            .context("Failed to create HMAC")?;
        mac.update(payload.as_bytes());
        let expected_hash = hex::encode(mac.finalize().into_bytes());
        
        // Compare signatures
        let valid = signature_hash == expected_hash;
        
        // Extract event type from payload if valid
        let event_type = if valid {
            if let Ok(payload_json) = serde_json::from_str::<Value>(&payload) {
                payload_json.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                "unknown".to_string()
            }
        } else {
            "unknown".to_string()
        };
        
        Ok(WebhookVerificationResponse {
            valid,
            error_message: if valid { "".to_string() } else { "Invalid signature".to_string() },
            event_type,
            delivery_id: "".to_string(), // Will be extracted from headers
        })
    }
    
    pub async fn process_webhook(&self, request: ProcessWebhookRequest) -> Result<ProcessWebhookResponse> {
        let payload = request.payload;
        let event_type = request.event_type;
        let delivery_id = request.delivery_id;
        
        info!("Processing webhook: event={}, delivery_id={}", event_type, delivery_id);
        
        // Parse payload
        let webhook_payload: WebhookPayload = serde_json::from_str(&payload)
            .context("Failed to parse webhook payload")?;
        
        // Find handler for event type
        let handler = self.event_handlers.get(&event_type)
            .ok_or_else(|| anyhow::anyhow!("No handler found for event type: {}", event_type))?;
        
        // Process event
        let response = handler.handle(&webhook_payload).await?;
        
        info!("Processed webhook successfully: event={}, delivery_id={}", event_type, delivery_id);
        
        Ok(response)
    }
    
    pub async fn extract_spec_documents(&self, payload: &WebhookPayload) -> Result<Vec<String>> {
        let mut spec_documents = Vec::new();
        
        // Extract from pull request body
        if let Some(pr) = &payload.pull_request {
            if let Some(body) = &pr.body {
                spec_documents.extend(self.extract_spec_references(body));
            }
        }
        
        // Extract from commit messages
        if let Some(commits) = &payload.commits {
            for commit in commits {
                spec_documents.extend(self.extract_spec_references(&commit.message));
            }
        }
        
        // Extract from head commit
        if let Some(head_commit) = &payload.head_commit {
            spec_documents.extend(self.extract_spec_references(&head_commit.message));
        }
        
        Ok(spec_documents)
    }
    
    fn extract_spec_references(&self, text: &str) -> Vec<String> {
        let mut references = Vec::new();
        
        // Look for spec document references in various formats
        let patterns = vec![
            r"spec:\s*([a-zA-Z0-9_-]+)",           // spec: DOC-123
            r"document:\s*([a-zA-Z0-9_-]+)",       // document: DOC-123
            r"#([a-zA-Z0-9_-]+)",                  // #DOC-123
            r"DOC-([0-9]+)",                       // DOC-123
            r"SPEC-([0-9]+)",                      // SPEC-123
        ];
        
        for pattern in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for cap in regex.captures_iter(text) {
                    if let Some(reference) = cap.get(1) {
                        references.push(reference.as_str().to_string());
                    }
                }
            }
        }
        
        references
    }
}

// Pull Request Event Handler
pub struct PullRequestHandler {
    config: GitHubAppConfig,
}

impl PullRequestHandler {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl EventHandler for PullRequestHandler {
    async fn handle(&self, payload: &WebhookPayload) -> Result<ProcessWebhookResponse> {
        info!("Handling pull request event");
        
        let mut badge_updates = Vec::new();
        let mut processed_events = Vec::new();
        
        if payload.pull_request.is_some() {
            if let Some(action) = &payload.action {
                match action.as_str() {
                    "opened" | "synchronize" | "reopened" => {
                        // Extract spec documents
                        let spec_documents = self.extract_spec_documents(payload).await?;
                        
                        if !spec_documents.is_empty() {
                            // Create the pending badge response that the caller
                            // (server/lib layer) persists via the BadgeManager.
                            let badge_response = BadgeStatusResponse {
                                status: BadgeStatus::BadgeStatusPending,
                                message: "Verifying spec documents...".to_string(),
                                target_url: self.config.badge_target_url.clone(),
                                description: self.config.badge_description.clone(),
                                context: self.config.badge_context.clone(),
                                proof_artifacts: Vec::new(),
                                sigstore_entries: Vec::new(),
                                created_at: Some(chrono::Utc::now()),
                                updated_at: Some(chrono::Utc::now()),
                            };
                            
                            badge_updates.push(badge_response);
                        }
                        
                        processed_events.push(format!("pull_request_{}", action));
                    }
                    "closed" | "merged" => {
                        processed_events.push(format!("pull_request_{}", action));
                    }
                    _ => {
                        warn!("Unhandled pull request action: {}", action);
                    }
                }
            }
        }
        
        Ok(ProcessWebhookResponse {
            success: true,
            message: "Pull request event processed successfully".to_string(),
            badge_updates,
            processed_events,
        })
    }
}

// Push Event Handler
pub struct PushHandler {
    config: GitHubAppConfig,
}

impl PushHandler {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl EventHandler for PushHandler {
    async fn handle(&self, payload: &WebhookPayload) -> Result<ProcessWebhookResponse> {
        info!("Handling push event");
        
        let mut badge_updates = Vec::new();
        let mut processed_events = Vec::new();
        
        // Extract spec documents from commit messages
        let spec_documents = self.extract_spec_documents(payload).await?;
        
        if !spec_documents.is_empty() {
            // Create badge status request for the head commit
            if let Some(head_commit) = &payload.head_commit {
                let _ = &head_commit.id;
                // Create the pending badge response for the head commit.
                let badge_response = BadgeStatusResponse {
                    status: BadgeStatus::BadgeStatusPending,
                    message: "Verifying spec documents...".to_string(),
                    target_url: self.config.badge_target_url.clone(),
                    description: self.config.badge_description.clone(),
                    context: self.config.badge_context.clone(),
                    proof_artifacts: Vec::new(),
                    sigstore_entries: Vec::new(),
                    created_at: Some(chrono::Utc::now()),
                    updated_at: Some(chrono::Utc::now()),
                };
                
                badge_updates.push(badge_response);
            }
        }
        
        processed_events.push("push".to_string());
        
        Ok(ProcessWebhookResponse {
            success: true,
            message: "Push event processed successfully".to_string(),
            badge_updates,
            processed_events,
        })
    }
}

// Status Event Handler
pub struct StatusHandler {
    #[allow(dead_code)]
    config: GitHubAppConfig,
}

impl StatusHandler {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl EventHandler for StatusHandler {
    async fn handle(&self, _payload: &WebhookPayload) -> Result<ProcessWebhookResponse> {
        info!("Handling status event");
        
        let processed_events = vec!["status".to_string()];

        Ok(ProcessWebhookResponse {
            success: true,
            message: "Status event processed successfully".to_string(),
            badge_updates: Vec::new(),
            processed_events,
        })
    }
}

// Helper trait for extracting spec documents
#[async_trait::async_trait]
trait SpecDocumentExtractor {
    async fn extract_spec_documents(&self, payload: &WebhookPayload) -> Result<Vec<String>>;
}

#[async_trait::async_trait]
impl SpecDocumentExtractor for PullRequestHandler {
    async fn extract_spec_documents(&self, payload: &WebhookPayload) -> Result<Vec<String>> {
        let mut spec_documents = Vec::new();
        
        // Extract from pull request body
        if let Some(pr) = &payload.pull_request {
            if let Some(body) = &pr.body {
                spec_documents.extend(self.extract_spec_references(body));
            }
        }
        
        Ok(spec_documents)
    }
}

#[async_trait::async_trait]
impl SpecDocumentExtractor for PushHandler {
    async fn extract_spec_documents(&self, payload: &WebhookPayload) -> Result<Vec<String>> {
        let mut spec_documents = Vec::new();
        
        // Extract from commit messages
        if let Some(commits) = &payload.commits {
            for commit in commits {
                spec_documents.extend(self.extract_spec_references(&commit.message));
            }
        }
        
        // Extract from head commit
        if let Some(head_commit) = &payload.head_commit {
            spec_documents.extend(self.extract_spec_references(&head_commit.message));
        }
        
        Ok(spec_documents)
    }
}

#[async_trait::async_trait]
impl SpecDocumentExtractor for StatusHandler {
    async fn extract_spec_documents(&self, _payload: &WebhookPayload) -> Result<Vec<String>> {
        Ok(Vec::new()) // Status events don't contain spec documents
    }
}

// Helper trait for extracting spec references
trait SpecReferenceExtractor {
    fn extract_spec_references(&self, text: &str) -> Vec<String>;
}

impl SpecReferenceExtractor for PullRequestHandler {
    fn extract_spec_references(&self, text: &str) -> Vec<String> {
        let mut references = Vec::new();
        
        // Look for spec document references in various formats
        let patterns = vec![
            r"spec:\s*([a-zA-Z0-9_-]+)",           // spec: DOC-123
            r"document:\s*([a-zA-Z0-9_-]+)",       // document: DOC-123
            r"#([a-zA-Z0-9_-]+)",                  // #DOC-123
            r"DOC-([0-9]+)",                       // DOC-123
            r"SPEC-([0-9]+)",                      // SPEC-123
        ];
        
        for pattern in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for cap in regex.captures_iter(text) {
                    if let Some(reference) = cap.get(1) {
                        references.push(reference.as_str().to_string());
                    }
                }
            }
        }
        
        references
    }
}

impl SpecReferenceExtractor for PushHandler {
    fn extract_spec_references(&self, text: &str) -> Vec<String> {
        let mut references = Vec::new();
        
        // Look for spec document references in various formats
        let patterns = vec![
            r"spec:\s*([a-zA-Z0-9_-]+)",           // spec: DOC-123
            r"document:\s*([a-zA-Z0-9_-]+)",       // document: DOC-123
            r"#([a-zA-Z0-9_-]+)",                  // #DOC-123
            r"DOC-([0-9]+)",                       // DOC-123
            r"SPEC-([0-9]+)",                      // SPEC-123
        ];
        
        for pattern in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for cap in regex.captures_iter(text) {
                    if let Some(reference) = cap.get(1) {
                        references.push(reference.as_str().to_string());
                    }
                }
            }
        }
        
        references
    }
}

impl SpecReferenceExtractor for StatusHandler {
    fn extract_spec_references(&self, _text: &str) -> Vec<String> {
        Vec::new() // Status events don't contain spec references
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_webhook_processor_creation() {
        let config = GitHubAppConfig::default();
        let processor = WebhookProcessor::new(&config).await;
        assert!(processor.is_ok());
    }
    
    #[test]
    fn test_spec_reference_extraction() {
        let handler = PullRequestHandler {
            config: GitHubAppConfig::default(),
        };
        
        let text = "This PR addresses spec: DOC-123 and document: SPEC-456. Also see #DOC-789.";
        let references = handler.extract_spec_references(text);
        
        assert!(references.contains(&"DOC-123".to_string()));
        assert!(references.contains(&"SPEC-456".to_string()));
        assert!(references.contains(&"DOC-789".to_string()));
    }
    
    #[test]
    fn test_webhook_signature_verification() {
        let config = GitHubAppConfig::default();
        let processor = WebhookProcessor {
            config,
            event_handlers: HashMap::new(),
            signature_cache: HashMap::new(),
        };
        
        // Test with valid signature
        let payload = r#"{"action":"opened","pull_request":{"id":123}}"#;
        let secret = "test_secret";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload.as_bytes());
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        
        let request = WebhookVerificationRequest {
            payload: payload.to_string(),
            signature,
            webhook_secret: secret.to_string(),
        };
        
        let response = tokio::runtime::Runtime::new().unwrap().block_on(
            processor.verify_webhook(request)
        ).unwrap();
        
        assert!(response.valid);
    }
} 