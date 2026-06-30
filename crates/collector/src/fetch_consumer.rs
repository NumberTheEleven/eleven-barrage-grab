//! HTTP fetch fallback 消费端
//!
//! 自包含的浏览器 + CDP 拦截器：
//! 1. 独立启动一个 headless 浏览器进程。
//! 2. 导航到抖音直播间并注入 auth cookie。
//! 3. 监听 `Network.requestWillBeSent` / `Network.responseReceived`，
//!    对 `/webcast/im/fetch/` 请求调用 `Network.getResponseBody`。
//! 4. 解码裸 `Response` protobuf，去重后通过 channel 输出 `BarrageEvent`。
//! 5. 周期注入 JS 保持页面活跃。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use eleven_barrage_core::{BarrageEvent, Dispatcher, EventFilter, MsgDedup, WssDecoder};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::browser::{Browser, BrowserConfig};
use crate::cdp::client::CdpClient;
use crate::cdp::commands::{
    AttachToTargetParams, AttachToTargetResult, CloseTargetParams, CreateTargetParams,
    CdpCommand, CdpEvent, NetworkGetResponseBodyParams, NetworkGetResponseBodyResult,
    NetworkSetCookieParams, PageNavigateParams, ResponseReceivedParams, RuntimeEvaluateParams,
};
use crate::signer::is_im_fetch_url;

/// 默认去重容量（与原项目一致）
const DEFAULT_DEDUP_CAPACITY: usize = 300;

/// `FetchConsumer` 配置
#[derive(Debug, Clone)]
pub struct FetchConsumerConfig {
    /// 浏览器可执行文件路径
    pub edge_path: PathBuf,
    /// 用户数据目录
    pub user_data_dir: PathBuf,
    /// CDP 调试端口
    pub cdp_port: u16,
    /// 额外浏览器参数
    pub extra_args: Vec<String>,
    /// 抖音直播间 web_rid
    pub web_rid: String,
    /// 需要注入的 auth cookie（ttwid/sessionid）
    pub auth_cookies: HashMap<String, String>,
    /// 活跃保持 JS 注入间隔
    pub keepalive_interval: Duration,
    /// 页面导航超时
    pub navigation_timeout: Duration,
}

impl Default for FetchConsumerConfig {
    fn default() -> Self {
        Self {
            edge_path: PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            ),
            user_data_dir: PathBuf::from("./data/fetch-consumer"),
            cdp_port: 9222,
            extra_args: vec![],
            web_rid: String::new(),
            auth_cookies: HashMap::new(),
            keepalive_interval: Duration::from_secs(5),
            navigation_timeout: Duration::from_secs(30),
        }
    }
}

/// 一个附加到 CDP 会话的 tab
#[derive(Debug, Clone)]
struct TabSession {
    target_id: String,
    session_id: String,
}

/// HTTP fetch 弹幕消费端
#[derive(Debug)]
pub struct FetchConsumer {
    config: FetchConsumerConfig,
}

impl FetchConsumer {
    /// 创建新的消费端
    pub fn new(config: FetchConsumerConfig) -> Self {
        Self { config }
    }

