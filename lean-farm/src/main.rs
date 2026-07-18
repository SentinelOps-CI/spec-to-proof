use std::path::PathBuf;
use clap::Parser;
use tokio::signal;
use tracing::info;

use lean_farm::config::Config;
use lean_farm::job_runner::JobRunner;
use lean_farm::metrics::{Metrics, MetricsServer};
use lean_farm::security::SecurityManager;
use lean_farm::BoxError;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/lean-farm/config.yaml")]
    config: PathBuf,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Log format (json, text)
    #[arg(long, default_value = "json")]
    log_format: String,

    /// Metrics port (overrides config when provided)
    #[arg(long)]
    metrics_port: Option<u16>,

    /// Metrics path (overrides config when provided)
    #[arg(long)]
    metrics_path: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let args = Args::parse();

    init_logging(&args.log_level, &args.log_format)?;

    info!("Starting Lean Farm Job Runner");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));
    info!(
        "Build timestamp: {}",
        option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown")
    );

    // Load and finalise configuration (CLI flags override file values).
    let mut config = Config::from_file(&args.config).await?;
    if let Some(port) = args.metrics_port {
        config.metrics.port = port;
    }
    if let Some(path) = args.metrics_path {
        config.metrics.path = path;
    }
    config.validate()?;
    info!("Configuration loaded from {:?}", args.config);

    // Shared metrics registry used by both the HTTP exposition server and the
    // job runner.
    let metrics = Metrics::new();

    let security_manager = SecurityManager::new(&config.security)?;
    info!("Security manager initialized");
    security_manager.validate_environment().await?;
    info!("Security validation passed");

    let metrics_server =
        MetricsServer::new(config.metrics.port, config.metrics.path.clone(), metrics.clone());
    let metrics_handle = {
        let server = metrics_server.clone();
        tokio::spawn(async move { server.start().await })
    };
    info!("Metrics server starting on port {}", config.metrics.port);

    let job_runner = JobRunner::new(config, security_manager, metrics.clone()).await?;
    info!("Job runner initialized");

    let health_handle = {
        let runner = job_runner.clone();
        tokio::spawn(async move { runner.start_health_server().await })
    };
    info!("Health check server started");

    let job_handle = {
        let runner = job_runner.clone();
        tokio::spawn(async move { runner.start_processing().await })
    };
    info!("Job processing started");

    wait_for_shutdown().await;
    info!("Shutting down Lean Farm Job Runner");

    job_runner.stop().await;
    job_handle.abort();
    health_handle.abort();
    metrics_handle.abort();

    info!("Lean Farm Job Runner stopped");
    Ok(())
}

fn init_logging(level: &str, format: &str) -> Result<(), BoxError> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));

    match format {
        "json" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(false)
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .init();
        }
        "text" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .init();
        }
        other => {
            return Err(format!("unsupported log format '{other}'; use 'json' or 'text'").into());
        }
    }

    Ok(())
}

async fn wait_for_shutdown() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating shutdown");
        }
        _ = terminate => {
            info!("Received SIGTERM, initiating shutdown");
        }
    }
}
