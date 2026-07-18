use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    Json, Router,
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;
use anyhow::Result;

use crate::config::GitHubAppConfig;
use crate::error::ErrorResponse;
use crate::proto::gh_app::v1::*;
use crate::{create_app, format_uptime, AppState};

pub struct Server {
    config: GitHubAppConfig,
    app: Router,
}

impl Server {
    pub async fn new(config: GitHubAppConfig) -> Result<Self> {
        let state = AppState::new(config.clone()).await?;
        let app = create_app(state).await;

        let rate_limiter = Arc::new(RateLimiter::new(
            config.rate_limit_requests,
            Duration::from_secs(config.rate_limit_window.max(1)),
        ));

        let app = app
            .layer(middleware::from_fn_with_state(rate_limiter, rate_limit_middleware))
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new())
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            );

        Ok(Self { config, app })
    }

    pub async fn run(&self, addr: SocketAddr) -> Result<()> {
        info!("Starting GitHub App server on {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("Listening on {}", addr);

        axum::serve(listener, self.app.clone())
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        info!("Server shut down gracefully");
        Ok(())
    }

    pub async fn run_with_config(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port).parse::<SocketAddr>()?;
        self.run(addr).await
    }
}

/// Resolve when the process receives a shutdown signal (Ctrl+C on all
/// platforms; SIGTERM additionally on Unix).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    info!("Shutdown signal received; draining in-flight requests");
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// A simple fixed-window in-memory rate limiter keyed by client identity
/// (`X-Forwarded-For` when present, else a global bucket). A `max_requests` of
/// zero disables limiting.
pub struct RateLimiter {
    max_requests: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, (u32, Instant)>>,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self { max_requests, window, buckets: Mutex::new(HashMap::new()) }
    }

    /// Record a request for `key`; returns `true` if it is within the limit.
    pub fn check(&self, key: &str) -> bool {
        if self.max_requests == 0 {
            return true;
        }
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate limiter mutex poisoned");
        let entry = buckets.entry(key.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) >= self.window {
            *entry = (0, now);
        }
        if entry.0 < self.max_requests {
            entry.0 += 1;
            true
        } else {
            false
        }
    }
}

pub async fn rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request<Body>,
    next: middleware::Next,
) -> Result<Response, StatusCode> {
    let key = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("global")
        .to_string();

    if limiter.check(&key) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::TOO_MANY_REQUESTS)
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn handle_error(err: axum::BoxError) -> impl IntoResponse {
    let error_response = ErrorResponse::new("INTERNAL_ERROR", &err.to_string(), "INTERNAL_001");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response))
}

pub async fn health_check_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthCheckResponse>, (StatusCode, Json<ErrorResponse>)> {
    let mut checks: HashMap<String, String> = HashMap::new();
    let jwt_healthy = state.jwt_manager.health_check().await.unwrap_or(false);
    checks.insert(
        "github_app_auth".to_string(),
        if jwt_healthy { "ok".to_string() } else { "degraded".to_string() },
    );

    let response = HealthCheckResponse {
        status: if jwt_healthy { "healthy" } else { "degraded" }.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: format_uptime(state.start_time.elapsed()),
        checks,
    };
    Ok(Json(response))
}

pub async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HashMap<String, u64>>, (StatusCode, Json<ErrorResponse>)> {
    let metrics = state.metrics.read().await;
    Ok(Json(metrics.clone()))
}

pub async fn webhook_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<ProcessWebhookResponse>, (StatusCode, Json<ErrorResponse>)> {
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

    let verification_request = WebhookVerificationRequest {
        payload: body.clone(),
        signature: signature.to_string(),
        webhook_secret: state.config.webhook_secret.clone(),
    };

    let verification_response = state
        .webhook_processor
        .verify_webhook(verification_request)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "WEBHOOK_ERROR",
                    &format!("Webhook verification failed: {}", e),
                    "WEBHOOK_001",
                )),
            )
        })?;

    if !verification_response.valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new("WEBHOOK_ERROR", "Invalid webhook signature", "WEBHOOK_001")),
        ));
    }

    let request = ProcessWebhookRequest {
        payload: body,
        signature: signature.to_string(),
        event_type: event_type.to_string(),
        delivery_id: delivery_id.to_string(),
        installation_id: "".to_string(),
    };

    let response = state.webhook_processor.process_webhook(request).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "WEBHOOK_ERROR",
                &format!("Webhook processing failed: {}", e),
                "WEBHOOK_004",
            )),
        )
    })?;

    {
        let mut metrics = state.metrics.write().await;
        *metrics.entry(format!("webhook_{}", event_type)).or_insert(0) += 1;
        *metrics.entry("webhook_total".to_string()).or_insert(0) += 1;
    }

    Ok(Json(response))
}

