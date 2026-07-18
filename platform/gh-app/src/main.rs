use tokio::signal;
use tracing::{info, warn, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use clap::Parser;
use anyhow::Result;

use gh_app::config::GitHubAppConfig;
use gh_app::server::Server;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "config/default.yaml")]
    config: String,
    
    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
    
    /// Log format (json, text)
    #[arg(long, default_value = "json")]
    log_format: String,
    
    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    
    /// Port to bind to
    #[arg(long, default_value = "8080")]
    port: u16,
    
    /// Enable debug mode
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();
    
    // Initialize logging
    init_logging(&args.log_level, &args.log_format, args.debug)?;
    
    info!("Starting Spec-to-Proof GitHub App");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));
    info!("Config path: {}", args.config);
    info!("Log level: {}", args.log_level);
    info!("Log format: {}", args.log_format);
    
    // Load configuration
    let config = load_config(&args.config).await?;
    info!("Configuration loaded successfully");
    
    // Create and start server
    let server = Server::new(config).await?;
    info!("Server created successfully");
    
    // Set up graceful shutdown (cross-platform: Ctrl+C everywhere, SIGTERM on Unix).
    let shutdown_signal = shutdown_signal();
    
    // Run server with graceful shutdown
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.run_with_config().await {
            error!("Server error: {}", e);
        }
    });
    
    // Wait for shutdown signal
    shutdown_signal.await;
    
    // Graceful shutdown
    info!("Initiating graceful shutdown...");
    
    // Cancel server task
    server_handle.abort();
    
    // Wait for server to shutdown
    match server_handle.await {
        Ok(_) => info!("Server shutdown completed successfully"),
        Err(e) if e.is_cancelled() => info!("Server was cancelled during shutdown"),
        Err(e) => error!("Server shutdown error: {}", e),
    }
    
    info!("Spec-to-Proof GitHub App shutdown complete");
    Ok(())
}

/// Wait for a shutdown signal. Listens for Ctrl+C on all platforms and, on
/// Unix, also for SIGTERM (as used by container orchestrators).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failed to install Ctrl+C handler: {}", e);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => error!("Failed to install SIGTERM handler: {}", e),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C, initiating shutdown"),
        _ = terminate => info!("Received SIGTERM, initiating shutdown"),
    }
}

fn init_logging(level: &str, format: &str, debug: bool) -> Result<()> {
    // Validate the requested level early so a bad value surfaces at startup.
    level
        .parse::<tracing::Level>()
        .map_err(|e| anyhow::anyhow!("invalid log level '{}': {}", level, e))?;

    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("gh_app={}", level).into())
        );
    
    match format.to_lowercase().as_str() {
        "json" => {
            subscriber
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
        "text" => {
            subscriber
                .with(tracing_subscriber::fmt::layer().pretty())
                .init();
        }
        _ => {
            warn!("Unknown log format '{}', using JSON", format);
            subscriber
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
    }
    
    if debug {
        info!("Debug mode enabled");
    }
    
    Ok(())
}

/// Load and validate configuration from a YAML file (if present), overlaid
/// with `GH_APP_*` environment variables and optional AWS Secrets Manager.
async fn load_config(config_path: &str) -> Result<GitHubAppConfig> {
    let config = GitHubAppConfig::load_from(config_path).await?;
    info!("Configuration loaded from {} (+ environment overrides)", config_path);
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_args_parsing() {
        let args = Args::parse_from(["gh-app", "--config", "test.yaml"]);
        assert_eq!(args.config, "test.yaml");
        assert_eq!(args.log_level, "info");
        assert_eq!(args.log_format, "json");
    }
    
    #[tokio::test]
    async fn test_config_loading() {
        // Set environment variables for testing
        std::env::set_var("GH_APP_ID", "12345");
        std::env::set_var("GH_APP_PRIVATE_KEY", "test-key");
        std::env::set_var("GH_APP_WEBHOOK_SECRET", "test-secret");
        std::env::set_var("GH_APP_INSTALLATION_ID", "67890");
        
        let config = load_config("test.yaml").await;
        assert!(config.is_ok());
        
        let config = config.unwrap();
        assert_eq!(config.app_id, "12345");
        assert_eq!(config.private_key, "test-key");
        assert_eq!(config.webhook_secret, "test-secret");
        assert_eq!(config.installation_id, "67890");
        
        // Clean up environment variables
        std::env::remove_var("GH_APP_ID");
        std::env::remove_var("GH_APP_PRIVATE_KEY");
        std::env::remove_var("GH_APP_WEBHOOK_SECRET");
        std::env::remove_var("GH_APP_INSTALLATION_ID");
    }
    
    #[test]
    fn test_logging_initialization() {
        let result = init_logging("info", "json", false);
        assert!(result.is_ok());
    }
} 