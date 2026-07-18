use std::time::Duration;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    base_delay: Duration,
    max_delay: Duration,
    max_attempts: u32,
    multiplier: f64,
}

impl ExponentialBackoff {
    pub fn new() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_attempts: 5,
            multiplier: 2.0,
        }
    }

    pub fn with_base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }

    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        if attempt >= self.max_attempts {
            return self.max_delay;
        }

        let exponential_delay = self.base_delay.as_millis() as f64 * self.multiplier.powi(attempt as i32);
        let capped_delay = exponential_delay.min(self.max_delay.as_millis() as f64);
        
        // Add jitter (±25% of the calculated delay)
        let jitter_range = capped_delay * 0.25;
        let jitter = rand::thread_rng().gen_range(-jitter_range..jitter_range);
        let final_delay = (capped_delay + jitter).max(1.0); // Minimum 1ms

        Duration::from_millis(final_delay as u64)
    }

    pub async fn execute_with_backoff<F, T, E>(
        &self,
        mut operation: F,
    ) -> Result<T, E>
    where
        F: FnMut() -> Result<T, E>,
        E: std::fmt::Display,
    {
        let mut last_error = None;
        
        for attempt in 0..self.max_attempts {
            match operation() {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    
                    if attempt < self.max_attempts - 1 {
                        let delay = self.calculate_delay(attempt);
                        tracing::warn!(
                            "Operation failed (attempt {}/{}): {}. Retrying in {:?}",
                            attempt + 1,
                            self.max_attempts,
                            last_error.as_ref().unwrap(),
                            delay
                        );
                        
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }

    /// Retry an asynchronous, fallible operation with exponential backoff.
    ///
    /// `operation` is a closure returning a future each time it is invoked, so
    /// each attempt performs a fresh call (e.g. a new HTTP request). Retries
    /// stop as soon as the operation succeeds or the attempt budget is
    /// exhausted, in which case the last error is returned.
    pub async fn execute_with_backoff_async<F, Fut, T, E>(
        &self,
        mut operation: F,
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut last_error = None;

        for attempt in 0..self.max_attempts {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);

                    if attempt < self.max_attempts - 1 {
                        let delay = self.calculate_delay(attempt);
                        tracing::warn!(
                            "Async operation failed (attempt {}/{}): {}. Retrying in {:?}",
                            attempt + 1,
                            self.max_attempts,
                            last_error.as_ref().unwrap(),
                            delay
                        );

                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_exponential_backoff() {
        let backoff = ExponentialBackoff::new();
        
        // Test delay calculation
        let delay1 = backoff.calculate_delay(0);
        let delay2 = backoff.calculate_delay(1);
        let delay3 = backoff.calculate_delay(2);
        
        assert!(delay2 > delay1);
        assert!(delay3 > delay2);
        assert!(delay3 <= backoff.max_delay);
    }

    #[tokio::test]
    async fn test_execute_with_backoff_success() {
        let backoff = ExponentialBackoff::new();
        let counter = AtomicU32::new(0);
        
        let result = backoff
            .execute_with_backoff(|| {
                let current = counter.fetch_add(1, Ordering::SeqCst);
                if current < 2 {
                    Err("Simulated failure")
                } else {
                    Ok("Success")
                }
            })
            .await;
        
        assert_eq!(result, Ok("Success"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_execute_with_backoff_failure() {
        let backoff = ExponentialBackoff::new().with_max_attempts(3);
        
        let result = backoff
            .execute_with_backoff(|| Err::<String, _>("Always fails"))
            .await;
        
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Always fails");
    }

    #[tokio::test]
    async fn test_execute_with_backoff_async_retries_then_succeeds() {
        // Use a tight schedule so the test stays fast.
        let backoff = ExponentialBackoff::new()
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(2))
            .with_max_attempts(5);
        let counter = AtomicU32::new(0);

        let result: Result<&str, &str> = backoff
            .execute_with_backoff_async(|| {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt < 2 {
                        Err("transient")
                    } else {
                        Ok("ok")
                    }
                }
            })
            .await;

        assert_eq!(result, Ok("ok"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_execute_with_backoff_async_gives_up() {
        let backoff = ExponentialBackoff::new()
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(2))
            .with_max_attempts(3);

        let result: Result<(), &str> = backoff
            .execute_with_backoff_async(|| async { Err("always fails") })
            .await;

        assert_eq!(result, Err("always fails"));
    }
} 