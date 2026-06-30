//! WebSocket 服务端 — 下游消费者接入（JSON 编码）
//!
//! 监听 `service.ws_listen_addr`（默认 0.0.0.0:8888），接受任意客户端连接。
//!
//! 路径与交互约定：
//! - 客户端发起 `GET /`（不带 path）
//! - 客户端连上后，服务端等待其发来的第一帧（文本），内容为期望订阅的 web_rid
//!   （兼容带上 `rooms/` 前缀，例如 `rooms/664637748606`）
//! - 服务端据此路由到 `DynamicRoomManager` 中对应房间的订阅频道
//! - 房间不存在时，服务端发一帧 JSON 错误并关闭连接
//! - 服务端持续推送 JSON 编码的 `BarrageEvent`
//!
//! 协议样例：
//! ```json
//! {
//!   "event_type": "ChatMessage",
//!   "data": { ... },
//!   "msg_id": 12345,
//!   "timestamp_ms": 1719475200000
//! }
//! ```
//!
//! 为什么不直接用 HTTP upgrade 路径？原因：tokio-tungstenite 的 `accept_hdr_async`
//! 回调没有 side-channel 传出 web_rid；目前采用"先连后报"模式，对调用方足够简单，
//! 标准 WebSocket 客户端均可使用。

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

use crate::dynamic_room::DynamicRoomManager;

/// WebSocket 服务端
pub struct WsServer {
    listen_addr: SocketAddr,
    rooms: Arc<DynamicRoomManager>,
}

impl WsServer {
    /// 创建 WS 服务端
    pub fn new(listen_addr: SocketAddr, rooms: Arc<DynamicRoomManager>) -> Self {
        Self { listen_addr, rooms }
    }

    /// 启动 WS 服务端（阻塞直到 shutdown）
    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(self.listen_addr)
            .await
            .with_context(|| format!("failed to bind WS server on {}", self.listen_addr))?;
        info!(addr = %self.listen_addr, "WS server listening");

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            debug!(peer = %peer_addr, "WS client connecting");

            let rooms = self.rooms.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, peer_addr, rooms).await {
                    warn!(peer = %peer_addr, error = %e, "WS client error");
                }
            });
        }
    }
}

async fn handle_client(
    stream: TcpStream,
    peer_addr: std::net::SocketAddr,
    rooms: Arc<DynamicRoomManager>,
) -> Result<()> {
    use tokio_tungstenite::accept_async;

    let ws_stream = accept_async(stream)
        .await
        .with_context(|| format!("WS handshake failed for {}", peer_addr))?;
    let (mut write, mut read) = ws_stream.split();

    // 等第一帧作为路由（兼容 `rooms/<id>` 或纯 `<id>`）
    let route = match read.next().await {
        Some(Ok(msg)) => match msg {
            tokio_tungstenite::tungstenite::Message::Text(text) => text,
            tokio_tungstenite::tungstenite::Message::Close(_) => return Ok(()),
            _ => {
                let _ = write
                    .send(tokio_tungstenite::tungstenite::Message::Close(None))
                    .await;
                return Ok(());
            }
        },
        _ => return Ok(()),
    };

    let web_rid: String = route
        .trim()
        .trim_start_matches('/')
        .trim_start_matches("rooms/")
        .to_string();
    if web_rid.is_empty() {
        let _ = write
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await;
        return Ok(());
    }

    let Some(sub) = rooms.subscribe(&web_rid) else {
        warn!(peer = %peer_addr, web_rid = %web_rid, "WS subscribe failed: room not found");
        let err_msg = format!(
            r#"{{"error":"room_not_found","room_id":"{}"}}"#,
            web_rid
        );
        let _ = write
            .send(tokio_tungstenite::tungstenite::Message::Text(err_msg))
            .await;
        let _ = write
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await;
        return Ok(());
    };

    let mut rx = sub.rx;
    info!(peer = %peer_addr, web_rid = %web_rid, "WS client subscribed to room");

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if let Err(e) = write
                            .send(tokio_tungstenite::tungstenite::Message::Text(json))
                            .await
                        {
                            error!(peer = %peer_addr, error = %e, "WS send failed");
                            break;
                        }
                    }
                    None => break,
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => break,
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data))) => {
                        if write
                            .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        warn!(peer = %peer_addr, error = %e, "WS read error");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    rooms.unsubscribe(&web_rid);
    info!(peer = %peer_addr, web_rid = %web_rid, "WS client disconnected");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_server_creation() {
        let mgr = Arc::new(DynamicRoomManager::new());
        let _server = WsServer::new("127.0.0.1:0".parse().unwrap(), mgr);
    }

    #[tokio::test]
    async fn ws_server_can_be_constructed_without_panic() {
        let mgr = Arc::new(DynamicRoomManager::new());
        let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = WsServer::new(listen, mgr);
        let _ = server;
    }
}
