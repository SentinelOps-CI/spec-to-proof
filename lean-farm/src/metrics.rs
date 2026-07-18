//! Lightweight, dependency-free metrics registry plus a Prometheus text
//! exposition endpoint.
//!
//! A single [`Metrics`] instance is shared (via `Arc`) between the
//! [`MetricsServer`] that exposes it over HTTP and the job runner that updates
//! it. Counters use relaxed atomics because they are monotonic and read for
//! reporting only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::get, Router};

use crate::BoxError;

/// Process-wide counters describing job throughput and latency.
#[derive(Debug, Default)]
pub struct Metrics {
    jobs_total: AtomicU64,
    jobs_succeeded: AtomicU64,
    jobs_failed: AtomicU64,
    proof_duration_ms_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record the outcome of a completed job.
    pub fn record_job(&self, success: bool, duration_ms: u64) {
        self.jobs_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.jobs_succeeded.fetch_add(1, Ordering::Relaxed);
        } else {
            self.jobs_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.proof_duration_ms_total
            .fetch_add(duration_ms, Ordering::Relaxed);
    }

    pub fn jobs_total(&self) -> u64 {
        self.jobs_total.load(Ordering::Relaxed)
    }

    pub fn jobs_succeeded(&self) -> u64 {
        self.jobs_succeeded.load(Ordering::Relaxed)
    }

    pub fn jobs_failed(&self) -> u64 {
        self.jobs_failed.load(Ordering::Relaxed)
    }

    /// Render the current counters in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP lean_farm_jobs_total Total number of proof jobs processed.\n");
        out.push_str("# TYPE lean_farm_jobs_total counter\n");
        out.push_str(&format!("lean_farm_jobs_total {}\n", self.jobs_total()));

        out.push_str("# HELP lean_farm_jobs_succeeded_total Total number of successful proof jobs.\n");
        out.push_str("# TYPE lean_farm_jobs_succeeded_total counter\n");
        out.push_str(&format!(
            "lean_farm_jobs_succeeded_total {}\n",
            self.jobs_succeeded()
        ));

        out.push_str("# HELP lean_farm_jobs_failed_total Total number of failed proof jobs.\n");
        out.push_str("# TYPE lean_farm_jobs_failed_total counter\n");
        out.push_str(&format!(
            "lean_farm_jobs_failed_total {}\n",
            self.jobs_failed()
        ));

        out.push_str(
            "# HELP lean_farm_proof_duration_ms_total Cumulative proof duration in milliseconds.\n",
        );
        out.push_str("# TYPE lean_farm_proof_duration_ms_total counter\n");
        out.push_str(&format!(
            "lean_farm_proof_duration_ms_total {}\n",
            self.proof_duration_ms_total.load(Ordering::Relaxed)
        ));
        out
    }
}

/// HTTP server that exposes [`Metrics`] on a configurable port and path.
#[derive(Clone)]
pub struct MetricsServer {
    port: u16,
    path: String,
    metrics: Arc<Metrics>,
}

impl MetricsServer {
    pub fn new(port: u16, path: String, metrics: Arc<Metrics>) -> Self {
        Self {
            port,
            path: if path.is_empty() { "/metrics".to_string() } else { path },
            metrics,
        }
    }

    pub fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }

    /// Serve the metrics endpoint until the process is shut down.
    pub async fn start(&self) -> Result<(), BoxError> {
        let app = Router::new()
            .route(&self.path, get(render))
            .with_state(self.metrics.clone());

        let addr = format!("0.0.0.0:{}", self.port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("failed to bind metrics server on {addr}: {e}"))?;
        tracing::info!("metrics server listening on {addr}{}", self.path);
        axum::serve(listener, app)
            .await
            .map_err(|e| format!("metrics server error: {e}"))?;
        Ok(())
    }
}

async fn render(State(metrics): State<Arc<Metrics>>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4")],
        metrics.render_prometheus(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_renders() {
        let metrics = Metrics::new();
        metrics.record_job(true, 100);
        metrics.record_job(false, 50);
        metrics.record_job(true, 25);

        assert_eq!(metrics.jobs_total(), 3);
        assert_eq!(metrics.jobs_succeeded(), 2);
        assert_eq!(metrics.jobs_failed(), 1);

        let text = metrics.render_prometheus();
        assert!(text.contains("lean_farm_jobs_total 3"));
        assert!(text.contains("lean_farm_jobs_succeeded_total 2"));
        assert!(text.contains("lean_farm_jobs_failed_total 1"));
        assert!(text.contains("lean_farm_proof_duration_ms_total 175"));
    }
}
