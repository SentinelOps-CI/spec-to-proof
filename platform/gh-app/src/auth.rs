use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::GitHubAppConfig;

/// Manages GitHub App authentication: minting app/installation JWTs, verifying
/// self-issued JWTs against the public half of the configured RSA key, and
/// exchanging app JWTs for installation access tokens.
///
/// All caches use interior mutability so the manager can be shared behind an
/// `Arc` (as it is inside [`crate::AppState`]) while still updating its caches.
#[derive(Debug, Clone)]
pub struct JWTManager {
    config: GitHubAppConfig,
    token_cache: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    public_key_cache: Arc<Mutex<Option<(String, Instant)>>>,
    cache_hits: Arc<AtomicU64>,
    cache_misses: Arc<AtomicU64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JWTPayload {
    pub iss: String, // App ID
    pub iat: i64,    // Issued at
    pub exp: i64,    // Expiration time
    pub alg: String, // Algorithm
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationTokenPayload {
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    pub alg: String,
    pub aud: String, // Audience (installation ID)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubTokenResponse {
    pub token: String,
    pub expires_at: String,
    #[serde(default)]
    pub permissions: HashMap<String, String>,
    #[serde(default)]
    pub repository_selection: String,
}

const APP_JWT_CACHE_KEY: &str = "app_jwt";
const JWT_TTL_MINUTES: i64 = 10;
/// Serve a cached JWT until it is within one minute of expiry.
const JWT_CACHE_VALID: Duration = Duration::from_secs((JWT_TTL_MINUTES as u64 - 1) * 60);

impl JWTManager {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            token_cache: Arc::new(Mutex::new(HashMap::new())),
            public_key_cache: Arc::new(Mutex::new(None)),
            cache_hits: Arc::new(AtomicU64::new(0)),
            cache_misses: Arc::new(AtomicU64::new(0)),
        })
    }

    fn encoding_key(&self) -> Result<EncodingKey> {
        if self.config.private_key.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "GitHub App private key is not configured; cannot mint JWTs"
            ));
        }
        EncodingKey::from_rsa_pem(self.config.private_key.as_bytes())
            .context("Failed to parse RSA private key (expected PKCS#1 or PKCS#8 PEM)")
    }

    pub async fn create_app_jwt(&self) -> Result<String> {
        if let Some(token) = self.cached_token(APP_JWT_CACHE_KEY).await {
            return Ok(token);
        }

        let now = Utc::now();
        let exp = now + ChronoDuration::minutes(JWT_TTL_MINUTES);
        let payload = JWTPayload {
            iss: self.config.app_id.clone(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
            alg: "RS256".to_string(),
        };

        let token = encode(&Header::new(Algorithm::RS256), &payload, &self.encoding_key()?)
            .context("Failed to encode app JWT")?;

        self.store_token(APP_JWT_CACHE_KEY, &token).await;
        info!("Created app JWT, expires at {}", exp);
        Ok(token)
    }

    pub async fn create_installation_jwt(&self, installation_id: &str) -> Result<String> {
        let cache_key = format!("installation_jwt_{}", installation_id);
        if let Some(token) = self.cached_token(&cache_key).await {
            return Ok(token);
        }

        let now = Utc::now();
        let exp = now + ChronoDuration::minutes(JWT_TTL_MINUTES);
        let payload = InstallationTokenPayload {
            iss: self.config.app_id.clone(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
            alg: "RS256".to_string(),
            aud: installation_id.to_string(),
        };

        let token = encode(&Header::new(Algorithm::RS256), &payload, &self.encoding_key()?)
            .context("Failed to encode installation JWT")?;

        self.store_token(&cache_key, &token).await;
        info!("Created installation JWT for {}, expires at {}", installation_id, exp);
        Ok(token)
    }

    /// Verify a self-issued JWT: signature (against the public half of the
    /// configured RSA key), algorithm, and expiry. This is a real
    /// cryptographic verification rather than an unchecked decode.
    pub async fn verify_jwt(&self, token: &str) -> Result<JWTPayload> {
        let public_pem = self.get_public_key().await?;
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .context("Failed to build decoding key from derived public key")?;

        let mut validation = Validation::new(Algorithm::RS256);
        // We are both the issuer and verifier, so we do not pin `iss`/`aud`
        // here, but expiry is always enforced.
        validation.validate_exp = true;
        validation.required_spec_claims.clear();
        validation.set_required_spec_claims(&["exp"]);

        let token_data = decode::<JWTPayload>(token, &decoding_key, &validation)
            .context("JWT verification failed")?;

        Ok(token_data.claims)
    }

    pub async fn get_installation_token(&self, installation_id: &str) -> Result<String> {
        let cache_key = format!("installation_token_{}", installation_id);
        if let Some(token) = self.cached_token_with_ttl(&cache_key, Duration::from_secs(3300)).await {
            return Ok(token);
        }

        let jwt = self.create_installation_jwt(installation_id).await?;
        let token = self.request_installation_token(installation_id, &jwt).await?;
        self.store_token(&cache_key, &token).await;
        Ok(token)
    }

    async fn request_installation_token(&self, installation_id: &str, jwt: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.config.base_url, installation_id
        );

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "spec-to-proof-gh-app")
            .send()
            .await
            .context("Failed to request installation token")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Failed to get installation token ({}): {}",
                status,
                error_text
            ));
        }

        let token_response: GitHubTokenResponse = response
            .json()
            .await
            .context("Failed to parse installation token response")?;
        Ok(token_response.token)
    }

    /// Validate a webhook signature using constant-time HMAC-SHA256 comparison.
    pub async fn validate_webhook_signature(&self, payload: &str, signature: &str) -> Result<bool> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        if self.config.webhook_secret.is_empty() {
            return Err(anyhow::anyhow!("webhook secret is not configured"));
        }

        let hex_signature = match signature.strip_prefix("sha256=") {
            Some(sig) => sig,
            None => return Ok(false),
        };

        let provided = match hex::decode(hex_signature) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(false),
        };

        let mut mac = Hmac::<Sha256>::new_from_slice(self.config.webhook_secret.as_bytes())
            .context("Failed to create HMAC")?;
        mac.update(payload.as_bytes());

        match mac.verify_slice(&provided) {
            Ok(()) => {
                info!("Webhook signature validated successfully");
                Ok(true)
            }
            Err(_) => {
                warn!("Webhook signature validation failed");
                Ok(false)
            }
        }
    }

    /// Derive the SubjectPublicKeyInfo (SPKI) PEM of the public key from the
    /// configured RSA private key. The result is cached for one hour.
    pub async fn get_public_key(&self) -> Result<String> {
        {
            let guard = self.public_key_cache.lock().await;
            if let Some((key, created_at)) = guard.as_ref() {
                if created_at.elapsed() < Duration::from_secs(3600) {
                    return Ok(key.clone());
                }
            }
        }

        let public_key = self.extract_public_key_from_private()?;
        let mut guard = self.public_key_cache.lock().await;
        *guard = Some((public_key.clone(), Instant::now()));
        Ok(public_key)
    }

    /// Real public-key extraction: parse the configured RSA private key
    /// (PKCS#1 or PKCS#8 PEM) and re-encode its public half as SPKI PEM.
    fn extract_public_key_from_private(&self) -> Result<String> {
        let pem = self.config.private_key.trim();
        if pem.is_empty() {
            return Err(anyhow::anyhow!(
                "GitHub App private key is not configured; cannot derive public key"
            ));
        }

        let private_key = RsaPrivateKey::from_pkcs1_pem(pem)
            .or_else(|_| RsaPrivateKey::from_pkcs8_pem(pem))
            .context("Failed to parse RSA private key (expected PKCS#1 or PKCS#8 PEM)")?;

        let public_key = RsaPublicKey::from(&private_key);
        let pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .context("Failed to encode derived public key as SPKI PEM")?;
        Ok(pem)
    }

    pub async fn refresh_token_cache(&self) -> Result<()> {
        let mut cache = self.token_cache.lock().await;
        cache.retain(|_, (_, created_at)| created_at.elapsed() < Duration::from_secs(600));
        info!("Token cache refreshed, {} tokens remaining", cache.len());
        Ok(())
    }

    pub async fn get_token_statistics(&self) -> Result<TokenStatistics> {
        let cache = self.token_cache.lock().await;
        let mut stats = TokenStatistics {
            total_tokens: cache.len() as u64,
            app_jwt_count: 0,
            installation_jwt_count: 0,
            installation_token_count: 0,
            cache_hit_rate: 0.0,
        };

        for key in cache.keys() {
            if key == APP_JWT_CACHE_KEY {
                stats.app_jwt_count += 1;
            } else if key.starts_with("installation_jwt_") {
                stats.installation_jwt_count += 1;
            } else if key.starts_with("installation_token_") {
                stats.installation_token_count += 1;
            }
        }

        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        stats.cache_hit_rate = if total == 0 { 0.0 } else { hits as f64 / total as f64 };
        Ok(stats)
    }

    pub async fn invalidate_token_cache(&self) -> Result<()> {
        self.token_cache.lock().await.clear();
        Ok(())
    }

    /// Readiness signal: confirms the configured RSA key can actually mint a
    /// signed app JWT. Returns `false` (not an error) when unconfigured so the
    /// health endpoint can report "degraded" without failing the request.
    pub async fn health_check(&self) -> Result<bool> {
        match self.create_app_jwt().await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("JWT health check failed: {}", e);
                Ok(false)
            }
        }
    }

    async fn cached_token(&self, key: &str) -> Option<String> {
        self.cached_token_with_ttl(key, JWT_CACHE_VALID).await
    }

    async fn cached_token_with_ttl(&self, key: &str, ttl: Duration) -> Option<String> {
        let cache = self.token_cache.lock().await;
        if let Some((token, created_at)) = cache.get(key) {
            if created_at.elapsed() < ttl {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Some(token.clone());
            }
        }
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    async fn store_token(&self, key: &str, token: &str) {
        self.token_cache
            .lock()
            .await
            .insert(key.to_string(), (token.to_string(), Instant::now()));
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenStatistics {
    pub total_tokens: u64,
    pub app_jwt_count: u64,
    pub installation_jwt_count: u64,
    pub installation_token_count: u64,
    pub cache_hit_rate: f64,
}

/// Authentication middleware for Axum: requires a `Bearer` token that is a
/// valid, unexpired JWT signed by this app's key. Wire with
/// `axum::middleware::from_fn_with_state(state, auth_middleware)`.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<crate::AppState>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::http::Response<axum::body::Body>, axum::http::StatusCode> {
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(axum::http::StatusCode::UNAUTHORIZED)?;

    if token.is_empty() {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }

    state
        .jwt_manager
        .verify_jwt(token)
        .await
        .map_err(|_| axum::http::StatusCode::UNAUTHORIZED)?;

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic 2048-bit RSA test key (PKCS#8). Generated solely for
    /// unit tests; it authenticates nothing.
    const TEST_PRIVATE_KEY: &str = include_str!("../test-data/test_rsa_pkcs8.pem");

    fn test_config() -> GitHubAppConfig {
        GitHubAppConfig {
            app_id: "12345".to_string(),
            private_key: TEST_PRIVATE_KEY.to_string(),
            webhook_secret: "test_secret".to_string(),
            installation_id: "67890".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_jwt_manager_creation() {
        let manager = JWTManager::new(&GitHubAppConfig::default()).await;
        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_sign_then_verify_roundtrip() {
        let manager = JWTManager::new(&test_config()).await.unwrap();
        let token = manager.create_app_jwt().await.expect("should mint JWT");
        let claims = manager.verify_jwt(&token).await.expect("should verify JWT");
        assert_eq!(claims.iss, "12345");
    }

    #[tokio::test]
    async fn test_verify_rejects_tampered_token() {
        let manager = JWTManager::new(&test_config()).await.unwrap();
        let token = manager.create_app_jwt().await.unwrap();
        // Flip a character in the signature segment.
        let mut parts: Vec<&str> = token.split('.').collect();
        let tampered_sig = format!("{}A", &parts[2][..parts[2].len() - 1]);
        parts[2] = &tampered_sig;
        let tampered = parts.join(".");
        assert!(manager.verify_jwt(&tampered).await.is_err());
    }

    #[tokio::test]
    async fn test_public_key_extraction_is_spki_pem() {
        let manager = JWTManager::new(&test_config()).await.unwrap();
        let pem = manager.get_public_key().await.unwrap();
        assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----"));
    }

    #[tokio::test]
    async fn test_unconfigured_key_is_degraded_not_panic() {
        let manager = JWTManager::new(&GitHubAppConfig::default()).await.unwrap();
        assert!(!manager.health_check().await.unwrap());
        assert!(manager.create_app_jwt().await.is_err());
    }

    #[tokio::test]
    async fn test_cache_hit_rate_is_real() {
        let manager = JWTManager::new(&test_config()).await.unwrap();
        // First call misses, second hits.
        let _ = manager.create_app_jwt().await.unwrap();
        let _ = manager.create_app_jwt().await.unwrap();
        let stats = manager.get_token_statistics().await.unwrap();
        assert!(stats.cache_hit_rate > 0.0 && stats.cache_hit_rate <= 1.0);
    }

    #[tokio::test]
    async fn test_webhook_signature_validation() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let manager = JWTManager::new(&test_config()).await.unwrap();
        let payload = r#"{"action":"opened"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"test_secret").unwrap();
        mac.update(payload.as_bytes());
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        assert!(manager.validate_webhook_signature(payload, &signature).await.unwrap());
        assert!(!manager
            .validate_webhook_signature(payload, "sha256=deadbeef")
            .await
            .unwrap());
    }
}
