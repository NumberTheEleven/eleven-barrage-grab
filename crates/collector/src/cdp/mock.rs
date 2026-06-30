//! Mock CdpTransport for unit tests (auto-signer spec section 6.5)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{broadcast, Mutex};

use crate::cdp::commands::{CdpCommand, CdpEvent};
use crate::cdp::error::{CdpError, Result};

/// Transport abstraction — both real CdpClient and MockCdpClient implement this.
#[async_trait]
pub trait CdpTransport: Send + Sync {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        timeout: Duration,
    ) -> Result<R>;

    fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent>;
}

/// In-memory mock that returns canned responses and replays pre-recorded events.
#[derive(Clone)]
pub struct MockCdpClient {
    inner: Arc<MockInner>,
}

struct MockInner {
    responses: Mutex<HashMap<String, Value>>,
    event_tx: broadcast::Sender<CdpEvent>,
}

impl MockCdpClient {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel::<CdpEvent>(256);
        Self {
            inner: Arc::new(MockInner {
                responses: Mutex::new(HashMap::new()),
                event_tx,
            }),
        }
    }

    /// Register a canned response for a given CDP method name (wire format, e.g., "Target.createTarget").
    pub async fn enqueue_response(&self, method: &str, response: Value) {
        self.inner.responses.lock().await.insert(method.to_string(), response);
    }

    /// Enqueue an event to be broadcast on `subscribe_events()`.
    pub async fn enqueue_event(&self, event: CdpEvent) {
        let _ = self.inner.event_tx.send(event);
    }
}

impl Default for MockCdpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CdpTransport for MockCdpClient {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        _timeout: Duration,
    ) -> Result<R> {
        let method = match &cmd {
            CdpCommand::SetDiscoverTargets { .. } => "Target.setDiscoverTargets",
            CdpCommand::SetAutoAttach { .. } => "Target.setAutoAttach",
            CdpCommand::CreateTarget { .. } => "Target.createTarget",
            CdpCommand::AttachToTarget { .. } => "Target.attachToTarget",
            CdpCommand::CloseTarget { .. } => "Target.closeTarget",
            CdpCommand::PageEnable { .. } => "Page.enable",
            CdpCommand::PageNavigate { .. } => "Page.navigate",
            CdpCommand::NetworkEnable { .. } => "Network.enable",
            CdpCommand::NetworkSetCookie { .. } => "Network.setCookie",
            CdpCommand::NetworkGetAllCookies { .. } => "Network.getAllCookies",
            CdpCommand::NetworkGetCookies { .. } => "Network.getCookies",
            CdpCommand::NetworkGetResponseBody { .. } => "Network.getResponseBody",
            CdpCommand::NetworkDisable { .. } => "Network.disable",
            CdpCommand::RuntimeEvaluate { .. } => "Runtime.evaluate",
            CdpCommand::GetVersion { .. } => "Browser.getVersion",
        };
        let responses = self.inner.responses.lock().await;
        let value = responses
            .get(method)
            .ok_or_else(|| CdpError::Protocol {
                code: -1,
                message: format!("no canned response for {}", method),
            })?
            .clone();
        drop(responses);
        let r: R = serde_json::from_value(value)?;
        Ok(r)
    }

    fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.event_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::commands::{CreateTargetParams, CreateTargetResult, CdpEvent, LoadEventFiredParams, Request, RequestWillBeSentParams};
    use crate::cdp::CdpTransport;
    use std::time::Duration;

    #[tokio::test]
    async fn mock_returns_canned_response() {
        let mock = MockCdpClient::new();
        mock.enqueue_response("Target.createTarget", serde_json::json!({"targetId": "T-42"})).await;

        let result: CreateTargetResult = mock
            .send_raw(
                CdpCommand::CreateTarget {
                    id: 1,
                    params: CreateTargetParams { url: "about:blank".into() },
                },
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(result.target_id, "T-42");
    }

    #[tokio::test]
    async fn mock_missing_response_returns_error() {
        let mock = MockCdpClient::new();
        let result: Result<CreateTargetResult> = mock
            .send_raw(
                CdpCommand::CreateTarget {
                    id: 1,
                    params: CreateTargetParams { url: "x".into() },
                },
                Duration::from_secs(1),
            )
            .await;
        assert!(matches!(result, Err(CdpError::Protocol { .. })));
    }

    #[tokio::test]
    async fn mock_replays_enqueued_events() {
        let mock = MockCdpClient::new();
        let mut rx = mock.subscribe_events();
        mock.enqueue_event(CdpEvent::LoadEventFired {
            params: LoadEventFiredParams { timestamp: 1.0 },
        }).await;
        mock.enqueue_event(CdpEvent::RequestWillBeSent {
            params: RequestWillBeSentParams {
                request_id: "1".into(),
                request: Request {
                    url: "wss://example.com".into(),
                    method: "GET".into(),
                    headers: Default::default(),
                },
            },
        }).await;

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e1, CdpEvent::LoadEventFired { .. }));
        assert!(matches!(e2, CdpEvent::RequestWillBeSent { .. }));
    }
}