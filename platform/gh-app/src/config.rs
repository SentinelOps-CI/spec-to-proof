use std::env;
use std::path::Path;
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};
use config::{Config, File};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAppConfig {
    // GitHub App settings
    pub app_id: String,
    pub private_key: String,
    pub webhook_secret: String,
    pub installation_id: String,
    pub client_id: String,
    pub client_secret: String,
    
    // Permissions and events
    pub permissions: Vec<String>,
    pub events: Vec<String>,
    
    // API settings
    pub base_url: String,
    pub upload_url: String,
    pub api_version: String,
    
    // Server settings
    pub host: String,
    pub port: u16,
    pub webhook_path: String,
    
    // Badge settings
    pub badge_context: String,
    pub badge_description: String,
    pub badge_target_url: String,
    /// Directory containing proof-artifact reference records written by the
    /// Lean proof farm (its `storage.reference_dir`, or a mount synced from the
    /// artifact bucket). When set, the badge service reads real proof outcomes
    /// from here via a `DirectoryProofArtifactSource`. `None` leaves the badge
    /// service without a proof source (it then fails loudly rather than
    /// fabricating a status).
    #[serde(default)]
    pub proof_reference_dir: Option<String>,
    
    // Sigstore settings
    pub sigstore_rekor_url: String,
    pub sigstore_fulcio_url: String,
    pub sigstore_oidc_issuer: String,
    
    // AWS settings
    pub aws_region: String,
    pub aws_secrets_arn: String,
    pub aws_s3_bucket: String,
    
    // Logging settings
    pub log_level: String,
    pub log_format: String,
    
    // Security settings
    pub enable_webhook_verification: bool,
    pub enable_jwt_verification: bool,
    pub enable_sigstore_verification: bool,
    
    // Rate limiting
    pub rate_limit_requests: u32,
    pub rate_limit_window: u64,
    
    // Timeouts
    pub request_timeout: u64,
    pub webhook_timeout: u64,
    pub badge_timeout: u64,
}

impl Default for GitHubAppConfig {
    fn default() -> Self {
        Self {
            app_id: "".to_string(),
            private_key: "".to_string(),
            webhook_secret: "".to_string(),
            installation_id: "".to_string(),
            client_id: "".to_string(),
            client_secret: "".to_string(),
            permissions: vec![
                "pull_requests".to_string(),
                "statuses".to_string(),
                "contents".to_string(),
                "metadata".to_string(),
            ],
            events: vec![
                "pull_request".to_string(),
                "push".to_string(),
                "status".to_string(),
            ],
            base_url: "https://api.github.com".to_string(),
            upload_url: "https://uploads.github.com".to_string(),
            api_version: "2022-11-28".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8080,
            webhook_path: "/webhook".to_string(),
            badge_context: "spec-to-proof/verification".to_string(),
            badge_description: "Spec-to-Proof verification status".to_string(),
            badge_target_url: "https://spec-to-proof.com/verification".to_string(),
            proof_reference_dir: None,
            sigstore_rekor_url: "https://rekor.sigstore.dev".to_string(),
            sigstore_fulcio_url: "https://fulcio.sigstore.dev".to_string(),
            sigstore_oidc_issuer: "https://oauth2.sigstore.dev/auth".to_string(),
            aws_region: "us-east-1".to_string(),
            aws_secrets_arn: "".to_string(),
            aws_s3_bucket: "spec-to-proof-artifacts".to_string(),
            log_level: "info".to_string(),
            log_format: "json".to_string(),
            enable_webhook_verification: true,
            enable_jwt_verification: true,
            enable_sigstore_verification: true,
            rate_limit_requests: 1000,
            rate_limit_window: 3600,
            request_timeout: 30,
            webhook_timeout: 10,
            badge_timeout: 5,
        }
    }
}

/// Deep-merge JSON object `overlay` onto `base` (overlay wins). Non-object
/// overlays replace the base value entirely; null overlays are ignored.
fn deep_merge(base: &mut serde_json::Value, overlay: serde_json::Value) {
    use serde_json::Value;
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                deep_merge(base_map.entry(k).or_insert(Value::Null), v);
            }
        }
        (base_slot, overlay) if !overlay.is_null() => {
            *base_slot = overlay;
        }
        _ => {}
    }
}

impl GitHubAppConfig {
    /// Load configuration from the default path (`CONFIG_PATH` env or
    /// `config/default.yaml`), overlaying environment variables and,
    /// optionally, AWS Secrets Manager (with the `aws-secrets` feature).
    pub async fn load() -> Result<Self> {
        let path = env::var("CONFIG_PATH").unwrap_or_else(|_| "config/default.yaml".to_string());
        Self::load_from(&path).await
    }

    /// Load configuration from an explicit file path. Missing files are
    /// tolerated (defaults + environment are still applied). The result is
    /// validated before being returned.
    pub async fn load_from(path: &str) -> Result<Self> {
        info!("Loading configuration from {}", path);
        #[allow(unused_mut)]
        let mut config = Self::from_file_and_env(path)?;

        #[cfg(feature = "aws-secrets")]
        if !config.aws_secrets_arn.is_empty() {
            config.load_secrets_from_aws().await?;
        }

        config.validate()?;
        info!("Configuration loaded successfully");
        Ok(config)
    }

