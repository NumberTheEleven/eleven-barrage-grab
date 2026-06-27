//! Room Manager — 单/多房间管理抽象
//!
//! # 设计
//! MVP 阶段实现 `SingleRoomManager`，管理一个直播间的完整生命周期。
//! 架构预留扩展点 `RoomManager` trait，便于后续实现 `MultiRoomManager`。
//!
//! # 多房间扩展点（R-019 架构预留）
//! ```ignore
//! pub trait RoomManager: Send + Sync {
//!     async fn start_all(&self) -> Result<()>;
//!     async fn add_room(&self, room_id: String) -> Result<RoomHandle>;
//!     async fn remove_room(&self, room_id: &str) -> Result<()>;
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use eleven_barrage_core::BarrageEvent;

use crate::api::RoomInfo;
use crate::wss::{WssConnectionManager, WssEventStream};

/// RoomManager trait — 预留多房间扩展（R-019）
#[async_trait]
pub trait RoomManager: Send + Sync {
    /// 启动房间拉流
    async fn start(&self) -> Result<()>;
    /// 停止房间拉流
    async fn stop(&self) -> Result<()>;
    /// 获取事件流（下游消费者接入）
    fn event_stream(&self) -> Option<WssEventStream>;
}

/// 单房间管理器（MVP 默认实现）
pub struct SingleRoomManager {
    room_id: String,
    wss: WssConnectionManager,
    event_tx: mpsc::Sender<BarrageEvent>,
    event_rx: Option<mpsc::Receiver<BarrageEvent>>,
    task_handle: Option<JoinHandle<Result<()>>>,
}

impl std::fmt::Debug for SingleRoomManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleRoomManager")
            .field("room_id", &self.room_id)
            .field("wss", &self.wss)
            .finish()
    }
}

impl SingleRoomManager {
    /// 创建单房间管理器
    pub fn new(room_id: String, wss: WssConnectionManager) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        Self {
            room_id,
            wss,
            event_tx: tx,
            event_rx: Some(rx),
            task_handle: None,
        }
    }

    /// 获取房间 ID
    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    /// 获取底层 wss 连接管理器
    pub fn wss(&self) -> &WssConnectionManager {
        &self.wss
    }

    /// 提取事件接收端（一次性，调用后不能再调用 event_stream）
    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<BarrageEvent>> {
        self.event_rx.take()
    }
}

#[async_trait]
impl RoomManager for SingleRoomManager {
    async fn start(&self) -> Result<()> {
        // 注：SingleRoomManager 不能直接 start（task_handle 需要 &mut self）
        // 实际启动通过 `start_owned` 或 `start_blocking` 方法
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    fn event_stream(&self) -> Option<WssEventStream> {
        None // MVP：使用 mpsc channel 而非 wss stream
    }
}

/// 单房间启动辅助方法
impl SingleRoomManager {
    /// 在 tokio task 中启动房间拉流
    pub async fn start_in_task(&mut self, room_info: Option<RoomInfo>) -> Result<()> {
        let room_id = self.room_id.clone();
        let wss = self.wss.clone();
        let tx = self.event_tx.clone();

        let handle = tokio::spawn(async move {
            wss.run(room_id, room_info, tx)
                .await
                .context("WssConnectionManager.run failed")
        });

        self.task_handle = Some(handle);
        Ok(())
    }

    /// 等待任务结束
    pub async fn join(&mut self) -> Result<()> {
        if let Some(handle) = self.task_handle.take() {
            handle.await.context("room task panicked")??;
        }
        Ok(())
    }
}