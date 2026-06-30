//! Real CDP client implementation (auto-signer spec section 3.3)

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::{
    connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

use crate::cdp::commands::{CdpCommand, CdpEvent, CdpResponse};
use crate::cdp::error::{CdpError, Result};
use crate::cdp::frame::{parse_message, ParsedCdpMessage};

pub use crate::cdp::mock::CdpTransport;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;
type WsRead = futures_util::stream::SplitStream<WsStream>;

#[derive(Clone)]
pub struct CdpClient {
    write: Arc<Mutex<WsSink>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<CdpResponse>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
    next_id: Arc<AtomicI64>,
    _read_task: Arc<JoinHandle<()>>,
}

impl CdpClient {
    /// Connect to a CDP WebSocket endpoint.
    /// Returns the client and a global event receiver (for events not tied to a session).
    pub async fn connect(
        ws_url: &str,
    ) -> Result<(Self, broadcast::Receiver<CdpEvent>)> {
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(CdpError::WebSocket)?;
        let (write, read) = ws_stream.split();

        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<CdpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel::<CdpEvent>(1024);

        let read_task = tokio::spawn(read_loop(read, pending.clone(), event_tx.clone()));

        Ok((
            Self {
                write: Arc::new(Mutex::new(write)),
                pending,
                event_tx: event_tx.clone(),
                next_id: Arc::new(AtomicI64::new(1)),
                _read_task: Arc::new(read_task),
            },
            event_tx.subscribe(),
        ))
    }

    /// Send a command and await its response with timeout.
    /// The closure receives a fresh id to embed in the command.
    pub async fn send<R: DeserializeOwned + Send + 'static>(
        &self,
        build: impl FnOnce(i64) -> CdpCommand,
        timeout: Duration,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cmd = build(id);
        self.send_raw(cmd, timeout).await
    }

    /// Subscribe to all events (no session filter — clients filter as needed).
    pub fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    /// Subscribe to events (currently forwards all events; session filtering is the caller's
    /// responsibility via AttachedToTarget / DetachedToTarget event matching).
    pub fn subscribe_session(
        &self,
        _session_id: &str,
    ) -> broadcast::Receiver<CdpEvent> {
        self.subscribe_events()
    }
}

#[async_trait]
impl CdpTransport for CdpClient {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        timeout: Duration,
    ) -> Result<R> {
        // Extract id from the command (assigned by `send`'s closure).
        // Do NOT re-assign here — that would orphan the pending waiter for the original id.
        let id = match &cmd {
            CdpCommand::SetDiscoverTargets { id, .. }
            | CdpCommand::SetAutoAttach { id, .. }
            | CdpCommand::CreateTarget { id, .. }
            | CdpCommand::AttachToTarget { id, .. }
            | CdpCommand::CloseTarget { id, .. }
            | CdpCommand::PageEnable { id, .. }
            | CdpCommand::PageNavigate { id, .. }
            | CdpCommand::NetworkEnable { id, .. }
            | CdpCommand::NetworkSetCookie { id, .. }
            | CdpCommand::NetworkGetAllCookies { id }
            | CdpCommand::NetworkGetCookies { id, .. }
            | CdpCommand::NetworkGetResponseBody { id, .. }
            | CdpCommand::NetworkDisable { id, .. }
            | CdpCommand::RuntimeEvaluate { id, .. }
            | CdpCommand::GetVersion { id } => *id,
        };
        let payload = serde_json::to_vec(&cmd)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut write = self.write.lock().await;
            let text = String::from_utf8(payload).expect("CDP payload is always valid UTF-8 JSON");
            write.send(Message::Text(text)).await.map_err(CdpError::WebSocket)?;
        }

        let resp = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| CdpError::Timeout(timeout))?
            .map_err(|_| CdpError::ConnectionClosed)?;

        if let Some(err) = resp.error {
            return Err(CdpError::Protocol {
                code: err.code,
                message: err.message,
            });
        }

        let result = resp.result.unwrap_or(Value::Null);
        let r: R = serde_json::from_value(result)?;
        Ok(r)
    }

    fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }
}

async fn read_loop(
    mut read: WsRead,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<CdpResponse>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
) {
    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        let parsed = match parse_message(msg) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CDP message");
                continue;
            }
        };
        match parsed {
            ParsedCdpMessage::Response(resp) => {
                let mut p = pending.lock().await;
                if let Some(tx) = p.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
            ParsedCdpMessage::Event(event) => {
                let _ = event_tx.send(event);
            }
            ParsedCdpMessage::Ignore => {}
        }
    }
}
