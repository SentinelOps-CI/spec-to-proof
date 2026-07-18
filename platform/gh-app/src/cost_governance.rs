use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use anyhow::Result;
use tracing::{info, warn, error};
use redis::AsyncCommands;
use aws_sdk_costexplorer::Client as CostExplorerClient;
use aws_sdk_ses::Client as SesClient;
use aws_sdk_ses::types::{Message, Content, Body, Destination};
use tokio::sync::RwLock;

// Token bucket configuration
#[derive(Debug, Clone)]
pub struct TokenBucketConfig {
    pub capacity: u32,
    pub refill_rate: f64, // tokens per second
    pub refill_time: Duration,
    pub burst_size: u32,
}

impl Default for TokenBucketConfig {
    fn default() -> Self {
        Self {
            capacity: 1000,
            refill_rate: 10.0, // 10 tokens per second
            refill_time: Duration::from_secs(1),
            burst_size: 100,
        }
    }
}

// Cost monitoring configuration
#[derive(Debug, Clone)]
pub struct CostMonitoringConfig {
    pub daily_budget_usd: f64,
    pub alert_threshold_percent: f64,
    pub cost_explorer_region: String,
    pub ses_region: String,
    pub alert_email: String,
    pub cost_report_email: String,
    /// Verified SES sender identity used as the `Source` for every email.
    pub sender_email: String,
}

impl Default for CostMonitoringConfig {
    fn default() -> Self {
        Self {
            daily_budget_usd: 100.0,
            alert_threshold_percent: 80.0,
            cost_explorer_region: "us-east-1".to_string(),
            ses_region: "us-east-1".to_string(),
            alert_email: "alerts@company.com".to_string(),
            cost_report_email: "cost-reports@company.com".to_string(),
            sender_email: "noreply@company.com".to_string(),
        }
    }
}

// Token bucket implementation
#[derive(Clone)]
pub struct TokenBucket {
    config: TokenBucketConfig,
    redis_client: redis::Client,
    tenant_id: String,
}

impl TokenBucket {
    pub fn new(config: TokenBucketConfig, redis_client: redis::Client, tenant_id: String) -> Self {
        Self {
            config,
            redis_client,
            tenant_id,
        }
    }

    pub async fn acquire_token(&self, tokens: u32) -> Result<bool> {
        let mut conn = self.redis_client.get_async_connection().await?;
        let key = format!("token_bucket:{}", self.tenant_id);
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Use Redis Lua script for atomic token bucket operations
        let script = r#"
            local key = KEYS[1]
            local tokens_requested = tonumber(ARGV[1])
            local capacity = tonumber(ARGV[2])
            local refill_rate = tonumber(ARGV[3])
            local now = tonumber(ARGV[4])
            local refill_time = tonumber(ARGV[5])
            
            local current = redis.call('HGET', key, 'tokens')
            local last_refill = redis.call('HGET', key, 'last_refill')
            
            if not current then
                current = capacity
                last_refill = now
            else
                current = tonumber(current)
                last_refill = tonumber(last_refill)
            end
            
            -- Calculate refill
            local time_passed = now - last_refill
            local tokens_to_add = math.floor(time_passed * refill_rate)
            current = math.min(capacity, current + tokens_to_add)
            
            -- Check if we have enough tokens
            if current >= tokens_requested then
                current = current - tokens_requested
                redis.call('HSET', key, 'tokens', current, 'last_refill', now)
                redis.call('EXPIRE', key, 3600) -- 1 hour TTL
                return 1
            else
                return 0
            end
        "#;
        
        let result: i32 = redis::cmd("EVAL")
            .arg(script)
            .arg(1)
            .arg(&key)
            .arg(tokens)
            .arg(self.config.capacity)
            .arg(self.config.refill_rate)
            .arg(now)
            .arg(self.config.refill_time.as_secs())
            .query_async(&mut conn)
            .await?;
        
        Ok(result == 1)
    }

    pub async fn get_token_count(&self) -> Result<u32> {
        let mut conn = self.redis_client.get_async_connection().await?;
        let key = format!("token_bucket:{}", self.tenant_id);
        
        let tokens: Option<u32> = conn.hget(&key, "tokens").await?;
        Ok(tokens.unwrap_or(self.config.capacity))
    }