    /// 启动浏览器、拦截 fetch 响应并输出事件。
    ///
    /// 该方法会一直运行，直到 CDP 连接断开或下游 `event_tx` 关闭。
    pub async fn run(
        self,
        event_tx: mpsc::Sender<BarrageEvent>,
        filter: EventFilter,
    ) -> Result<()> {
        let browser_config = BrowserConfig {
            edge_path: self.config.edge_path.clone(),
            user_data_dir: self.config.user_data_dir.clone(),
            extra_args: self.config.extra_args.clone(),
            cdp_port: self.config.cdp_port,
        };

        info!(cdp_port = %self.config.cdp_port, "starting fetch consumer browser");
        let mut browser = Browser::spawn(browser_config).context("spawn browser")?;
        let ws_url = browser
            .discover_cdp_url()
            .await
            .context("discover cdp url")?;
        let (cdp, _global_events) =
            CdpClient::connect(&ws_url).await.context("connect cdp")?;

        let tab = self.acquire_tab(&cdp).await.context("acquire tab")?;
        let session_id = tab.session_id.clone();

        // 启用 Page / Network 域
        cdp.send::<serde_json::Value>(
            |id| CdpCommand::PageEnable {
                id,
                session_id: Some(session_id.clone()),
            },
            Duration::from_secs(2),
        )
        .await
        .context("enable page domain")?;

        cdp.send::<serde_json::Value>(
            |id| CdpCommand::NetworkEnable {
                id,
                session_id: Some(session_id.clone()),
            },
            Duration::from_secs(2),
        )
        .await
        .context("enable network domain")?;

        // 注入 auth cookie
        self.set_auth_cookies(&cdp, &session_id).await?;

        // 导航到直播间
        cdp.send::<serde_json::Value>(
            |id| CdpCommand::PageNavigate {
                id,
                session_id: Some(session_id.clone()),
                params: PageNavigateParams {
                    url: format!("https://live.douyin.com/{}", self.config.web_rid),
                    referrer: Some("https://live.douyin.com/".into()),
                },
            },
            self.config.navigation_timeout,
        )
        .await
        .context("navigate to live room")?;

        info!(web_rid = %self.config.web_rid, "fetch consumer navigated");

        let mut events = cdp.subscribe_session(&session_id);
        let mut pending_requests: HashMap<String, ()> = HashMap::new();
        let decoder = WssDecoder::new();
        let dispatcher = Dispatcher::new();
        let dedup = MsgDedup::new(DEFAULT_DEDUP_CAPACITY);

        // 启动活跃保持任务
        let keepalive_handle =
            self.spawn_keepalive_task(cdp.clone(), session_id.clone());

        let result = self
            .event_loop(
                &cdp,
                &session_id,
                &mut events,
                &mut pending_requests,
                &decoder,
                &dispatcher,
                &dedup,
                &filter,
                &event_tx,
            )
            .await;

        keepalive_handle.abort();
        let _ = self.close_tab(&cdp, &tab.target_id).await;
        let _ = browser.kill();

        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn event_loop(
        &self,
        cdp: &CdpClient,
        session_id: &str,
        events: &mut tokio::sync::broadcast::Receiver<CdpEvent>,
        pending_requests: &mut HashMap<String, ()>,
        decoder: &WssDecoder,
        dispatcher: &Dispatcher,
        dedup: &MsgDedup,
        filter: &EventFilter,
        event_tx: &mpsc::Sender<BarrageEvent>,
    ) -> Result<()> {
        loop {
            if event_tx.is_closed() {
                info!("downstream closed, stopping fetch consumer");
                return Ok(());
            }

            let evt = events
                .recv()
                .await
                .map_err(|_| anyhow::anyhow!("cdp event channel closed"))?;

            match evt {
                CdpEvent::RequestWillBeSent { params } => {
                    if is_im_fetch_url(&params.request.url) {
                        debug!(request_id = %params.request_id, url = %params.request.url, "fetch request observed");
                        pending_requests.insert(params.request_id, ());
                    }
                }
                CdpEvent::ResponseReceived {
                    params: ResponseReceivedParams { request_id },
                } => {
                    if pending_requests.remove(&request_id).is_some() {
                        self.handle_fetch_response(
                            cdp,
                            session_id,
                            &request_id,
                            decoder,
                            dispatcher,
                            dedup,
                            filter,
                            event_tx,
                        )
                        .await;
                    }
                }
                CdpEvent::DetachedFromTarget { .. } => {
                    error!("fetch consumer tab detached");
                    return Err(anyhow::anyhow!("cdp tab detached"));
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_fetch_response(
        &self,
        cdp: &CdpClient,
        session_id: &str,
        request_id: &str,
        decoder: &WssDecoder,
        dispatcher: &Dispatcher,
        dedup: &MsgDedup,
        filter: &EventFilter,
        event_tx: &mpsc::Sender<BarrageEvent>,
    ) {
        debug!(request_id = %request_id, "fetch response received, requesting body");

        let body_result: NetworkGetResponseBodyResult = match cdp
            .send(
                |id| CdpCommand::NetworkGetResponseBody {
                    id,
                    session_id: Some(session_id.into()),
                    params: NetworkGetResponseBodyParams {
                        request_id: request_id.into(),
                    },
                },
                Duration::from_secs(5),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "failed to get fetch response body");
                return;
            }
        };

        let body_bytes = if body_result.base64_encoded {
            match base64::engine::general_purpose::STANDARD.decode(&body_result.body) {
                Ok(v) => v,
                Err(e) => {
                    warn!(request_id = %request_id, error = %e, "failed to base64 decode response body");
                    return;
                }
            }
        } else {
            body_result.body.into_bytes()
        };

        self.process_body_bytes(
            request_id,
            &body_bytes,
            decoder,
            dispatcher,
            dedup,
            filter,
            event_tx,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_body_bytes(
        &self,
        request_id: &str,
        body_bytes: &[u8],
        decoder: &WssDecoder,
        dispatcher: &Dispatcher,
        dedup: &MsgDedup,
        filter: &EventFilter,
        event_tx: &mpsc::Sender<BarrageEvent>,
    ) {
        debug!(request_id = %request_id, len = body_bytes.len(), "captured fetch response body");

        if body_bytes.is_empty() {
            return;
        }

        let response = match decoder.decode_fetch_body(body_bytes) {
            Ok(r) => r,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "failed to decode fetch body");
                return;
            }
        };

        let events = match dispatcher.dispatch_response(&response) {
            Ok(events) => events,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "dispatch error");
                return;
            }
        };

        for event in events {
            if !filter.allows(&event) {
                continue;
            }
            if !dedup.check_and_mark(event.method(), event.msg_id()) {
                debug!(method = %event.method(), msg_id = %event.msg_id(), "duplicate event skipped");
                continue;
            }
            if event_tx.send(event).await.is_err() {
                return;
            }
        }
    }

    async fn set_auth_cookies(
        &self,
        cdp: &CdpClient,
        session_id: &str,
    ) -> Result<()> {
        for (name, value) in &self.config.auth_cookies {
            if let Err(e) = cdp
                .send::<serde_json::Value>(
                    |id| CdpCommand::NetworkSetCookie {
                        id,
                        session_id: Some(session_id.into()),
                        params: NetworkSetCookieParams {
                            name: name.clone(),
                            value: value.clone(),
                            domain: Some(".douyin.com".into()),
                            url: None,
                            path: Some("/".into()),
                        },
                    },
                    Duration::from_secs(2),
                )
                .await
            {
                warn!(cookie_name = %name, error = %e, "failed to set cookie");
            }
        }
        Ok(())
    }

    fn spawn_keepalive_task(
        &self,
        cdp: CdpClient,
        session_id: String,
    ) -> tokio::task::JoinHandle<()> {
        let interval = self.config.keepalive_interval;
        let expression = r#"
            Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
            window.dispatchEvent(new Event('focus'));
            window.dispatchEvent(new Event('mousemove'));
            true
        "#
        .to_string();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let _ = cdp
                    .send::<serde_json::Value>(
                        |id| CdpCommand::RuntimeEvaluate {
                            id,
                            session_id: Some(session_id.clone()),
                            params: RuntimeEvaluateParams {
                                expression: expression.clone(),
                                return_by_value: Some(false),
                            },
                        },
                        Duration::from_secs(2),
                    )
                    .await;
            }
        })
    }

    async fn acquire_tab(&self, cdp: &CdpClient) -> Result<TabSession> {
        let target: crate::cdp::commands::CreateTargetResult = cdp
            .send(
                |id| CdpCommand::CreateTarget {
                    id,
                    params: CreateTargetParams {
                        url: "about:blank".into(),
                    },
                },
                Duration::from_secs(3),
            )
            .await
            .context("create target")?;

        let attach: AttachToTargetResult = cdp
            .send(
                |id| CdpCommand::AttachToTarget {
                    id,
                    params: AttachToTargetParams {
                        target_id: target.target_id.clone(),
                        flatten: Some(true),
                    },
                },
                Duration::from_secs(3),
            )
            .await
            .context("attach to target")?;

        Ok(TabSession {
            target_id: target.target_id,
            session_id: attach.session_id,
        })
    }

    async fn close_tab(&self, cdp: &CdpClient, target_id: &str) -> Result<()> {
        let _: serde_json::Value = cdp
            .send(
                |id| CdpCommand::CloseTarget {
                    id,
                    params: CloseTargetParams {
                        target_id: target_id.into(),
                    },
                },
                Duration::from_secs(2),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use std::time::Duration;

    #[tokio::test]
    async fn fetch_consumer_processes_raw_response_body() {
        // 构造一个最小 Response protobuf
        let chat = eleven_barrage_proto::ChatMessage {
            content: "hello fetch".to_string(),
            common: Some(eleven_barrage_proto::Common {
                msg_id: 100,
                ..Default::default()
            }),
            ..Default::default()
        };
        let msg = eleven_barrage_proto::Message {
            method: eleven_barrage_core::message_method::CHAT.to_string(),
            payload: chat.encode_to_vec(),
            msg_id: 100,
            ..Default::default()
        };
        let response = eleven_barrage_proto::Response {
            messages: vec![msg],
            ..Default::default()
        };
        let body = response.encode_to_vec();

        let consumer = FetchConsumer::new(FetchConsumerConfig {
            web_rid: "123".into(),
            ..Default::default()
        });

        let (tx, mut rx) = mpsc::channel::<BarrageEvent>(16);
        let decoder = WssDecoder::new();
        let dispatcher = Dispatcher::new();
        let dedup = MsgDedup::new(300);
        let filter = EventFilter::mvp_default();

        consumer
            .process_body_bytes("r1", &body, &decoder, &dispatcher, &dedup, &filter, &tx)
            .await;

        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.method(), eleven_barrage_core::message_method::CHAT);
        assert_eq!(event.msg_id(), 100);
    }

    #[test]
    fn fetch_consumer_config_default() {
        let cfg = FetchConsumerConfig::default();
        assert_eq!(cfg.keepalive_interval, Duration::from_secs(5));
        assert!(cfg.auth_cookies.is_empty());
    }
}
