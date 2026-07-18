pub mod config;
pub mod github;
pub mod webhook;
pub mod badge;
pub mod sigstore;
pub mod auth;
pub mod proto;
pub mod error;
pub mod server;
pub mod workflows;
pub mod webhook_handlers;

// Optional AWS cost-governance integrations (Cost Explorer, SES, EventBridge,
// ECS, Lambda). Gated behind the `aws-governance` feature so the default build
// does not pull in the heavyweight AWS SDK crates.
#[cfg(feature = "aws-governance")]
pub mod cost_governance;
#[cfg(feature = "aws-governance")]
pub mod cloudwatch_events;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use axum::{
    routing::{post, get},
    Router,
    http::{HeaderMap, StatusCode},
    Json,
    extract::{State, Path},
};
use tracing::info;
use anyhow::Result;

use crate::config::GitHubAppConfig;
use crate::github::GitHubClient;
use crate::webhook::WebhookProcessor;
use crate::badge::{BadgeManager, DirectoryProofArtifactSource};
use crate::sigstore::SigstoreClient;
use crate::auth::JWTManager;
use crate::proto::gh_app::v1::*;

#[derive(Clone)]
pub struct AppState {
    pub config: GitHubAppConfig,
    pub github_client: Arc<GitHubClient>,
    pub webhook_processor: Arc<WebhookProcessor>,
    pub badge_manager: Arc<BadgeManager>,
    pub sigstore_client: Arc<SigstoreClient>,
    pub jwt_manager: Arc<JWTManager>,
    pub metrics: Arc<RwLock<HashMap<String, u64>>>,
    /// Process start time, used to report real uptime on the health endpoint.
    pub start_time: std::time::Instant,
}

impl AppState {
    pub async fn new(config: GitHubAppConfig) -> Result<Self> {
        let github_client = Arc::new(GitHubClient::new(&config).await?);
        let webhook_processor = Arc::new(WebhookProcessor::new(&config).await?);
        // When a proof-artifact reference directory is configured, back the
        // badge service with the real farm-produced artifacts read from it.
        let mut badge_manager = BadgeManager::new(&config).await?;
        if let Some(dir) = &config.proof_reference_dir {
            info!("Badge service backed by proof reference directory {}", dir);
            badge_manager =
                badge_manager.with_proof_source(Arc::new(DirectoryProofArtifactSource::new(dir)));
        }
        let badge_manager = Arc::new(badge_manager);
        let sigstore_client = Arc::new(SigstoreClient::new(&config).await?);
        let jwt_manager = Arc::new(JWTManager::new(&config).await?);
        let metrics = Arc::new(RwLock::new(HashMap::new()));

        Ok(Self {
            config,
            github_client,
            webhook_processor,
            badge_manager,
            sigstore_client,
            jwt_manager,
            metrics,
            start_time: std::time::Instant::now(),
        })
    }
}

/// Format a duration as a compact, human-readable uptime string (e.g. `1d2h3m4s`).
pub fn format_uptime(elapsed: std::time::Duration) -> String {
    let total_secs = elapsed.as_secs();
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;

    let mut out = String::new();
    if days > 0 {
        out.push_str(&format!("{}d", days));
    }
    if days > 0 || hours > 0 {
        out.push_str(&format!("{}h", hours));
    }
    if days > 0 || hours > 0 || minutes > 0 {
        out.push_str(&format!("{}m", minutes));
    }
    out.push_str(&format!("{}s", seconds));
    out
}

pub async fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/webhook", post(handle_webhook))
        .route("/badge/:repo/:pr", post(update_badge))
        .route("/health", get(health_check))
        .route("/metrics", get(get_metrics))
        .with_state(Arc::new(state))
}

async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<ProcessWebhookResponse>, (StatusCode, String)> {
    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let delivery_id = headers
        .get("X-GitHub-Delivery")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    info!("Received webhook: event={}, delivery_id={}", event_type, delivery_id);

    // Verify webhook signature
    let verification_request = WebhookVerificationRequest {
        payload: body.clone(),
        signature: signature.to_string(),
        webhook_secret: state.config.webhook_secret.clone(),
    };

    let verification_response = state.webhook_processor.verify_webhook(verification_request).await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Webhook verification failed: {}", e)))?;

    if !verification_response.valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid webhook signature".to_string()));
    }

    // Process webhook
    let request = ProcessWebhookRequest {
        payload: body,
        signature: signature.to_string(),
        event_type: event_type.to_string(),
        delivery_id: delivery_id.to_string(),
        installation_id: "".to_string(), // Will be extracted from payload
    };

    let response = state.webhook_processor.process_webhook(request).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Webhook processing failed: {}", e)))?;

    // Update metrics
    {
        let mut metrics = state.metrics.write().await;
        *metrics.entry(format!("webhook_{}", event_type)).or_insert(0) += 1;
        *metrics.entry("webhook_total".to_string()).or_insert(0) += 1;
    }

    Ok(Json(response))
}

async fn update_badge(
    State(state): State<Arc<AppState>>,
    Path((repo, pr)): Path<(String, String)>,
    Json(request): Json<BadgeStatusRequest>,
) -> Result<Json<BadgeStatusResponse>, (StatusCode, String)> {
    info!("Updating badge for repo={}, pr={}", repo, pr);

    let response = state.badge_manager.update_badge_status(request).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Badge update failed: {}", e)))?;

    // Update metrics
    {
        let mut metrics = state.metrics.write().await;
        *metrics.entry(format!("badge_{:?}", response.status)).or_insert(0) += 1;
        *metrics.entry("badge_total".to_string()).or_insert(0) += 1;
    }

    Ok(Json(response))
}

async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthCheckResponse>, (StatusCode, String)> {
    let mut checks: HashMap<String, String> = HashMap::new();

    // The JWT manager health check exercises the real GitHub App auth path
    // (loading the configured RSA key and minting an app JWT), so it is a
    // meaningful readiness signal rather than a hardcoded "ok".
    let jwt_healthy = state.jwt_manager.health_check().await.unwrap_or(false);
    checks.insert(
        "github_app_auth".to_string(),
        if jwt_healthy { "ok".to_string() } else { "degraded".to_string() },
    );

    let status = if jwt_healthy { "healthy" } else { "degraded" };

    let response = HealthCheckResponse {
        status: status.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: format_uptime(state.start_time.elapsed()),
        checks,
    };

    Ok(Json(response))
}

async fn get_metrics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HashMap<String, u64>>, (StatusCode, String)> {
    let metrics = state.metrics.read().await;
    Ok(Json(metrics.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GitHubAppConfig;

    #[tokio::test]
    async fn test_app_state_creation() {
        let config = GitHubAppConfig::default();
        let state = AppState::new(config).await;
        assert!(state.is_ok());
    }

    #[tokio::test]
    async fn test_health_check() {
        let config = GitHubAppConfig::default();
        let state = AppState::new(config).await.unwrap();
        let response = health_check(State(Arc::new(state))).await;
        assert!(response.is_ok());
    }
} 