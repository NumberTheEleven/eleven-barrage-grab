//! 动态房间管理器（DynamicRoomManager）
//!
//! 负责管理与外部 URL 对应的房间生命周期：
//! - `create_or_get`：create-or-get 语义，相同 web_rid 共享同一份房间
//! - `destroy`：停止采集并清理资源
//! - `list`：列出活跃房间
//! - `subscribe`/`unsubscribe`：管理下游 WS 客户端订阅
//!
//! # 设计要点
//!
//! - 所有内部状态都在一个 `Mutex` 下协调；考虑房间数量很小（通常 < 100），性能不构成瓶颈
//! - 客户端订阅用 `parking_lot::Mutex<Vec<mpsc::Sender<BarrageEvent>>>`，每个房间独立
//! - 客户端数量计数用 `AtomicUsize`，避免热点锁
//! - 房间状态用 `AtomicU8`，允许无锁读取
//!
//! 与现有 `SingleRoomManager` 的区别：
//! - 多房间并存
//! - 房间通过 web_rid 索引
//! - 显式 create / destroy，不随服务启动自动创建

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use eleven_barrage_core::BarrageEvent;

/// 房间状态（`AtomicU8`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomStatus {
    Connecting,
    Connected,
    Failed,
}

impl RoomStatus {
    fn as_u8(self) -> u8 {
        match self {
            RoomStatus::Connecting => 0,
            RoomStatus::Connected => 1,
            RoomStatus::Failed => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => RoomStatus::Connected,
            2 => RoomStatus::Failed,
            _ => RoomStatus::Connecting,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RoomStatus::Connecting => "connecting",
            RoomStatus::Connected => "connected",
            RoomStatus::Failed => "failed",
        }
    }
}

/// 房间订阅者句柄
#[derive(Debug)]
pub struct SubscriberHandle {
    /// 当前订阅者所属房间 ID（web_rid）
    pub room_id: String,
    /// 用以接收 BarrageEvent 的 mpsc Receiver
    pub rx: mpsc::Receiver<BarrageEvent>,
}

/// 房间内部状态（持有在 RoomManager 的 HashMap 中）
pub struct RoomHandle {
    web_rid: String,
    url: String,
    status: AtomicU8,
    created_at_unix: i64,
    /// 订阅此房间的下游发送端
    subscribers: Mutex<Vec<mpsc::Sender<BarrageEvent>>>,
    /// 客户端订阅者数量（`AtomicUsize` 减少锁竞争）
    client_count: AtomicUsize,
    /// collector 任务句柄（destroy 时用于清理）
    collector_handle: Mutex<Option<JoinHandle<()>>>,
}

/// 房间信息快照（对外暴露）
#[derive(Debug, Clone)]
pub struct RoomSnapshot {
    pub room_id: String,
    pub url: String,
    pub status: RoomStatus,
    pub client_count: usize,
    pub created_at_unix: i64,
}

#[derive(Debug, Default)]
pub struct RoomManagerError;

impl std::fmt::Display for RoomManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("room manager error")
    }
}

impl std::error::Error for RoomManagerError {}

/// 动态房间管理器
pub struct DynamicRoomManager {
    rooms: Mutex<HashMap<String, Arc<RoomHandle>>>,
}

impl std::fmt::Debug for DynamicRoomManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicRoomManager")
            .field("rooms_count", &self.rooms.lock().len())
            .finish()
    }
}