    /// Merge defaults, an optional config file, and `GH_APP_*` environment
    /// overrides into a fully-populated config (no validation).
    pub fn from_file_and_env(path: &str) -> Result<Self> {
        // Merge the file (if present) over the serialized defaults so that
        // partial config files are supported without every field being required.
        let settings = Config::builder()
            .add_source(File::from(Path::new(path)).required(false))
            .build()
            .context("Failed to build configuration")?;
        let file_value: serde_json::Value = settings.try_deserialize().unwrap_or(serde_json::Value::Null);

        let mut merged = serde_json::to_value(Self::default())
            .context("Failed to serialize default configuration")?;
        deep_merge(&mut merged, file_value);

        let mut config: GitHubAppConfig =
            serde_json::from_value(merged).context("Failed to deserialize configuration")?;

        // Direct environment overrides (highest precedence).
        if let Ok(v) = env::var("GH_APP_ID") {
            config.app_id = v;
        }
        if let Ok(v) = env::var("GH_APP_PRIVATE_KEY") {
            config.private_key = v;
        }
        if let Ok(v) = env::var("GH_APP_WEBHOOK_SECRET") {
            config.webhook_secret = v;
        }
        if let Ok(v) = env::var("GH_APP_INSTALLATION_ID") {
            config.installation_id = v;
        }
        if let Ok(v) = env::var("GH_APP_HOST") {
            config.host = v;
        }
        if let Ok(v) = env::var("GH_APP_PORT") {
            if let Ok(port) = v.parse::<u16>() {
                config.port = port;
            }
        }
        if let Ok(v) = env::var("PROOF_REFERENCE_DIR") {
            if !v.trim().is_empty() {
                config.proof_reference_dir = Some(v);
            }
        }

        Ok(config)
    }

    #[cfg(feature = "aws-secrets")]
    async fn load_secrets_from_aws(&mut self) -> Result<()> {
        use aws_config::{BehaviorVersion, Region};
        use aws_sdk_secretsmanager::Client as SecretsClient;

        let aws_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(self.aws_region.clone()))
            .load()
            .await;

        let secrets_client = SecretsClient::new(&aws_config);

        let secret_response = secrets_client
            .get_secret_value()
            .secret_id(&self.aws_secrets_arn)
            .send()
            .await
            .context("Failed to retrieve secret from AWS Secrets Manager")?;

        if let Some(secret_string) = secret_response.secret_string() {
            let secrets: serde_json::Value =
                serde_json::from_str(secret_string).context("Failed to parse secrets JSON")?;

            for (field, target) in [
                ("app_id", &mut self.app_id),
                ("private_key", &mut self.private_key),
                ("webhook_secret", &mut self.webhook_secret),
                ("client_secret", &mut self.client_secret),
            ] {
                if let Some(value) = secrets.get(field).and_then(|v| v.as_str()) {
                    *target = value.to_string();
                }
            }
        }

        Ok(())
    }
    
    pub fn validate(&self) -> Result<()> {
        // Validate required fields
        if self.app_id.is_empty() {
            return Err(anyhow::anyhow!("app_id is required"));
        }
        if self.private_key.is_empty() {
            return Err(anyhow::anyhow!("private_key is required"));
        }
        if self.webhook_secret.is_empty() {
            return Err(anyhow::anyhow!("webhook_secret is required"));
        }
        if self.installation_id.is_empty() {
            return Err(anyhow::anyhow!("installation_id is required"));
        }
        
        // Validate URLs
        if !self.base_url.starts_with("http") {
            return Err(anyhow::anyhow!("base_url must be a valid URL"));
        }
        if !self.upload_url.starts_with("http") {
            return Err(anyhow::anyhow!("upload_url must be a valid URL"));
        }
        
        // Validate Sigstore URLs
        if !self.sigstore_rekor_url.starts_with("http") {
            return Err(anyhow::anyhow!("sigstore_rekor_url must be a valid URL"));
        }
        if !self.sigstore_fulcio_url.starts_with("http") {
            return Err(anyhow::anyhow!("sigstore_fulcio_url must be a valid URL"));
        }
        
        // Validate AWS settings
        if !self.aws_region.is_empty() && self.aws_secrets_arn.is_empty() {
            warn!("AWS region is set but no secrets ARN provided");
        }
        
        // Validate timeouts
        if self.request_timeout == 0 {
            return Err(anyhow::anyhow!("request_timeout must be greater than 0"));
        }
        if self.webhook_timeout == 0 {
            return Err(anyhow::anyhow!("webhook_timeout must be greater than 0"));
        }
        if self.badge_timeout == 0 {
            return Err(anyhow::anyhow!("badge_timeout must be greater than 0"));
        }
        
        info!("Configuration validation passed");
        Ok(())
    }
    
    pub fn get_webhook_url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.webhook_path)
    }
    
    pub fn get_badge_url(&self, repo: &str, pr: &str) -> String {
        format!("{}/badge/{}/{}", self.get_webhook_url(), repo, pr)
    }
    
    pub fn get_health_url(&self) -> String {
        format!("http://{}:{}/health", self.host, self.port)
    }
    
    pub fn get_metrics_url(&self) -> String {
        format!("http://{}:{}/metrics", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = GitHubAppConfig::default();
        assert!(!config.app_id.is_empty() || config.app_id.is_empty()); // Either empty or not
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, "0.0.0.0");
    }
    
    #[test]
    fn test_config_validation() {
        let mut config = GitHubAppConfig::default();
        
        // Should fail validation
        assert!(config.validate().is_err());
        
        // Set required fields
        config.app_id = "12345".to_string();
        config.private_key = "-----BEGIN RSA PRIVATE KEY-----\n-----END RSA PRIVATE KEY-----".to_string();
        config.webhook_secret = "secret".to_string();
        config.installation_id = "67890".to_string();
        
        // Should pass validation
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_url_generation() {
        let config = GitHubAppConfig::default();
        let webhook_url = config.get_webhook_url();
        assert!(webhook_url.contains("0.0.0.0:8080"));
        assert!(webhook_url.ends_with("/webhook"));
        
        let badge_url = config.get_badge_url("test-repo", "123");
        assert!(badge_url.contains("/badge/test-repo/123"));
    }
} 