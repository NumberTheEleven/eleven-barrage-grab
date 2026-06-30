//! Room Manager — 单/多房间管理抽象
//!
//! # 设计
//! MVP 阶段实现 `SingleRoomManager`，管理一个直播间的完整生命周期。
//! 架构预留扩展点 `RoomManager` trait，便于后续实现 `MultiRoomManager`。

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use eleven_barrage_core::BarrageEvent;

use crate::api::RoomInfo;
use crate::fetch::FetchConnectionManager;
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

/// 上游连接类型
#[derive(Debug, Clone)]
pub enum RoomConnection {
    /// WSS push 连接
    Wss(WssConnectionManager),
    /// HTTP fetch fallback 连接
    Fetch(FetchConnectionManager),
}

/// 单房间管理器（MVP 默认实现）
pub struct SingleRoomManager {
    room_id: String,
    connection: RoomConnection,
    event_tx: mpsc::Sender<BarrageEvent>,
    event_rx: Option<mpsc::Receiver<BarrageEvent>>,
    task_handle: Option<JoinHandle<Result<()>>>,
}

impl std::fmt::Debug for SingleRoomManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleRoomManager")
            .field("room_id", &self.room_id)
            .field("connection", &self.connection)
            .finish()
    }
}

impl SingleRoomManager {
    fn with_connection(room_id: String, connection: RoomConnection) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        Self {
            room_id,
            connection,
            event_tx: tx,
            event_rx: Some(rx),
            task_handle: None,
        }
    }

    /// 创建 WSS 单房间管理器（向后兼容）
    pub fn new(room_id: String, wss: WssConnectionManager) -> Self {
        Self::with_connection(room_id, RoomConnection::Wss(wss))
    }

    /// 创建 HTTP fetch 单房间管理器
    pub fn new_fetch(room_id: String, fetch: FetchConnectionManager) -> Self {
        Self::with_connection(room_id, RoomConnection::Fetch(fetch))
    }

    /// 获取房间 ID
    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    /// 提取事件接收端（一次性，调用后不能再调用 event_stream）
    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<BarrageEvent>> {
        self.event_rx.take()
    }

    /// 在 tokio task 中启动房间拉流
    pub async fn start_in_task(&mut self,
        room_info: Option<RoomInfo>,
    ) -> Result<()> {
        let room_id = self.room_id.clone();
        let tx = self.event_tx.clone();
        let connection = self.connection.clone();

        let handle = tokio::spawn(async move {
            match connection {
                RoomConnection::Wss(wss) => wss
                    .run(room_id, room_info, tx)
                    .await
                    .context("WssConnectionManager.run failed"),
                RoomConnection::Fetch(fetch) => fetch
                    .run(room_id, room_info, tx)
                    .await
                    .context("FetchConnectionManager.run failed"),
            }
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

#[async_trait]
impl RoomManager for SingleRoomManager {
    async fn start(&self) -> Result<()> {
        // 实际启动通过 `start_in_task`（需要 &mut self）
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    fn event_stream(&self) -> Option<WssEventStream> {
        None // MVP：使用 mpsc channel 而非 wss stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WssConfig;

    #[test]
    fn single_room_manager_wss_new() {
        let wss = WssConnectionManager::new(WssConfig::default(), eleven_barrage_core::EventFilter::mvp_default());
        let manager = SingleRoomManager::new("123".into(), wss);
        assert_eq!(manager.room_id(), "123");
        assert!(matches!(manager.connection, RoomConnection::Wss(_)));
    }
}