impl DynamicRoomManager {
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
        }
    }

    /// create-or-get：根据 web_rid 创建或返回现有房间。
    /// `on_create` 只在房间是新建时调用，用于启动 collector 任务。
    pub fn create_or_get<F>(
        &self,
        web_rid: &str,
        url: &str,
        on_create: F,
    ) -> Arc<RoomHandle>
    where
        F: FnOnce(Arc<RoomHandle>),
    {
        let mut rooms = self.rooms.lock();
        if let Some(existing) = rooms.get(web_rid) {
            return existing.clone();
        }

        let handle = Arc::new(RoomHandle {
            web_rid: web_rid.to_string(),
            url: url.to_string(),
            status: AtomicU8::new(RoomStatus::Connecting.as_u8()),
            created_at_unix: now_unix(),
            subscribers: Mutex::new(Vec::new()),
            client_count: AtomicUsize::new(0),
            collector_handle: Mutex::new(None),
        });
        rooms.insert(web_rid.to_string(), handle.clone());
        drop(rooms);

        on_create(handle.clone());
        handle
    }

    /// 根据 web_rid 获取房间（不创建）
    pub fn get(&self, web_rid: &str) -> Option<Arc<RoomHandle>> {
        self.rooms.lock().get(web_rid).cloned()
    }

    /// 销毁房间：先停止 collector，然后从 map 中移除。
    /// `stop_collector`：调用方负责实际停止后台任务（通过关闭事件 channel 或 abort task）
    pub fn destroy<F>(&self, web_rid: &str, stop_collector: F) -> Result<(), RoomManagerError>
    where
        F: FnOnce(Arc<RoomHandle>),
    {
        let handle = match self.rooms.lock().remove(web_rid) {
            Some(h) => h,
            None => return Err(RoomManagerError),
        };

        stop_collector(handle.clone());

        // 关闭订阅者 channel 以断开下游 WS
        handle.subscribers.lock().clear();
        *handle.collector_handle.lock() = None;

        Ok(())
    }

    /// 列出所有房间的快照
    pub fn list(&self) -> Vec<RoomSnapshot> {
        let rooms = self.rooms.lock();
        rooms
            .values()
            .map(|h| RoomSnapshot {
                room_id: h.web_rid.clone(),
                url: h.url.clone(),
                status: RoomStatus::from_u8(h.status.load(Ordering::Relaxed)),
                client_count: h.client_count.load(Ordering::Relaxed),
                created_at_unix: h.created_at_unix,
            })
            .collect()
    }

    /// 订阅某房间，获得 mpsc Receiver
    pub fn subscribe(&self, web_rid: &str) -> Option<SubscriberHandle> {
        let handle = self.get(web_rid)?;
        let (tx, rx) = mpsc::channel(1024);
        handle.subscribers.lock().push(tx);
        handle.client_count.fetch_add(1, Ordering::Relaxed);
        Some(SubscriberHandle {
            room_id: handle.web_rid.clone(),
            rx,
        })
    }

    /// 取消订阅（仅关闭此客户端的接收端，不影响其他订阅者和采集）
    pub fn unsubscribe(&self, web_rid: &str) {
        if let Some(handle) = self.get(web_rid) {
            // 减少计数（带饱和处理）
            let _ = handle.client_count.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |v| Some(v.saturating_sub(1)),
            );
        }
    }

    /// 将事件推送给某房间的所有订阅者
    pub fn dispatch(&self, web_rid: &str, event: BarrageEvent) {
        let Some(handle) = self.get(web_rid) else {
            return;
        };
        let subs = handle.subscribers.lock();
        for tx in subs.iter() {
            if tx.is_closed() {
                continue;
            }
            // 容量满则丢弃事件，避免阻塞采集任务
            let _ = tx.try_send(event.clone());
        }
    }

    /// 设置房间状态
    pub fn set_status(&self, web_rid: &str, status: RoomStatus) {
        if let Some(handle) = self.get(web_rid) {
            handle.status.store(status.as_u8(), Ordering::Relaxed);
        }
    }
}

