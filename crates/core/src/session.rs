//! Session 故障检测器
//!
//! 设计参考原项目 `WssBarrageGrab.cs:23-39, 161-176`：
//! - 连续 N 次 decoder 错误 → 触发 session 重连
//! - **不杀进程**（原项目 commit `2af80cf` 修复点：decoder 异常不再触发进程重启）
//! - 成功处理后错误计数清零
//!
//! 修复 commit 参考：
//! - `2af80cf fix(proxy): WebSocket decoder 异常不再触发进程重启`
//! - `7a83d7b feat(proxy): Decoder故障检测与自动恢复`

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// 连续 decoder 失败的最大次数（达到即触发 session fault）
const MAX_CONSECUTIVE_ERRORS: u32 = 5;

/// Session 故障检测器
#[derive(Debug, Clone)]
pub struct SessionFaultDetector {
    state: Arc<Inner>,
}

struct Inner {
    consecutive_errors: AtomicU32,
    fault_notify: Notify,
    #[allow(clippy::type_complexity)]
    fault_callback: Mutex<Option<Arc<dyn Fn(SessionFaultReason) + Send + Sync>>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("consecutive_errors", &self.consecutive_errors)
            .field("fault_notify", &"Notify")
            .field("fault_callback", &"<callback>")
            .finish()
    }
}

/// Session 故障原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionFaultReason {
    /// 连续 N 次 protobuf 解析失败
    ConsecutiveDecodeFailures(u32),
}

impl SessionFaultDetector {
    /// 创建新的故障检测器
    pub fn new() -> Self {
        Self {
            state: Arc::new(Inner {
                consecutive_errors: AtomicU32::new(0),
                fault_notify: Notify::new(),
                fault_callback: Mutex::new(None),
            }),
        }
    }

    /// 注册故障回调
    pub fn on_fault(&self, callback: Arc<dyn Fn(SessionFaultReason) + Send + Sync>) {
        *self.state.fault_callback.lock() = Some(callback);
    }

    /// 记录一次 decoder 错误
    ///
    /// - 错误次数达到阈值 → 触发 fault 通知并重置计数器
    /// - 否则仅增加计数
    pub fn record_error(&self) {
        let count = self.state.consecutive_errors.fetch_add(1, Ordering::SeqCst) + 1;

        if count >= MAX_CONSECUTIVE_ERRORS {
            self.trigger_fault(SessionFaultReason::ConsecutiveDecodeFailures(count));
        }
    }

    /// 记录一次成功处理（重置错误计数）
    pub fn record_success(&self) {
        self.state.consecutive_errors.store(0, Ordering::SeqCst);
    }

    /// 触发 session fault
    fn trigger_fault(&self, reason: SessionFaultReason) {
        // 重置计数器
        self.state.consecutive_errors.store(0, Ordering::SeqCst);

        // 通知等待者
        self.state.fault_notify.notify_waiters();

        // 调用回调
        if let Some(callback) = self.state.fault_callback.lock().clone() {
            callback(reason);
        }
    }

    /// 等待 fault 事件
    pub async fn wait_fault(&self) {
        self.state.fault_notify.notified().await
    }

    /// 当前连续错误数（调试用）
    pub fn current_error_count(&self) -> u32 {
        self.state.consecutive_errors.load(Ordering::SeqCst)
    }
}

impl Default for SessionFaultDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn single_error_no_fault() {
        let detector = SessionFaultDetector::new();
        detector.record_error();
        assert_eq!(detector.current_error_count(), 1);
    }

    #[test]
    fn consecutive_errors_trigger_fault() {
        let detector = SessionFaultDetector::new();
        let callback_count = Arc::new(AtomicU32::new(0));
        let counter = callback_count.clone();
        detector.on_fault(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));

        for _ in 0..MAX_CONSECUTIVE_ERRORS {
            detector.record_error();
        }

        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
        assert_eq!(detector.current_error_count(), 0); // 重置
    }

    #[test]
    fn success_resets_error_count() {
        let detector = SessionFaultDetector::new();
        detector.record_error();
        detector.record_error();
        detector.record_success();
        assert_eq!(detector.current_error_count(), 0);
    }

    #[tokio::test]
    async fn wait_fault_notified() {
        let detector = SessionFaultDetector::new();
        let detector_clone = detector.clone();

        let handle = tokio::spawn(async move {
            detector_clone.wait_fault().await;
            true
        });

        // 给一点时间让 task 进入 wait
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // 触发故障
        for _ in 0..MAX_CONSECUTIVE_ERRORS {
            detector.record_error();
        }

        let result = tokio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(result);
    }
}