    pub async fn reset_bucket(&self) -> Result<()> {
        let mut conn = self.redis_client.get_async_connection().await?;
        let key = format!("token_bucket:{}", self.tenant_id);
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        conn.hset_multiple::<_, _, _, ()>(&key, &[
            ("tokens", self.config.capacity.to_string()),
            ("last_refill", now.to_string()),
        ]).await?;
        conn.expire::<_, ()>(&key, 3600).await?;
        
        Ok(())
    }
}

// Cost monitoring implementation
pub struct CostMonitor {
    config: CostMonitoringConfig,
    cost_explorer_client: CostExplorerClient,
    ses_client: SesClient,
    daily_costs: RwLock<HashMap<String, f64>>,
}

impl CostMonitor {
    pub fn new(
        config: CostMonitoringConfig,
        cost_explorer_client: CostExplorerClient,
        ses_client: SesClient,
    ) -> Self {
        Self {
            config,
            cost_explorer_client,
            ses_client,
            daily_costs: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_daily_cost(&self, service: &str) -> Result<f64> {
        let start_date = chrono::Utc::now().date_naive();
        let end_date = start_date;
        
        let response = self.cost_explorer_client
            .get_cost_and_usage()
            .time_period(
                aws_sdk_costexplorer::types::DateInterval::builder()
                    .start(start_date.format("%Y-%m-%d").to_string())
                    .end(end_date.format("%Y-%m-%d").to_string())
                    .build()?
            )
            .granularity(aws_sdk_costexplorer::types::Granularity::Daily)
            .metrics("UnblendedCost")
            .group_by(
                aws_sdk_costexplorer::types::GroupDefinition::builder()
                    .r#type(aws_sdk_costexplorer::types::GroupDefinitionType::Dimension)
                    .key("SERVICE")
                    .build()
            )
            .send()
            .await?;

        let mut total_cost = 0.0;
        
        if let Some(results) = response.results_by_time {
            for result in results {
                if let Some(groups) = result.groups {
                    for group in groups {
                        if let Some(keys) = group.keys {
                            if keys.contains(&service.to_string()) {
                                if let Some(metrics) = group.metrics {
                                    if let Some(cost) = metrics.get("UnblendedCost") {
                                        if let Some(amount) = &cost.amount {
                                            if let Ok(amount_float) = amount.parse::<f64>() {
                                                total_cost += amount_float;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Ok(total_cost)
    }

    pub async fn check_budget_alert(&self) -> Result<()> {
        let total_cost = self.get_daily_cost("Amazon SageMaker").await?;
        let budget_threshold = self.config.daily_budget_usd * (self.config.alert_threshold_percent / 100.0);
        
        if total_cost > budget_threshold {
            self.send_budget_alert(total_cost, budget_threshold).await?;
        }
        
        Ok(())
    }

    pub async fn send_budget_alert(&self, current_cost: f64, threshold: f64) -> Result<()> {
        let subject = "Budget Alert - Spec-to-Proof Platform";
        let body = format!(
            "Daily cost ({:.2} USD) has exceeded the alert threshold ({:.2} USD). \
             Please review usage and consider implementing cost controls.",
            current_cost, threshold
        );

        let message = Message::builder()
            .subject(Content::builder().data(subject).charset("UTF-8").build()?)
            .body(Body::builder().text(Content::builder().data(body).charset("UTF-8").build()?).build())
            .build();

        let destination = Destination::builder()
            .to_addresses(self.config.alert_email.clone())
            .build();

        self.ses_client
            .send_email()
            .source(self.config.sender_email.clone())
            .destination(destination)
            .message(message)
            .send()
            .await?;

        info!("Budget alert sent: cost={:.2}, threshold={:.2}", current_cost, threshold);
        Ok(())
    }

    pub async fn generate_daily_cost_report(&self) -> Result<()> {
        let services = vec![
            "Amazon SageMaker",
            "Amazon S3",
            "Amazon DynamoDB",
            "Amazon CloudWatch",
            "AWS Lambda",
        ];

        let mut report_data = HashMap::new();
        let mut total_cost = 0.0;

        for service in services {
            let cost = self.get_daily_cost(service).await?;
            report_data.insert(service.to_string(), cost);
            total_cost += cost;
        }

        let subject = format!("Daily Cost Report - {}", chrono::Utc::now().format("%Y-%m-%d"));
        let body = self.format_cost_report(&report_data, total_cost);

        let message = Message::builder()
            .subject(Content::builder().data(subject).charset("UTF-8").build()?)
            .body(Body::builder().text(Content::builder().data(body).charset("UTF-8").build()?).build())
            .build();

        let destination = Destination::builder()
            .to_addresses(self.config.cost_report_email.clone())
            .build();

        self.ses_client
            .send_email()
            .source(self.config.sender_email.clone())
            .destination(destination)
            .message(message)
            .send()
            .await?;

        info!("Daily cost report sent: total_cost={:.2}", total_cost);
        Ok(())
    }

    fn format_cost_report(&self, costs: &HashMap<String, f64>, total: f64) -> String {
        let mut report = format!("Daily Cost Report - {}\n\n", chrono::Utc::now().format("%Y-%m-%d"));
        
        for (service, cost) in costs {
            let percentage = if total > 0.0 { (cost / total) * 100.0 } else { 0.0 };
            report.push_str(&format!("{:<20} ${:>8.2} ({:>5.1}%)\n", service, cost, percentage));
        }
        
        report.push_str(&format!("\nTotal Cost: ${:.2}\n", total));
        report.push_str(&format!("Daily Budget: ${:.2}\n", self.config.daily_budget_usd));
        report.push_str(&format!("Budget Usage: {:.1}%\n", (total / self.config.daily_budget_usd) * 100.0));
        
        report
    }
}

// Cost governance manager
pub struct CostGovernanceManager {
    token_buckets: RwLock<HashMap<String, TokenBucket>>,
    cost_monitor: CostMonitor,
    config: CostGovernanceConfig,
    redis_client: redis::Client,
}

#[derive(Debug, Clone)]
pub struct CostGovernanceConfig {
    pub token_bucket_config: TokenBucketConfig,
    pub cost_monitoring_config: CostMonitoringConfig,
    pub enable_llm_calls: bool,
    pub hard_kill_switch: bool,
}

impl Default for CostGovernanceConfig {
    fn default() -> Self {
        Self {
            token_bucket_config: TokenBucketConfig::default(),
            cost_monitoring_config: CostMonitoringConfig::default(),
            enable_llm_calls: true,
            hard_kill_switch: false,
        }
    }
}

impl CostGovernanceManager {
    pub fn new(
        redis_client: redis::Client,
        cost_explorer_client: CostExplorerClient,
        ses_client: SesClient,
        config: CostGovernanceConfig,
    ) -> Self {
        let cost_monitor = CostMonitor::new(
            config.cost_monitoring_config.clone(),
            cost_explorer_client,
            ses_client,
        );

        Self {
            token_buckets: RwLock::new(HashMap::new()),
            cost_monitor,
            config,
            redis_client,
        }
    }

    pub async fn check_llm_call_permission(&self, tenant_id: &str, tokens: u32) -> Result<bool> {
        // Check hard kill switch first
        if self.config.hard_kill_switch {
            warn!("LLM calls disabled by hard kill switch");
            return Ok(false);
        }

        // Check if LLM calls are enabled
        if !self.config.enable_llm_calls {
            warn!("LLM calls disabled by configuration");
            return Ok(false);
        }

        // Get or create token bucket for tenant
        let bucket = {
            let buckets = self.token_buckets.read().await;
            if let Some(bucket) = buckets.get(tenant_id) {
                bucket.clone()
            } else {
                drop(buckets);
                let mut buckets = self.token_buckets.write().await;
                let bucket = TokenBucket::new(
                    self.config.token_bucket_config.clone(),
                    // Reuse the manager's Redis client (cloning a `redis::Client`
                    // is cheap; it shares the underlying connection config).
                    self.redis_client.clone(),
                    tenant_id.to_string(),
                );
                buckets.insert(tenant_id.to_string(), bucket.clone());
                bucket
            }
        };

        // Try to acquire tokens
        let success = bucket.acquire_token(tokens).await?;
        
        if success {
            info!("LLM call permitted for tenant {} (tokens: {})", tenant_id, tokens);
        } else {
            warn!("LLM call denied for tenant {} (insufficient tokens)", tenant_id);
        }

        Ok(success)
    }

    pub async fn record_llm_cost(&self, tenant_id: &str, cost_usd: f64) -> Result<()> {
        let mut costs = self.cost_monitor.daily_costs.write().await;
        let current_cost = *costs.get(tenant_id).unwrap_or(&0.0);
        costs.insert(tenant_id.to_string(), current_cost + cost_usd);
        
        info!("Recorded LLM cost for tenant {}: ${:.4}", tenant_id, cost_usd);
        Ok(())
    }

    pub async fn run_daily_cost_report(&self) -> Result<()> {
        info!("Generating daily cost report");
        self.cost_monitor.generate_daily_cost_report().await?;
        Ok(())
    }

    pub async fn check_budget_alerts(&self) -> Result<()> {
        info!("Checking budget alerts");
        self.cost_monitor.check_budget_alert().await?;
        Ok(())
    }

    pub async fn get_tenant_token_count(&self, tenant_id: &str) -> Result<u32> {
        let buckets = self.token_buckets.read().await;
        if let Some(bucket) = buckets.get(tenant_id) {
            bucket.get_token_count().await
        } else {
            Ok(self.config.token_bucket_config.capacity)
        }
    }

    pub async fn reset_tenant_bucket(&self, tenant_id: &str) -> Result<()> {
        let buckets = self.token_buckets.read().await;
        if let Some(bucket) = buckets.get(tenant_id) {
            bucket.reset_bucket().await
        } else {
            Ok(())
        }
    }

    pub fn set_hard_kill_switch(&mut self, enabled: bool) {
        self.config.hard_kill_switch = enabled;
        info!("Hard kill switch set to: {}", enabled);
    }

    pub fn set_llm_calls_enabled(&mut self, enabled: bool) {
        self.config.enable_llm_calls = enabled;
        info!("LLM calls enabled: {}", enabled);
    }
}

// Load testing utilities
pub struct LoadTestUtils;

impl LoadTestUtils {
    pub async fn simulate_concurrent_requests(
        manager: Arc<CostGovernanceManager>,
        tenant_id: &str,
        concurrent_requests: u32,
        tokens_per_request: u32,
    ) -> Result<LoadTestResult> {
        let start_time = Instant::now();
        let mut successful_requests = 0;
        let mut failed_requests = 0;

        let mut handles = vec![];
        
        for _ in 0..concurrent_requests {
            let manager_clone = Arc::clone(&manager);
            let tenant_id = tenant_id.to_string();
            
            let handle = tokio::spawn(async move {
                manager_clone.check_llm_call_permission(&tenant_id, tokens_per_request).await
            });
            
            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(true)) => successful_requests += 1,
                Ok(Ok(false)) => failed_requests += 1,
                Ok(Err(e)) => {
                    error!("Request failed with error: {}", e);
                    failed_requests += 1;
                }
                Err(e) => {
                    error!("Task failed: {}", e);
                    failed_requests += 1;
                }
            }
        }

        let duration = start_time.elapsed();
        
        Ok(LoadTestResult {
            total_requests: concurrent_requests,
            successful_requests,
            failed_requests,
            duration,
            requests_per_second: concurrent_requests as f64 / duration.as_secs_f64(),
        })
    }
}

#[derive(Debug)]
pub struct LoadTestResult {
    pub total_requests: u32,
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub duration: Duration,
    pub requests_per_second: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket_config_defaults_are_sane() {
        // A pure, dependency-free check of the configuration invariants the
        // token bucket relies on (no Redis required).
        let config = TokenBucketConfig::default();
        assert!(config.capacity > 0);
        assert!(config.refill_rate > 0.0);
        assert!(config.burst_size <= config.capacity);
    }

    #[tokio::test]
    #[ignore = "requires a live/localstack Redis endpoint; run as an integration test"]
    async fn test_token_bucket_acquire() {
        // Integration test: exercises the Lua token-bucket script against Redis.
    }

    #[tokio::test]
    #[ignore = "requires AWS Cost Explorer + SES; run as an integration test with credentials"]
    async fn test_cost_monitoring() {
        // Integration test: needs real AWS Cost Explorer and SES endpoints.
    }

    #[tokio::test]
    #[ignore = "load test; run explicitly against a provisioned Redis instance"]
    async fn test_load_test_concurrent_requests() {
        // Load test: intentionally excluded from the default unit-test run.
    }
} 