//! WSS 上游连接管理器
//!
//! 职责：
//! 1. 维护到抖音 wss 端点的长连接（tokio-tungstenite）
//! 2. 接收 binary frame，解码 + 分发 + 过滤 + 去重
//! 3. 5s 心跳保底（参考原项目 commit 85d9514）
//! 4. 指数退避自动重连（参考原项目 commit 系列）
//! 5. decoder 故障检测 + session 重连（参考原项目 commit 7a83d7b）
//!
//! # 数据流
//! ```text
//! tungstenite::Message::Binary
//!     → WssDecoder::decode()
//!     → Dispatcher::dispatch()
//!     → EventFilter::filter()
//!     → MsgDedup::check_and_mark()
//!     → mpsc::Sender<BarrageEvent> 推送给下游
//! ```

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

use eleven_barrage_core::{
    BarrageEvent, Dispatcher, EventFilter, MsgDedup, SessionFaultDetector, WssDecoder,
};

use crate::api::RoomInfo;
use crate::config::WssConfig;
use crate::metrics::{record, WssState};

/// WSS 事件流类型别名（用于多房间架构扩展）
pub type WssEventStream = mpsc::Receiver<BarrageEvent>;

/// WSS 连接管理器
#[derive(Debug, Clone)]
pub struct WssConnectionManager {
    config: WssConfig,
    decoder: Arc<WssDecoder>,
    dispatcher: Arc<Dispatcher>,
    filter: Arc<EventFilter>,
    dedup: Arc<MsgDedup>,
    fault_detector: SessionFaultDetector,
}

impl WssConnectionManager {
    /// 创建新连接管理器
    pub fn new(config: WssConfig, filter: EventFilter) -> Self {
        Self {
            config,
            decoder: Arc::new(WssDecoder::new()),
            dispatcher: Arc::new(Dispatcher::new()),
            filter: Arc::new(filter),
            dedup: Arc::new(MsgDedup::default()),
            fault_detector: SessionFaultDetector::new(),
        }
    }

    /// 运行主循环（长连接 + 重连）
    ///
    /// 这是入口方法，会一直阻塞直到 `event_tx` 被关闭或达到最大重连次数。
    ///
    /// # 参数
    /// - `room_id`: 房间标识（用于日志）
    /// - `_room_info`: 房间元数据（可选，用于日志/调试）
    /// - `event_tx`: 事件发送端（推送给下游 WS/gRPC servers）
    pub async fn run(
        &self,
        room_id: String,
        _room_info: Option<RoomInfo>,
        event_tx: mpsc::Sender<BarrageEvent>,
    ) -> Result<()> {
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_reconnect_attempts;

        loop {
            // 检查是否达到最大重连次数（0 = 无限）
            if max_attempts > 0 && attempt >= max_attempts {
                anyhow::bail!(
                    "max reconnect attempts ({}) reached for room {}",
                    max_attempts,
                    room_id
                );
            }

            // 更新 metrics
            record::wss_state(&room_id, WssState::Connecting);

            // 尝试连接
            match self.connect_and_process(&room_id, &event_tx).await {
                Ok(()) => {
                    info!(room_id = %room_id, "wss connection closed cleanly");
                }
                Err(e) => {
                    error!(
                        room_id = %room_id,
                        attempt = attempt + 1,
                        error = %e,
                        "wss connection error"
                    );
                    record::reconnect("error");
                }
            }

            attempt += 1;
            record::wss_state(&room_id, WssState::Disconnected);

            // 计算退避延迟
            let delay = self.backoff_delay(attempt);
            warn!(
                room_id = %room_id,
                delay_secs = delay.as_secs(),
                "reconnecting after backoff"
            );
            tokio::time::sleep(delay).await;

            // 检查下游是否已关闭（如果 event_tx 关闭则退出）
            if event_tx.is_closed() {
                info!(
                    room_id = %room_id,
                    "downstream closed, exiting reconnect loop"
                );
                return Ok(());
            }
        }
    }

