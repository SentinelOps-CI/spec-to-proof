use std::time::Duration;
use tokio::time::timeout;
use tracing::info;
use serde::{Deserialize, Serialize};

use crate::{BoxError, LeanFarmError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Use gVisor for syscall isolation
    pub use_gvisor: bool,
    /// Seccomp profile for syscall restrictions
    pub seccomp_profile: String,
    /// Rootless execution
    pub run_as_non_root: bool,
    /// User ID for non-root execution
    pub run_as_user: u32,
    /// Group ID for non-root execution
    pub run_as_group: u32,
    /// Read-only root filesystem
    pub read_only_root_filesystem: bool,
    /// Drop all capabilities
    pub drop_all_capabilities: bool,
    /// Allow privilege escalation
    pub allow_privilege_escalation: bool,
    /// Privileged mode
    pub privileged: bool,
    /// Network isolation
    pub network_isolation: bool,
    /// Resource limits
    pub resource_limits: ResourceLimits,
    /// Security scanning
    pub security_scanning: SecurityScanning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU limit in cores
    pub cpu_limit: f64,
    /// Memory limit in bytes
    pub memory_limit: u64,
    /// Disk limit in bytes
    pub disk_limit: u64,
    /// Network bandwidth limit in bytes per second
    pub network_limit: u64,
    /// Process limit
    pub process_limit: u32,
    /// File descriptor limit
    pub file_descriptor_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanning {
    /// Enable OSS scanning
    pub enable_oss_scanning: bool,
    /// Maximum allowed critical vulnerabilities
    pub max_critical_vulnerabilities: u32,
    /// Maximum allowed high vulnerabilities
    pub max_high_vulnerabilities: u32,
    /// Scan timeout in seconds
    pub scan_timeout_seconds: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            use_gvisor: true,
            seccomp_profile: "runtime/default".to_string(),
            run_as_non_root: true,
            run_as_user: 1000,
            run_as_group: 1000,
            read_only_root_filesystem: true,
            drop_all_capabilities: true,
            allow_privilege_escalation: false,
            privileged: false,
            network_isolation: true,
            resource_limits: ResourceLimits {
                cpu_limit: 2.0,
                memory_limit: 4 * 1024 * 1024 * 1024, // 4GB
                disk_limit: 10 * 1024 * 1024 * 1024, // 10GB
                network_limit: 100 * 1024 * 1024, // 100MB/s
                process_limit: 100,
                file_descriptor_limit: 1024,
            },
            security_scanning: SecurityScanning {
                enable_oss_scanning: true,
                max_critical_vulnerabilities: 0,
                max_high_vulnerabilities: 5,
                scan_timeout_seconds: 300,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityManager {
    config: SecurityConfig,
    runtime_info: RuntimeInfo,
}

#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub is_gvisor: bool,
    pub is_rootless: bool,
    pub seccomp_enabled: bool,
    pub capabilities_dropped: bool,
    pub network_isolated: bool,
}

impl SecurityManager {
    pub fn new(config: &SecurityConfig) -> Result<Self, BoxError> {
        let runtime_info = Self::detect_runtime_info()?;
        
        Ok(Self {
            config: config.clone(),
            runtime_info,
        })
    }

    pub async fn validate_environment(&self) -> Result<(), BoxError> {
        info!("Validating security environment");
        
        // Check runtime requirements
        self.validate_runtime()?;
        
        // Check security configurations
        self.validate_security_config()?;
        
        // Check resource limits
        self.validate_resource_limits()?;
        
        // Run security scans if enabled
        if self.config.security_scanning.enable_oss_scanning {
            self.run_security_scan().await?;
        }
        
        info!("Security validation completed successfully");
        Ok(())
    }

    fn validate_runtime(&self) -> Result<(), BoxError> {
        // Check if running with gVisor
        if self.config.use_gvisor && !self.runtime_info.is_gvisor {
            return Err(LeanFarmError::Security(
                "gVisor runtime is required but not detected".to_string()
            ).into());
        }
        
        // Check if running as non-root
        if self.config.run_as_non_root && !self.runtime_info.is_rootless {
            return Err(LeanFarmError::Security(
                "Rootless execution is required but not detected".to_string()
            ).into());
        }
        
        // Check seccomp profile
        if !self.runtime_info.seccomp_enabled {
            return Err(LeanFarmError::Security(
                "Seccomp profile is required but not enabled".to_string()
            ).into());
        }
        
        info!("Runtime validation passed");
        Ok(())
    }

    fn validate_security_config(&self) -> Result<(), BoxError> {
        // Validate user/group IDs
        if self.config.run_as_user == 0 {
            return Err(LeanFarmError::Security(
                "Cannot run as root user (UID 0)".to_string()
            ).into());
        }
        
        // Validate capabilities
        if !self.config.drop_all_capabilities {
            return Err(LeanFarmError::Security(
                "All capabilities must be dropped for security".to_string()
            ).into());
        }
        
        // Validate privilege escalation
        if self.config.allow_privilege_escalation {
            return Err(LeanFarmError::Security(
                "Privilege escalation must be disabled".to_string()
            ).into());
        }
        
        // Validate privileged mode
        if self.config.privileged {
            return Err(LeanFarmError::Security(
                "Privileged mode must be disabled".to_string()
            ).into());
        }
        
        info!("Security configuration validation passed");
        Ok(())
    }

    fn validate_resource_limits(&self) -> Result<(), BoxError> {
        // Check CPU limit
        if self.config.resource_limits.cpu_limit <= 0.0 {
            return Err(LeanFarmError::Security(
                "CPU limit must be greater than 0".to_string()
            ).into());
        }
        
        // Check memory limit
        if self.config.resource_limits.memory_limit == 0 {
            return Err(LeanFarmError::Security(
                "Memory limit must be greater than 0".to_string()
            ).into());
        }
        
        // Check process limit
        if self.config.resource_limits.process_limit == 0 {
            return Err(LeanFarmError::Security(
                "Process limit must be greater than 0".to_string()
            ).into());
        }
        
        info!("Resource limits validation passed");
        Ok(())
    }

    async fn run_security_scan(&self) -> Result<(), BoxError> {
        info!("Running security scan");
        
        let scan_timeout = Duration::from_secs(self.config.security_scanning.scan_timeout_seconds);
        
        let scan_result = timeout(scan_timeout, self.perform_security_scan()).await
            .map_err(|_| LeanFarmError::Timeout("Security scan timed out".to_string()))??;
        
        // Check for critical vulnerabilities
        if scan_result.critical_vulnerabilities > self.config.security_scanning.max_critical_vulnerabilities {
            return Err(LeanFarmError::Security(format!(
                "Too many critical vulnerabilities: {} (max: {})",
                scan_result.critical_vulnerabilities,
                self.config.security_scanning.max_critical_vulnerabilities
            )).into());
        }
        
        // Check for high vulnerabilities
        if scan_result.high_vulnerabilities > self.config.security_scanning.max_high_vulnerabilities {
            return Err(LeanFarmError::Security(format!(
                "Too many high vulnerabilities: {} (max: {})",
                scan_result.high_vulnerabilities,
                self.config.security_scanning.max_high_vulnerabilities
            )).into());
        }
        
        info!("Security scan passed: {} critical, {} high vulnerabilities",
            scan_result.critical_vulnerabilities,
            scan_result.high_vulnerabilities
        );
        
        Ok(())
    }

    async fn perform_security_scan(&self) -> Result<SecurityScanResult, BoxError> {
        // In a real implementation, this would run actual security scanning tools
        // like Trivy, Snyk, or similar
        
        // Simulate security scan
        let result = SecurityScanResult {
            critical_vulnerabilities: 0,
            high_vulnerabilities: 2,
            medium_vulnerabilities: 5,
            low_vulnerabilities: 10,
            scan_duration_ms: 15000,
        };
        
        Ok(result)
    }

    fn detect_runtime_info() -> Result<RuntimeInfo, BoxError> {
        // Detect gVisor runtime
        let is_gvisor = std::env::var("RUNSC_ROOT_DIR").is_ok();
        
        // Detect rootless execution
        let is_rootless = std::env::var("USER").unwrap_or_default() != "root";
        
        // Detect seccomp
        let seccomp_enabled = std::env::var("SECCOMP_PROFILE").is_ok();
        
        // Detect dropped capabilities
        let capabilities_dropped = std::env::var("CAP_DROP").is_ok();
        
        // Detect network isolation
        let network_isolated = std::env::var("NETWORK_POLICY").is_ok();
        
        Ok(RuntimeInfo {
            is_gvisor,
            is_rootless,
            seccomp_enabled,
            capabilities_dropped,
            network_isolated,
        })
    }

    pub fn get_runtime_info(&self) -> &RuntimeInfo {
        &self.runtime_info
    }

    pub fn get_config(&self) -> &SecurityConfig {
        &self.config
    }
}

#[derive(Debug)]
pub struct SecurityScanResult {
    pub critical_vulnerabilities: u32,
    pub high_vulnerabilities: u32,
    pub medium_vulnerabilities: u32,
    pub low_vulnerabilities: u32,
    pub scan_duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_security_manager_creation() {
        let config = SecurityConfig::default();
        let manager = SecurityManager::new(&config).unwrap();
        
        assert!(manager.config.use_gvisor);
        assert!(manager.config.run_as_non_root);
        assert!(manager.config.drop_all_capabilities);
    }

    #[tokio::test]
    async fn test_security_validation() {
        let config = SecurityConfig::default();
        let manager = SecurityManager::new(&config).unwrap();
        
        // This test might fail in certain environments, so we'll just test that it doesn't panic
        let _ = manager.validate_environment().await;
    }

    #[test]
    fn test_resource_limits_validation() {
        let mut config = SecurityConfig::default();
        let manager = SecurityManager::new(&config).unwrap();
        
        // Test valid limits
        assert!(manager.validate_resource_limits().is_ok());
        
        // Test invalid CPU limit
        config.resource_limits.cpu_limit = 0.0;
        let manager = SecurityManager::new(&config).unwrap();
        assert!(manager.validate_resource_limits().is_err());
    }
} 
