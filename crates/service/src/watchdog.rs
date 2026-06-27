//! Watchdog 后台监控
//!
//! 参考原项目 `refactor(program): Watchdog后台线程 + 异常防御体系` (commit 1ca6107)
//!
//! 设计要点：
//! - 启动后台 tokio task
//! - 每 N 秒检查主任务心跳
//! - 主任务无响应超过 M 秒 → 记录告警（**不强制杀进程**，避免与原项目 commit 2af80cf 修复冲突）
//! - 优雅退出时自动清理

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing;

#[derive(Debug, Clone, Copy)]
struct Heartbeat {
    last_tick: Instant,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            last_tick: Instant::now(),
        }
    }
}

/// Watchdog
#[derive(Clone)]
pub struct Watchdog {
    heartbeat: Arc<Mutex<Heartbeat>>,
    check_interval: Duration,
    alert_threshold: Duration,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl std::fmt::Debug for Watchdog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watchdog")
            .field("check_interval", &self.check_interval)
            .field("alert_threshold", &self.alert_threshold)
            .finish()
    }
}

impl Watchdog {
    /// 创建 Watchdog
    ///
    /// - `check_interval`: 检查间隔（默认 30s）
    /// - `alert_threshold`: 主任务无响应阈值（默认 60s）
    pub fn new(check_interval: Duration, alert_threshold: Duration) -> Self {
        Self {
            heartbeat: Arc::new(Mutex::new(Heartbeat::default())),
            check_interval,
            alert_threshold,
            handle: Arc::new(Mutex::new(None)),
        }
    }

    /// 主任务调用以更新心跳
    pub fn tick(&self) {
        *self.heartbeat.lock() = Heartbeat {
            last_tick: Instant::now(),
        };
    }

    /// 启动后台监控任务
    pub fn start(&self) {
        let heartbeat = self.heartbeat.clone();
        let check_interval = self.check_interval;
        let alert_threshold = self.alert_threshold;

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(check_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                let elapsed = {
                    let hb = heartbeat.lock();
                    hb.last_tick.elapsed()
                };

                if elapsed > alert_threshold {
                    tracing::error!(
                        elapsed_secs = elapsed.as_secs(),
                        threshold_secs = alert_threshold.as_secs(),
                        "main task heartbeat stalled (watchdog alert)"
                    );
                } else if elapsed > alert_threshold / 2 {
                    tracing::warn!(
                        elapsed_secs = elapsed.as_secs(),
                        threshold_secs = alert_threshold.as_secs(),
                        "main task heartbeat slow"
                    );
                }
            }
        });

        *self.handle.lock() = Some(handle);
        tracing::info!(
            check_interval_secs = check_interval.as_secs(),
            alert_threshold_secs = alert_threshold.as_secs(),
            "watchdog started"
        );
    }

    /// 停止 Watchdog
    pub async fn stop(&self) {
        if let Some(handle) = self.handle.lock().take() {
            handle.abort();
            // 等待 task 终止（如果尚未终止）
            let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
            tracing::info!("watchdog stopped");
        }
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new(Duration::from_secs(30), Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn watchdog_tick_updates_heartbeat() {
        let watchdog = Watchdog::new(Duration::from_millis(100), Duration::from_secs(60));
        watchdog.tick();
        // 验证 heartbeat 已更新（通过内部状态）
        // 这里仅验证调用不 panic
    }

    #[tokio::test]
    async fn watchdog_start_and_stop() {
        let watchdog = Watchdog::new(Duration::from_millis(100), Duration::from_millis(500));
        watchdog.start();
        tokio::time::sleep(Duration::from_millis(50)).await;
        watchdog.stop().await;
    }
}