pub async fn badge_handler(
    State(state): State<Arc<AppState>>,
    Path((repo, pr)): Path<(String, String)>,
    Json(request): Json<BadgeStatusRequest>,
) -> Result<Json<BadgeStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("Updating badge for repo={}, pr={}", repo, pr);

    let response = state.badge_manager.update_badge_status(request).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("BADGE_ERROR", &format!("Badge update failed: {}", e), "BADGE_001")),
        )
    })?;

    {
        let mut metrics = state.metrics.write().await;
        *metrics.entry(format!("badge_{:?}", response.status)).or_insert(0) += 1;
        *metrics.entry("badge_total".to_string()).or_insert(0) += 1;
    }

    Ok(Json(response))
}

pub async fn sigstore_verification_handler(
    State(state): State<Arc<AppState>>,
    Path(entry_id): Path<String>,
) -> Result<Json<SigstoreEntry>, (StatusCode, Json<ErrorResponse>)> {
    info!("Verifying Sigstore entry: {}", entry_id);
    let entry = state.sigstore_client.get_entry(&entry_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "SIGSTORE_ERROR",
                &format!("Failed to get Sigstore entry: {}", e),
                "SIGSTORE_001",
            )),
        )
    })?;
    Ok(Json(entry))
}

/// Retrieve proof artifacts. Backed by the proof-artifact source when
/// configured; otherwise responds `501 Not Implemented` with a typed error
/// rather than returning a misleading empty result.
pub async fn proof_artifacts_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GetProofArtifactsRequest>,
) -> Result<Json<GetProofArtifactsResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!(
        "Getting proof artifacts for repo={}, commit={}",
        request.repository_id, request.commit_sha
    );

    match state.badge_manager.get_badge_history(&request.repository_id, "").await {
        Ok(history) if !history.is_empty() => {
            let artifacts: Vec<ProofArtifactReference> = history
                .into_iter()
                .flat_map(|h| h.proof_artifacts)
                .collect();
            let response = summarize_artifacts(artifacts);
            Ok(Json(response))
        }
        _ => Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse::new(
                "NOT_CONFIGURED",
                "proof artifact retrieval is not configured; attach a ProofArtifactSource / proof-farm backend",
                "PROOF_001",
            )),
        )),
    }
}

fn summarize_artifacts(artifacts: Vec<ProofArtifactReference>) -> GetProofArtifactsResponse {
    let total_count = artifacts.len() as i32;
    let proven_count = artifacts.iter().filter(|a| a.status == "proven").count() as i32;
    let failed_count = artifacts.iter().filter(|a| a.status == "failed").count() as i32;
    let pending_count = artifacts.iter().filter(|a| a.status == "pending").count() as i32;
    GetProofArtifactsResponse { artifacts, total_count, proven_count, failed_count, pending_count }
}

pub async fn logging_middleware(request: Request<Body>, next: middleware::Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();
    info!("{} {} - starting request", method, uri);
    let response = next.run(request).await;
    info!("{} {} - completed in {:?}", method, uri, start.elapsed());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let config = GitHubAppConfig::default();
        assert!(Server::new(config).await.is_ok());
    }

    #[test]
    fn test_rate_limiter_enforces_window() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("k"));
        assert!(limiter.check("k"));
        assert!(!limiter.check("k")); // third request in window is blocked
        assert!(limiter.check("other")); // different key has its own bucket
    }

    #[test]
    fn test_rate_limiter_zero_is_unlimited() {
        let limiter = RateLimiter::new(0, Duration::from_secs(60));
        for _ in 0..1000 {
            assert!(limiter.check("k"));
        }
    }

    #[tokio::test]
    async fn test_health_check_handler() {
        let config = GitHubAppConfig::default();
        let state = AppState::new(config).await.unwrap();
        let response = health_check_handler(State(Arc::new(state))).await;
        assert!(response.is_ok());
    }
}