impl RoomHandle {
    pub fn web_rid(&self) -> &str {
        &self.web_rid
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn status(&self) -> RoomStatus {
        RoomStatus::from_u8(self.status.load(Ordering::Relaxed))
    }

    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::Relaxed)
    }

    pub fn subscribers(&self) -> &Mutex<Vec<mpsc::Sender<BarrageEvent>>> {
        &self.subscribers
    }

    pub fn collector_handle_slot(&self) -> &Mutex<Option<JoinHandle<()>>> {
        &self.collector_handle
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eleven_barrage_core::ChatMessage;

    fn make_event() -> BarrageEvent {
        BarrageEvent::ChatMessage(ChatMessage {
            content: "hi".into(),
            ..Default::default()
        })
    }

    #[test]
    fn create_or_get_returns_same_handle_for_same_web_rid() {
        let mgr = DynamicRoomManager::new();
        let a1 = mgr.create_or_get("rid1", "https://x", |_| {});
        let a2 = mgr.create_or_get("rid1", "https://x", |_| {});
        assert_eq!(a1.web_rid, "rid1");
        assert_eq!(Arc::as_ptr(&a1), Arc::as_ptr(&a2));
    }

    #[test]
    fn create_or_get_different_web_rid_creates_new() {
        let mgr = DynamicRoomManager::new();
        let _ = mgr.create_or_get("rid1", "u1", |_| {});
        let _ = mgr.create_or_get("rid2", "u2", |_| {});
        assert_eq!(mgr.list().len(), 2);
    }

    #[test]
    fn destroy_existing_room_returns_ok() {
        let mgr = DynamicRoomManager::new();
        mgr.create_or_get("rid1", "u", |_| {});
        let result = mgr.destroy("rid1", |_| {});
        assert!(result.is_ok());
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn destroy_nonexistent_room_returns_err() {
        let mgr = DynamicRoomManager::new();
        let result = mgr.destroy("nope", |_| {});
        assert!(result.is_err());
    }

    #[test]
    fn subscribe_returns_handle_with_correct_room_id() {
        let mgr = DynamicRoomManager::new();
        mgr.create_or_get("rid1", "u", |_| {});
        let sub = mgr.subscribe("rid1").expect("subscribe");
        assert_eq!(sub.room_id, "rid1");
        // unsubscribe 后 client_count 减一
        mgr.unsubscribe("rid1");
        let snap = mgr.list();
        assert_eq!(snap[0].client_count, 0);
    }

    #[test]
    fn subscribe_to_nonexistent_returns_none() {
        let mgr = DynamicRoomManager::new();
        assert!(mgr.subscribe("nope").is_none());
    }

    #[tokio::test]
    async fn dispatch_sends_event_to_subscribers() {
        let mgr = DynamicRoomManager::new();
        mgr.create_or_get("rid1", "u", |_| {});
        let mut sub = mgr.subscribe("rid1").expect("subscribe");

        mgr.dispatch("rid1", make_event());
        let event = sub
            .rx
            .recv()
            .await
            .expect("should receive dispatched event");
        if let BarrageEvent::ChatMessage(chat) = event {
            assert_eq!(chat.content, "hi");
        } else {
            panic!("expected ChatMessage");
        }
    }

    #[test]
    fn on_create_callback_fires_only_for_new_rooms() {
        let mgr = DynamicRoomManager::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = counter.clone();
        mgr.create_or_get("rid1", "u", move |_| {
            c1.fetch_add(1, Ordering::Relaxed);
        });
        let c2 = counter.clone();
        mgr.create_or_get("rid1", "u", move |_| {
            c2.fetch_add(1, Ordering::Relaxed);
        });
        let c3 = counter.clone();
        mgr.create_or_get("rid2", "u", move |_| {
            c3.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn set_status_then_snapshot() {
        let mgr = DynamicRoomManager::new();
        mgr.create_or_get("rid1", "u", |_| {});
        mgr.set_status("rid1", RoomStatus::Connected);
        let snap = mgr.list();
        assert_eq!(snap[0].status, RoomStatus::Connected);
        mgr.set_status("rid1", RoomStatus::Failed);
        let snap = mgr.list();
        assert_eq!(snap[0].status, RoomStatus::Failed);
    }
}
