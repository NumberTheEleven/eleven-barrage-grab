//! HTTP fetch 上游连接管理器
//!
//! 职责：
//! 1. 持有自包含的 `FetchConsumer`，在浏览器/CDP 网络路径内拦截抖音 `webcast/im/fetch` 响应。
//! 2. 解码 + 分发 + 过滤 + 去重，与 `WssConnectionManager` 输出同构的 `BarrageEvent`。
//! 3. 将事件推送到同一个下游 `mpsc::Sender`。

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tracing::info;

use eleven_barrage_collector::fetch_consumer::{FetchConsumer, FetchConsumerConfig};
use eleven_barrage_core::{BarrageEvent, Dispatcher, EventFilter, MsgDedup, WssDecoder};

use crate::api::RoomInfo;

/// HTTP fetch 连接管理器
#[derive(Debug, Clone)]
pub struct FetchConnectionManager {
    config: FetchConsumerConfig,
    decoder: Arc<WssDecoder>,
    dispatcher: Arc<Dispatcher>,
    filter: Arc<EventFilter>,
    dedup: Arc<MsgDedup>,
}

impl FetchConnectionManager {
    /// 创建新连接管理器
    pub fn new(config: FetchConsumerConfig, filter: EventFilter) -> Self {
        Self {
            config,
            decoder: Arc::new(WssDecoder::new()),
            dispatcher: Arc::new(Dispatcher::new()),
            filter: Arc::new(filter),
            dedup: Arc::new(MsgDedup::default()),
        }
    }

    /// 运行主循环（启动 FetchConsumer 并泵事件到下游）
    ///
    /// 该方法会一直阻塞直到 `event_tx` 关闭或浏览器/CDP 会话失败。
    pub async fn run(
        &self,
        room_id: String,
        _room_info: Option<RoomInfo>,
        event_tx: mpsc::Sender<BarrageEvent>,
    ) -> Result<()> {
        info!(room_id = %room_id, "fetch consumer starting");
        let consumer = FetchConsumer::new(self.config.clone());
        consumer
            .run(event_tx, (*self.filter).clone())
            .await
            .context("FetchConsumer.run failed")
    }

    /// 获取 decoder（用于监控）
    pub fn decoder(&self) -> &WssDecoder {
        &self.decoder
    }

    /// 获取 dispatcher（用于监控）
    pub fn dispatcher(&self) -> &Dispatcher {
        &self.dispatcher
    }

    /// 获取去重器（用于监控）
    pub fn dedup(&self) -> &MsgDedup {
        &self.dedup
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn fetch_manager_new() {
        let config = FetchConsumerConfig {
            edge_path: PathBuf::from("/usr/bin/chromium"),
            user_data_dir: PathBuf::from("/tmp/fetch-test"),
            cdp_port: 9222,
            extra_args: vec![],
            web_rid: "123".into(),
            auth_cookies: Default::default(),
            keepalive_interval: Duration::from_secs(5),
            navigation_timeout: Duration::from_secs(30),
        };
        let manager = FetchConnectionManager::new(config, EventFilter::mvp_default());
        assert_eq!(manager.config.web_rid, "123");
    }
}