    /// 单次连接 + 处理循环
    async fn connect_and_process(
        &self,
        room_id: &str,
        event_tx: &mpsc::Sender<BarrageEvent>,
    ) -> Result<()> {
        if self.config.url.is_empty() {
            anyhow::bail!(
                "wss.url is empty. Configure via [wss] section, \
                 ELEVEN_BARRAGE_WSS_URL env var, or provide via collector (R-011/R-012)."
            );
        }

        // 构建 request（tungstenite 0.23 的 Request 类型 = http::Request<()>）
        // 注：本版本 tungstenite 的 Builder::header() 不可失败
        let mut request_builder = tokio_tungstenite::tungstenite::http::Request::builder()
            .method("GET")
            .uri(&self.config.url);

        for (key, value) in &self.config.headers {
            request_builder = request_builder.header(key.as_str(), value.as_str());
        }

        let request = request_builder
            .body(())
            .expect("failed to build wss request");

        let (ws_stream, response) = connect_async(request)
            .await
            .context("failed to connect to wss")?;

        info!(
            room_id = %room_id,
            status = ?response.status(),
            "wss connected"
        );
        record::wss_state(room_id, WssState::Connected);

        let (mut write, mut read) = ws_stream.split();

        // SplitSink 不实现 Clone，需要 Arc<Mutex<>> 包装以在心跳任务与主循环间共享
        let shared_write = Arc::new(tokio::sync::Mutex::new(write));

        // 启动心跳任务
        let heartbeat_handle = self.spawn_heartbeat(room_id, shared_write.clone());

        // 主消息循环
        loop {
            tokio::select! {
                // 接收下游关闭信号（通过 fault detector）
                _ = self.fault_detector.wait_fault() => {
                    warn!(room_id = %room_id, "session fault detected, triggering reconnect");
                    heartbeat_handle.abort();
                    return Err(anyhow::anyhow!("session fault"));
                }
                // 接收 wss 消息
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Binary(frame))) => {
                            self.handle_binary_frame(room_id, &frame, event_tx).await;
                        }
                        Some(Ok(Message::Text(text))) => {
                            debug!(room_id = %room_id, text = %text, "received text frame");
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!(room_id = %room_id, "wss closed by server");
                            heartbeat_handle.abort();
                            return Ok(());
                        }
                        Some(Ok(Message::Ping(data))) => {
                            debug!(room_id = %room_id, len = data.len(), "received ping");
                            // 业务库会自动响应 pong，无需手动处理
                        }
                        Some(Ok(_)) => {
                            // Pong / Frame 等其他类型
                        }
                        Some(Err(e)) => {
                            error!(room_id = %room_id, error = %e, "wss read error");
                            heartbeat_handle.abort();
                            return Err(e.into());
                        }
                        None => {
                            info!(room_id = %room_id, "wss stream ended");
                            heartbeat_handle.abort();
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// 处理单个二进制帧（解码 + 分发 + 过滤 + 去重 + 推送）
    async fn handle_binary_frame(
        &self,
        room_id: &str,
        frame: &[u8],
        event_tx: &mpsc::Sender<BarrageEvent>,
    ) {
        let start = std::time::Instant::now();

        // 1. 解码
        let (wss_response, response) = match self.decoder.decode(frame, false) {
            Ok(r) => {
                self.fault_detector.record_success();
                record::decode_success();
                r
            }
            Err(e) => {
                use eleven_barrage_core::CoreError;
                match &e {
                    CoreError::InvalidWireType(_) => {
                        // 非 protobuf 数据（如 WebSocket 控制帧误投），不计入错误
                        debug!(room_id = %room_id, error = %e, "non-protobuf frame skipped");
                    }
                    _ => {
                        error!(room_id = %room_id, error = %e, "decode error");
                        record::decode_error("decode");
                        self.fault_detector.record_error();
                    }
                }
                return;
            }
        };

        // 2. 分发
        let events = match self.dispatcher.dispatch(&wss_response, &response) {
            Ok(events) => events,
            Err(e) => {
                error!(room_id = %room_id, error = %e, "dispatch error");
                return;
            }
        };

        // 3. 过滤 + 4. 去重 + 5. 推送
        for event in events {
            if !self.filter.allows(&event) {
                continue;
            }

            if !self.dedup.check_and_mark(event.method(), event.msg_id()) {
                debug!(
                    room_id = %room_id,
                    method = %event.method(),
                    msg_id = event.msg_id(),
                    "duplicate event, skipped"
                );
                continue;
            }

            let duration = start.elapsed().as_secs_f64();
            record::event_processed(event.method(), duration);

            if let Err(e) = event_tx.send(event).await {
                error!(
                    room_id = %room_id,
                    error = %e,
                    "failed to send event to downstream"
                );
                break;
            }
        }
    }

    /// 启动心跳任务（5s 保底）
    ///
    /// 参考原项目 commit `85d9514 fix(proxy): 消除可避免的进程崩溃源 + 5s 心跳保底`
    fn spawn_heartbeat(
        &self,
        room_id: &str,
        write: Arc<
            tokio::sync::Mutex<
                futures::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
            >,
        >,
    ) -> tokio::task::JoinHandle<()> {
        let interval_secs = self.config.heartbeat_interval_secs;
        let room_id = room_id.to_string();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));
            // 第一次 tick 立即触发（让心跳与连接建立尽量同步）
            ticker.tick().await;

            loop {
                ticker.tick().await;

                let ping = Message::Ping(Vec::new());
                match write.lock().await.send(ping).await {
                    Ok(()) => {
                        record::heartbeat_success(&room_id);
                        debug!(room_id = %room_id, "heartbeat sent");
                    }
                    Err(e) => {
                        record::heartbeat_failure(&room_id);
                        error!(room_id = %room_id, error = %e, "heartbeat send failed");
                        break;
                    }
                }
            }
        })
    }

    /// 计算重连退避延迟
    fn backoff_delay(&self, attempt: u32) -> Duration {
        let initial_secs = self.config.reconnect_initial_secs;
        let max_secs = self.config.reconnect_max_secs;
        let exp_secs = initial_secs.saturating_mul(2u64.saturating_pow(attempt.min(20)));
        let capped = exp_secs.min(max_secs);
        Duration::from_secs(capped)
    }

    /// 触发 session fault（供外部调用，例如在 dispatcher 错误时）
    pub fn trigger_session_fault(&self) {
        // 故意触发连续错误达到阈值
        for _ in 0..6 {
            self.fault_detector.record_error();
        }
    }

    /// 获取 decoder 故障检测器（用于监控）
    pub fn fault_detector(&self) -> &SessionFaultDetector {
        &self.fault_detector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_exponential() {
        let config = WssConfig {
            reconnect_initial_secs: 1,
            reconnect_max_secs: 60,
            ..Default::default()
        };
        let manager = WssConnectionManager::new(config, EventFilter::mvp_default());

        assert_eq!(manager.backoff_delay(0).as_secs(), 1);
        assert_eq!(manager.backoff_delay(1).as_secs(), 2);
        assert_eq!(manager.backoff_delay(2).as_secs(), 4);
        assert_eq!(manager.backoff_delay(3).as_secs(), 8);
        // 封顶
        assert_eq!(manager.backoff_delay(20).as_secs(), 60);
    }
}
