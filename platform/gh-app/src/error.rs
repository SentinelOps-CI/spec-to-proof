use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubAppError {
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("Authentication error: {0}")]
    Auth(String),
    
    #[error("GitHub API error: {0}")]
    GitHubApi(String),
    
    #[error("Webhook error: {0}")]
    Webhook(String),
    
    #[error("Badge error: {0}")]
    Badge(String),
    
    #[error("Sigstore error: {0}")]
    Sigstore(String),
    
    #[error("JWT error: {0}")]
    JWT(String),
    
    #[error("HTTP error: {0}")]
    Http(String),
    
    #[error("JSON serialization error: {0}")]
    Json(String),
    
    #[error("Database error: {0}")]
    Database(String),
    
    #[error("Cache error: {0}")]
    Cache(String),
    
    #[error("Rate limit exceeded: {0}")]
    RateLimit(String),
    
    #[error("Timeout error: {0}")]
    Timeout(String),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for GitHubAppError {
    fn from(err: serde_json::Error) -> Self {
        GitHubAppError::Json(err.to_string())
    }
}

impl From<reqwest::Error> for GitHubAppError {
    fn from(err: reqwest::Error) -> Self {
        GitHubAppError::Http(err.to_string())
    }
}

impl From<jsonwebtoken::errors::Error> for GitHubAppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        GitHubAppError::JWT(err.to_string())
    }
}

impl From<hmac::digest::InvalidLength> for GitHubAppError {
    fn from(err: hmac::digest::InvalidLength) -> Self {
        GitHubAppError::Auth(format!("Invalid key length: {}", err))
    }
}

impl From<std::io::Error> for GitHubAppError {
    fn from(err: std::io::Error) -> Self {
        GitHubAppError::Internal(format!("IO error: {}", err))
    }
}

impl From<base64::DecodeError> for GitHubAppError {
    fn from(err: base64::DecodeError) -> Self {
        GitHubAppError::Validation(format!("Base64 decode error: {}", err))
    }
}

impl From<hex::FromHexError> for GitHubAppError {
    fn from(err: hex::FromHexError) -> Self {
        GitHubAppError::Validation(format!("Hex decode error: {}", err))
    }
}

impl From<uuid::Error> for GitHubAppError {
    fn from(err: uuid::Error) -> Self {
        GitHubAppError::Internal(format!("UUID error: {}", err))
    }
}

impl From<chrono::ParseError> for GitHubAppError {
    fn from(err: chrono::ParseError) -> Self {
        GitHubAppError::Validation(format!("Date parse error: {}", err))
    }
}

impl From<regex::Error> for GitHubAppError {
    fn from(err: regex::Error) -> Self {
        GitHubAppError::Validation(format!("Regex error: {}", err))
    }
}

impl From<anyhow::Error> for GitHubAppError {
    fn from(err: anyhow::Error) -> Self {
        GitHubAppError::Internal(err.to_string())
    }
}

