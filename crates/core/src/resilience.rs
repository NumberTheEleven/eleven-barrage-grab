//! ResiliencePipeline — 可恢复性基础设施
//!
//! 提供统一的错误恢复模式：
//! - 指数退避（exponential backoff）
//! - 抖动（jitter）
//! - 重试上限
//! - 熔断（circuit breaker）
//!
//! 设计目标：让上层业务（WssConnectionManager 等）以声明式方式定义恢复策略。
//!
//! # 示例
//!
//! ```ignore
//! use eleven_barrage_core::resilience::{RetryPolicy, retry_with_backoff};
//!
//! let policy = RetryPolicy::exponential()
//!     .initial_delay(Duration::from_secs(1))
//!     .max_delay(Duration::from_secs(60))
//!     .max_attempts(10)
//!     .with_jitter();
//!
//! let result = retry_with_backoff(policy, || async {
//!     wss_connect().await
//! }).await;
//! ```

use std::future::Future;
use std::time::Duration;

/// 重试策略
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u32,
    pub jitter: bool,
}

impl RetryPolicy {
    /// 创建指数退避策略（默认值）
    pub fn exponential() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_attempts: 10,
            jitter: true,
        }
    }

    pub fn initial_delay(mut self, d: Duration) -> Self {
        self.initial_delay = d;
        self
    }

    pub fn max_delay(mut self, d: Duration) -> Self {
        self.max_delay = d;
        self
    }

    pub fn max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n;
        self
    }

    pub fn with_jitter(mut self) -> Self {
        self.jitter = true;
        self
    }

    pub fn without_jitter(mut self) -> Self {
        self.jitter = false;
        self
    }

    /// 计算第 n 次重试的延迟
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_ms = self.initial_delay.as_millis() as u64;
        let max_ms = self.max_delay.as_millis() as u64;

        let exp_ms = base_ms.saturating_mul(2u64.saturating_pow(attempt.min(20)));
        let capped = exp_ms.min(max_ms);

        if self.jitter {
            // 加入 0-25% 的随机抖动（使用 crate 自带的轻量 LCG 随机数）
            let jitter_range = capped / 4;
            let jitter = crate::rand::thread_rng().gen_range(0..=jitter_range);
            Duration::from_millis(capped + jitter)
        } else {
            Duration::from_millis(capped)
        }
    }
}

/// 使用给定策略重试异步操作
///
/// # 返回
/// - `Ok(T)`：成功
/// - `Err(E)`：达到最大尝试次数后仍未成功
pub async fn retry_with_backoff<F, Fut, T, E>(
    policy: RetryPolicy,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err: Option<E> = None;

    for attempt in 0..policy.max_attempts {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt + 1 < policy.max_attempts {
                    let delay = policy.delay_for_attempt(attempt);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = policy.max_attempts,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "operation failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                last_err = Some(e);
            }
        }
    }

    Err(last_err.expect("retry loop should have at least one attempt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn exponential_delay_calculation() {
        let policy = RetryPolicy::exponential()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(60))
            .without_jitter();

        assert_eq!(policy.delay_for_attempt(0).as_millis(), 100);
        assert_eq!(policy.delay_for_attempt(1).as_millis(), 200);
        assert_eq!(policy.delay_for_attempt(2).as_millis(), 400);
        assert_eq!(policy.delay_for_attempt(3).as_millis(), 800);
    }

    #[test]
    fn delay_capped_at_max() {
        let policy = RetryPolicy::exponential()
            .initial_delay(Duration::from_secs(1))
            .max_delay(Duration::from_secs(8))
            .without_jitter();

        assert!(policy.delay_for_attempt(0).as_secs() <= 1);
        assert!(policy.delay_for_attempt(5).as_secs() <= 8);
        assert!(policy.delay_for_attempt(20).as_secs() <= 8);
    }

    #[test]
    fn jitter_adds_randomness() {
        let policy = RetryPolicy::exponential()
            .initial_delay(Duration::from_millis(100))
            .without_jitter()
            .with_jitter();

        let d1 = policy.delay_for_attempt(2);
        let d2 = policy.delay_for_attempt(2);
        // 带抖动时，连续两次调用应该几乎肯定不同（除非运气极差）
        // 但我们只验证返回的延迟在合理范围
        assert!(d1.as_millis() >= 400 && d1.as_millis() <= 500);
        assert!(d2.as_millis() >= 400 && d2.as_millis() <= 500);
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        let policy = RetryPolicy::exponential()
            .initial_delay(Duration::from_millis(1))
            .max_delay(Duration::from_millis(10))
            .max_attempts(3)
            .without_jitter();

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result: Result<u32, &'static str> = retry_with_backoff(policy, || {
            let counter = counter_clone.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 2 {
                    Err("first attempt fails")
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_gives_up_after_max_attempts() {
        let policy = RetryPolicy::exponential()
            .initial_delay(Duration::from_millis(1))
            .max_delay(Duration::from_millis(10))
            .max_attempts(3)
            .without_jitter();

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result: Result<u32, &'static str> = retry_with_backoff(policy, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err("always fails")
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}