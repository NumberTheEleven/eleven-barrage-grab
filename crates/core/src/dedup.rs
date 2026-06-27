//! 消息去重器 — 基于 msgId 的环形缓冲
//!
//! 设计参考原项目 `WssBarrageGrab.cs:202-227` 的去重逻辑：
//! - 每种消息类型独立的环形缓冲
//! - 容量默认 300 条（与原项目一致）
//! - 命中已存在 msgId → 跳过；未命中 → 加入缓冲
//! - 缓冲满时移除最旧条目（FIFO）

use std::collections::{HashSet, VecDeque};

use parking_lot::Mutex;

/// 每种消息类型的环形缓冲容量
const DEFAULT_CAPACITY: usize = 300;

/// 消息去重器
///
/// # 线程安全
/// 内部使用 `parking_lot::Mutex` 保护共享状态，可在多线程环境安全使用。
#[derive(Debug)]
pub struct MsgDedup {
    /// 每种 method 独立的环形缓冲
    buffers: Mutex<HashSet<(String, i64)>>,
    /// 每个 method 的 FIFO 队列（用于容量控制）
    fifos: Mutex<std::collections::HashMap<String, VecDeque<i64>>>,
    capacity: usize,
}

impl Default for MsgDedup {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl MsgDedup {
    /// 创建去重器
    pub fn new(capacity: usize) -> Self {
        Self {
            buffers: Mutex::new(HashSet::with_capacity(capacity * 8)),
            fifos: Mutex::new(std::collections::HashMap::new()),
            capacity,
        }
    }

    /// 检查并标记 msgId
    ///
    /// # 返回
    /// - `true`：是**新**消息，应处理
    /// - `false`：是**重复**消息，应跳过
    pub fn check_and_mark(&self, method: &str, msg_id: i64) -> bool {
        let key = (method.to_string(), msg_id);
        {
            let mut buffers = self.buffers.lock();
            if buffers.contains(&key) {
                return false;
            }
            buffers.insert(key);
        }

        // FIFO 队列容量管理
        let mut fifos = self.fifos.lock();
        let fifo = fifos.entry(method.to_string()).or_insert_with(|| VecDeque::with_capacity(self.capacity));

        if fifo.len() >= self.capacity {
            // 移除最旧的
            if let Some(old) = fifo.pop_front() {
                let mut buffers = self.buffers.lock();
                buffers.remove(&(method.to_string(), old));
            }
        }
        fifo.push_back(msg_id);

        true
    }

    /// 当前缓冲大小（调试用）
    pub fn buffer_size(&self) -> usize {
        self.buffers.lock().len()
    }

    /// 清空所有缓冲
    pub fn clear(&self) {
        self.buffers.lock().clear();
        self.fifos.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_message_returns_true() {
        let dedup = MsgDedup::new(100);
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
    }

    #[test]
    fn duplicate_message_returns_false() {
        let dedup = MsgDedup::new(100);
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
        assert!(!dedup.check_and_mark("WebcastChatMessage", 1));
    }

    #[test]
    fn same_msg_id_different_methods_both_new() {
        let dedup = MsgDedup::new(100);
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
        assert!(dedup.check_and_mark("WebcastGiftMessage", 1));
    }

    #[test]
    fn ring_buffer_capacity_evicts_oldest() {
        let dedup = MsgDedup::new(3);
        // 插入 3 个达到容量上限
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
        assert!(dedup.check_and_mark("WebcastChatMessage", 2));
        assert!(dedup.check_and_mark("WebcastChatMessage", 3));
        // 插入第 4 个，evict msgId=1
        assert!(dedup.check_and_mark("WebcastChatMessage", 4));
        // msgId=1 现在可以重新被识别为新消息
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
    }

    #[test]
    fn buffer_size_reflects_state() {
        let dedup = MsgDedup::new(100);
        assert_eq!(dedup.buffer_size(), 0);
        dedup.check_and_mark("WebcastChatMessage", 1);
        assert_eq!(dedup.buffer_size(), 1);
    }

    #[test]
    fn clear_resets_state() {
        let dedup = MsgDedup::new(100);
        dedup.check_and_mark("WebcastChatMessage", 1);
        dedup.clear();
        assert_eq!(dedup.buffer_size(), 0);
        assert!(dedup.check_and_mark("WebcastChatMessage", 1));
    }
}