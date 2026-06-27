//! WebSocket 服务端 — 下游消费者接入（JSON 编码）
//!
//! 监听 `service.ws_listen_addr`（默认 0.0.0.0:8888）
//! 接受任意客户端连接，转发 BarrageEvent 为 JSON：
//!
//! ```json
//! {
//!   "event_type": "ChatMessage",
//!   "data": { ... },
//!   "msg_id": 12345,
//!   "timestamp_ms": 1719475200000
//! }
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, WebSocketStream};
use tracing::{debug, error, info, warn};

use eleven_barrage_core::BarrageEvent;

/// WS 服务端共享状态
type SharedClientMap = Arc<Mutex<Vec<mpsc::Sender<BarrageEvent>>>>;

/// WS 服务端
pub struct WsServer {
    listen_addr: SocketAddr,
    clients: SharedClientMap,
}

impl WsServer {
    /// 创建 WS 服务端
    pub fn new(listen_addr: SocketAddr) -> Self {
        Self {
            listen_addr,
            clients: Arc::new(Mutex::new(Vec::new())),
        }
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

            let clients = self.clients.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, peer_addr, clients).await {
                    warn!(peer = %peer_addr, error = %e, "WS client error");
                }
            });
        }
    }

    /// 获取事件分发器（用于上游注入事件）
    pub fn dispatcher(&self) -> EventDispatcher {
        EventDispatcher {
            clients: self.clients.clone(),
        }
    }
}

/// WS 事件分发器（上游调用 `dispatch` 推送给所有客户端）
pub struct EventDispatcher {
    clients: SharedClientMap,
}

impl EventDispatcher {
    /// 将 BarrageEvent 推送给所有客户端（JSON 序列化在 `handle_client` 中完成）
    pub async fn dispatch(&self, event: BarrageEvent) {
        let mut clients = self.clients.lock();
        let mut disconnected = Vec::new();

        for (idx, tx) in clients.iter().enumerate() {
            if tx.is_closed() {
                disconnected.push(idx);
                continue;
            }

            if let Err(e) = tx.try_send(event.clone()) {
                match e {
                    mpsc::error::TrySendError::Full(_) => {
                        warn!("client channel full, dropping event");
                    }
                    mpsc::error::TrySendError::Closed(_) => {
                        disconnected.push(idx);
                    }
                }
            }
        }

        // 清理断开的客户端
        for &idx in disconnected.iter().rev() {
            clients.remove(idx);
        }
    }

    /// 创建独立 mpsc 通道（用于接收上游事件）
    pub fn subscribe(&self) -> mpsc::Receiver<BarrageEvent> {
        let (tx, rx) = mpsc::channel(1024);
        self.clients.lock().push(tx);
        rx
    }
}

async fn handle_client(
    stream: TcpStream,
    peer_addr: std::net::SocketAddr,
    clients: SharedClientMap,
) -> Result<()> {
    let ws_stream: WebSocketStream<TcpStream> = accept_async(stream)
        .await
        .with_context(|| format!("WS handshake failed for {}", peer_addr))?;
    info!(peer = %peer_addr, "WS client connected");

    let (mut write, mut read) = ws_stream.split();

    // 注册客户端
    let (tx, mut rx) = mpsc::channel::<BarrageEvent>(1024);
    {
        clients.lock().push(tx);
    }

    loop {
        tokio::select! {
            // 接收上游事件 → 推送给客户端
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        let json = serde_json::to_string(&event)?;
                        if let Err(e) = write.send(Message::Text(json)).await {
                            error!(peer = %peer_addr, error = %e, "WS send failed");
                            break;
                        }
                    }
                    None => break, // 通道关闭
                }
            }
            // 接收客户端消息（处理 ping/pong 或 close）
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if write.send(Message::Pong(data)).await.is_err() {
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

    info!(peer = %peer_addr, "WS client disconnected");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_server_creation() {
        let server = WsServer::new("127.0.0.1:0".parse().unwrap());
        let _dispatcher = server.dispatcher();
        // 仅验证创建不 panic
    }

    #[tokio::test]
    async fn subscribe_and_dispatch() {
        let server = WsServer::new("127.0.0.1:0".parse().unwrap());
        let dispatcher = server.dispatcher();
        let mut rx = dispatcher.subscribe();

        let event = BarrageEvent::ChatMessage(
            eleven_barrage_core::ChatMessage {
                content: "hello".to_string(),
                ..Default::default()
            },
        );

        dispatcher.dispatch(event.clone()).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();

        if let BarrageEvent::ChatMessage(chat) = received {
            assert_eq!(chat.content, "hello");
        } else {
            panic!("expected ChatMessage");
        }
    }
}