impl From<GitHubAppError> for axum::http::StatusCode {
    fn from(err: GitHubAppError) -> Self {
        match err {
            GitHubAppError::Config(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            GitHubAppError::Auth(_) => axum::http::StatusCode::UNAUTHORIZED,
            GitHubAppError::GitHubApi(_) => axum::http::StatusCode::BAD_GATEWAY,
            GitHubAppError::Webhook(_) => axum::http::StatusCode::BAD_REQUEST,
            GitHubAppError::Badge(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            GitHubAppError::Sigstore(_) => axum::http::StatusCode::BAD_GATEWAY,
            GitHubAppError::JWT(_) => axum::http::StatusCode::UNAUTHORIZED,
            GitHubAppError::Http(_) => axum::http::StatusCode::BAD_GATEWAY,
            GitHubAppError::Json(_) => axum::http::StatusCode::BAD_REQUEST,
            GitHubAppError::Database(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            GitHubAppError::Cache(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            GitHubAppError::RateLimit(_) => axum::http::StatusCode::TOO_MANY_REQUESTS,
            GitHubAppError::Timeout(_) => axum::http::StatusCode::REQUEST_TIMEOUT,
            GitHubAppError::Validation(_) => axum::http::StatusCode::BAD_REQUEST,
            GitHubAppError::Internal(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<GitHubAppError> for String {
    fn from(err: GitHubAppError) -> Self {
        err.to_string()
    }
}

// Custom result type for the application
pub type Result<T> = std::result::Result<T, GitHubAppError>;

// Error response for API endpoints
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub code: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ErrorResponse {
    pub fn new(error: &str, message: &str, code: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
            code: code.to_string(),
            timestamp: chrono::Utc::now(),
        }
    }
    
    pub fn from_error(err: &GitHubAppError) -> Self {
        let (error, code) = match err {
            GitHubAppError::Config(_) => ("CONFIG_ERROR", "CONFIG_001"),
            GitHubAppError::Auth(_) => ("AUTH_ERROR", "AUTH_001"),
            GitHubAppError::GitHubApi(_) => ("GITHUB_API_ERROR", "GITHUB_001"),
            GitHubAppError::Webhook(_) => ("WEBHOOK_ERROR", "WEBHOOK_001"),
            GitHubAppError::Badge(_) => ("BADGE_ERROR", "BADGE_001"),
            GitHubAppError::Sigstore(_) => ("SIGSTORE_ERROR", "SIGSTORE_001"),
            GitHubAppError::JWT(_) => ("JWT_ERROR", "JWT_001"),
            GitHubAppError::Http(_) => ("HTTP_ERROR", "HTTP_001"),
            GitHubAppError::Json(_) => ("JSON_ERROR", "JSON_001"),
            GitHubAppError::Database(_) => ("DATABASE_ERROR", "DB_001"),
            GitHubAppError::Cache(_) => ("CACHE_ERROR", "CACHE_001"),
            GitHubAppError::RateLimit(_) => ("RATE_LIMIT_ERROR", "RATE_001"),
            GitHubAppError::Timeout(_) => ("TIMEOUT_ERROR", "TIMEOUT_001"),
            GitHubAppError::Validation(_) => ("VALIDATION_ERROR", "VALID_001"),
            GitHubAppError::Internal(_) => ("INTERNAL_ERROR", "INTERNAL_001"),
        };
        
        Self::new(error, &err.to_string(), code)
    }
}

// Error codes for different types of errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // Configuration errors
    ConfigMissingAppId,
    ConfigMissingPrivateKey,
    ConfigMissingWebhookSecret,
    ConfigInvalidUrl,
    
    // Authentication errors
    AuthInvalidJWT,
    AuthExpiredToken,
    AuthInvalidSignature,
    AuthMissingToken,
    
    // GitHub API errors
    GitHubApiRateLimit,
    GitHubApiNotFound,
    GitHubApiUnauthorized,
    GitHubApiForbidden,
    GitHubApiServerError,
    
    // Webhook errors
    WebhookInvalidSignature,
    WebhookInvalidPayload,
    WebhookUnsupportedEvent,
    WebhookProcessingFailed,
    
    // Badge errors
    BadgeUpdateFailed,
    BadgeInvalidStatus,
    BadgeCacheError,
    BadgeVerificationFailed,
    
    // Sigstore errors
    SigstoreEntryNotFound,
    SigstoreVerificationFailed,
    SigstoreInvalidCertificate,
    SigstoreLogUnavailable,
    
    // JWT errors
    JWTInvalidToken,
    JWTExpiredToken,
    JWTInvalidSignature,
    JWTInvalidAlgorithm,
    
    // HTTP errors
    HttpRequestFailed,
    HttpTimeout,
    HttpConnectionError,
    HttpInvalidResponse,
    
    // JSON errors
    JsonSerializationFailed,
    JsonDeserializationFailed,
    JsonInvalidFormat,
    
    // Database errors
    DatabaseConnectionFailed,
    DatabaseQueryFailed,
    DatabaseTransactionFailed,
    DatabaseConstraintViolation,
    
    // Cache errors
    CacheGetFailed,
    CacheSetFailed,
    CacheDeleteFailed,
    CacheConnectionFailed,
    
    // Rate limiting errors
    RateLimitExceeded,
    RateLimitWindowExpired,
    RateLimitInvalidRequest,
    
    // Timeout errors
    TimeoutRequest,
    TimeoutOperation,
    TimeoutConnection,
    
    // Validation errors
    ValidationInvalidInput,
    ValidationMissingField,
    ValidationInvalidFormat,
    ValidationConstraintViolation,
    
    // Internal errors
    InternalServerError,
    InternalProcessingError,
    InternalStateError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            // Configuration errors
            ErrorCode::ConfigMissingAppId => "CONFIG_001",
            ErrorCode::ConfigMissingPrivateKey => "CONFIG_002",
            ErrorCode::ConfigMissingWebhookSecret => "CONFIG_003",
            ErrorCode::ConfigInvalidUrl => "CONFIG_004",
            
            // Authentication errors
            ErrorCode::AuthInvalidJWT => "AUTH_001",
            ErrorCode::AuthExpiredToken => "AUTH_002",
            ErrorCode::AuthInvalidSignature => "AUTH_003",
            ErrorCode::AuthMissingToken => "AUTH_004",
            
            // GitHub API errors
            ErrorCode::GitHubApiRateLimit => "GITHUB_001",
            ErrorCode::GitHubApiNotFound => "GITHUB_002",
            ErrorCode::GitHubApiUnauthorized => "GITHUB_003",
            ErrorCode::GitHubApiForbidden => "GITHUB_004",
            ErrorCode::GitHubApiServerError => "GITHUB_005",
            
            // Webhook errors
            ErrorCode::WebhookInvalidSignature => "WEBHOOK_001",
            ErrorCode::WebhookInvalidPayload => "WEBHOOK_002",
            ErrorCode::WebhookUnsupportedEvent => "WEBHOOK_003",
            ErrorCode::WebhookProcessingFailed => "WEBHOOK_004",
            
            // Badge errors
            ErrorCode::BadgeUpdateFailed => "BADGE_001",
            ErrorCode::BadgeInvalidStatus => "BADGE_002",
            ErrorCode::BadgeCacheError => "BADGE_003",
            ErrorCode::BadgeVerificationFailed => "BADGE_004",
            
            // Sigstore errors
            ErrorCode::SigstoreEntryNotFound => "SIGSTORE_001",
            ErrorCode::SigstoreVerificationFailed => "SIGSTORE_002",
            ErrorCode::SigstoreInvalidCertificate => "SIGSTORE_003",
            ErrorCode::SigstoreLogUnavailable => "SIGSTORE_004",
            
            // JWT errors
            ErrorCode::JWTInvalidToken => "JWT_001",
            ErrorCode::JWTExpiredToken => "JWT_002",
            ErrorCode::JWTInvalidSignature => "JWT_003",
            ErrorCode::JWTInvalidAlgorithm => "JWT_004",
            
            // HTTP errors
            ErrorCode::HttpRequestFailed => "HTTP_001",
            ErrorCode::HttpTimeout => "HTTP_002",
            ErrorCode::HttpConnectionError => "HTTP_003",
            ErrorCode::HttpInvalidResponse => "HTTP_004",
            
            // JSON errors
            ErrorCode::JsonSerializationFailed => "JSON_001",
            ErrorCode::JsonDeserializationFailed => "JSON_002",
            ErrorCode::JsonInvalidFormat => "JSON_003",
            
            // Database errors
            ErrorCode::DatabaseConnectionFailed => "DB_001",
            ErrorCode::DatabaseQueryFailed => "DB_002",
            ErrorCode::DatabaseTransactionFailed => "DB_003",
            ErrorCode::DatabaseConstraintViolation => "DB_004",
            
            // Cache errors
            ErrorCode::CacheGetFailed => "CACHE_001",
            ErrorCode::CacheSetFailed => "CACHE_002",
            ErrorCode::CacheDeleteFailed => "CACHE_003",
            ErrorCode::CacheConnectionFailed => "CACHE_004",
            
            // Rate limiting errors
            ErrorCode::RateLimitExceeded => "RATE_001",
            ErrorCode::RateLimitWindowExpired => "RATE_002",
            ErrorCode::RateLimitInvalidRequest => "RATE_003",
            
            // Timeout errors
            ErrorCode::TimeoutRequest => "TIMEOUT_001",
            ErrorCode::TimeoutOperation => "TIMEOUT_002",
            ErrorCode::TimeoutConnection => "TIMEOUT_003",
            
            // Validation errors
            ErrorCode::ValidationInvalidInput => "VALID_001",
            ErrorCode::ValidationMissingField => "VALID_002",
            ErrorCode::ValidationInvalidFormat => "VALID_003",
            ErrorCode::ValidationConstraintViolation => "VALID_004",
            
            // Internal errors
            ErrorCode::InternalServerError => "INTERNAL_001",
            ErrorCode::InternalProcessingError => "INTERNAL_002",
            ErrorCode::InternalStateError => "INTERNAL_003",
        }
    }
    
    pub fn description(&self) -> &'static str {
        match self {
            // Configuration errors
            ErrorCode::ConfigMissingAppId => "Missing GitHub App ID",
            ErrorCode::ConfigMissingPrivateKey => "Missing private key",
            ErrorCode::ConfigMissingWebhookSecret => "Missing webhook secret",
            ErrorCode::ConfigInvalidUrl => "Invalid URL configuration",
            
            // Authentication errors
            ErrorCode::AuthInvalidJWT => "Invalid JWT token",
            ErrorCode::AuthExpiredToken => "Token has expired",
            ErrorCode::AuthInvalidSignature => "Invalid signature",
            ErrorCode::AuthMissingToken => "Missing authentication token",
            
            // GitHub API errors
            ErrorCode::GitHubApiRateLimit => "GitHub API rate limit exceeded",
            ErrorCode::GitHubApiNotFound => "GitHub API resource not found",
            ErrorCode::GitHubApiUnauthorized => "GitHub API unauthorized",
            ErrorCode::GitHubApiForbidden => "GitHub API forbidden",
            ErrorCode::GitHubApiServerError => "GitHub API server error",
            
            // Webhook errors
            ErrorCode::WebhookInvalidSignature => "Invalid webhook signature",
            ErrorCode::WebhookInvalidPayload => "Invalid webhook payload",
            ErrorCode::WebhookUnsupportedEvent => "Unsupported webhook event",
            ErrorCode::WebhookProcessingFailed => "Webhook processing failed",
            
            // Badge errors
            ErrorCode::BadgeUpdateFailed => "Badge update failed",
            ErrorCode::BadgeInvalidStatus => "Invalid badge status",
            ErrorCode::BadgeCacheError => "Badge cache error",
            ErrorCode::BadgeVerificationFailed => "Badge verification failed",
            
            // Sigstore errors
            ErrorCode::SigstoreEntryNotFound => "Sigstore entry not found",
            ErrorCode::SigstoreVerificationFailed => "Sigstore verification failed",
            ErrorCode::SigstoreInvalidCertificate => "Invalid Sigstore certificate",
            ErrorCode::SigstoreLogUnavailable => "Sigstore log unavailable",
            
            // JWT errors
            ErrorCode::JWTInvalidToken => "Invalid JWT token",
            ErrorCode::JWTExpiredToken => "JWT token expired",
            ErrorCode::JWTInvalidSignature => "Invalid JWT signature",
            ErrorCode::JWTInvalidAlgorithm => "Invalid JWT algorithm",
            
            // HTTP errors
            ErrorCode::HttpRequestFailed => "HTTP request failed",
            ErrorCode::HttpTimeout => "HTTP request timeout",
            ErrorCode::HttpConnectionError => "HTTP connection error",
            ErrorCode::HttpInvalidResponse => "Invalid HTTP response",
            
            // JSON errors
            ErrorCode::JsonSerializationFailed => "JSON serialization failed",
            ErrorCode::JsonDeserializationFailed => "JSON deserialization failed",
            ErrorCode::JsonInvalidFormat => "Invalid JSON format",
            
            // Database errors
            ErrorCode::DatabaseConnectionFailed => "Database connection failed",
            ErrorCode::DatabaseQueryFailed => "Database query failed",
            ErrorCode::DatabaseTransactionFailed => "Database transaction failed",
            ErrorCode::DatabaseConstraintViolation => "Database constraint violation",
            
            // Cache errors
            ErrorCode::CacheGetFailed => "Cache get operation failed",
            ErrorCode::CacheSetFailed => "Cache set operation failed",
            ErrorCode::CacheDeleteFailed => "Cache delete operation failed",
            ErrorCode::CacheConnectionFailed => "Cache connection failed",
            
            // Rate limiting errors
            ErrorCode::RateLimitExceeded => "Rate limit exceeded",
            ErrorCode::RateLimitWindowExpired => "Rate limit window expired",
            ErrorCode::RateLimitInvalidRequest => "Invalid rate limit request",
            
            // Timeout errors
            ErrorCode::TimeoutRequest => "Request timeout",
            ErrorCode::TimeoutOperation => "Operation timeout",
            ErrorCode::TimeoutConnection => "Connection timeout",
            
            // Validation errors
            ErrorCode::ValidationInvalidInput => "Invalid input",
            ErrorCode::ValidationMissingField => "Missing required field",
            ErrorCode::ValidationInvalidFormat => "Invalid format",
            ErrorCode::ValidationConstraintViolation => "Constraint violation",
            
            // Internal errors
            ErrorCode::InternalServerError => "Internal server error",
            ErrorCode::InternalProcessingError => "Internal processing error",
            ErrorCode::InternalStateError => "Internal state error",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.as_str(), self.description())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_code_display() {
        let error = ErrorCode::ConfigMissingAppId;
        let display = error.to_string();
        assert!(display.contains("CONFIG_001"));
        assert!(display.contains("Missing GitHub App ID"));
    }
    
    #[test]
    fn test_error_response_creation() {
        let response = ErrorResponse::new("TEST_ERROR", "Test message", "TEST_001");
        assert_eq!(response.error, "TEST_ERROR");
        assert_eq!(response.message, "Test message");
        assert_eq!(response.code, "TEST_001");
    }
    
    #[test]
    fn test_error_response_from_error() {
        let err = GitHubAppError::Config("Missing app ID".to_string());
        let response = ErrorResponse::from_error(&err);
        assert_eq!(response.error, "CONFIG_ERROR");
        assert_eq!(response.code, "CONFIG_001");
    }
    
    #[test]
    fn test_error_code_conversion() {
        let status: axum::http::StatusCode = GitHubAppError::Auth("Invalid token".to_string()).into();
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    }
} 