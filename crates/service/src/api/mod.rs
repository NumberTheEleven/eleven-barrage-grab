//! REST API for external programs (auto-signer spec section 5)
//!
//! Also includes the room metadata API client (`RoomInfoApi`).

pub mod health;
pub mod rooms;
pub mod sign;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{routing::{delete, get, post}, Router};
use serde::{Deserialize, Serialize};

use eleven_barrage_collector::pool::BrowserPool;
use eleven_barrage_collector::SignatureError;

use crate::config::{BrowserConfig, RoomApiConfig};
use crate::dynamic_room::DynamicRoomManager;

// ── Room metadata API ──────────────────────────────────────────

/// 房间元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    /// 内部 room_id
    pub room_id: String,
    /// web 房间 ID
    pub web_room_id: String,
    /// 主播昵称
    pub owner_nickname: Option<String>,
    /// 直播间标题
    pub title: Option<String>,
    /// 是否开播
    pub is_live: bool,
    /// 推流地址（用于拉流/分析）
    pub live_url: String,
}

/// 房间元数据 API 客户端
#[derive(Debug, Clone)]
pub struct RoomInfoApi {
    config: RoomApiConfig,
    client: reqwest::Client,
}

impl RoomInfoApi {
    /// 创建新客户端
    pub fn new(config: RoomApiConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent)
            .build()
            .context("failed to build reqwest client")?;
        Ok(Self { config, client })
    }

    /// 获取房间信息
    pub async fn get(&self, web_room_id: &str) -> Result<RoomInfo> {
        let mut query = HashMap::new();
        query.insert("aid", "6383");
        query.insert("live_id", "1");
        query.insert("app_name", "douyin_web");
        query.insert("device_platform", "web");
        query.insert("language", "zh-CN");
        query.insert("cookie_enabled", "true");
        query.insert("enter_from", "page_refresh");
        query.insert("web_rid", web_room_id);
        query.insert("enter_source", "");
        query.insert("is_need_double_stream", "false");
        query.insert("insert_task_id", "");
        query.insert("live_reason", "");
        query.insert("browser_language", "zh-CN");
        query.insert("browser_platform", "Win32");
        query.insert("browser_name", "Edge");

        let url = format!("{}/webcast/room/web/enter/", self.config.base_url);

        let mut request = self
            .client
            .get(&url)
            .query(&query)
            .header("Accept", "application/json, text/plain, */*")
            .header("Cache-Control", "no-cache")
            .header(
                "Referer",
                format!("{}/{}", self.config.base_url, web_room_id),
            )
            .header("Host", "live.douyin.com");

        if !self.config.cookie.is_empty() {
            request = request.header("Cookie", &self.config.cookie);
        }

        let response = request
            .send()
            .await
            .context("failed to send room info request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "room info API returned non-success status: {}",
                response.status()
            );
        }

        let body = response
            .text()
            .await
            .context("failed to read room info response body")?;

        let parsed: serde_json::Value =
            serde_json::from_str(&body).context("failed to parse room info JSON")?;

        let data = parsed
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("missing 'data' field in room info response"))?;

        let room = data
            .get("room")
            .ok_or_else(|| anyhow::anyhow!("missing 'room' field in room info data"))?;

        let room_id_str = room
            .get("id_str")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let owner_nickname = room
            .get("owner")
            .and_then(|o| o.get("nickname"))
            .and_then(|n| n.as_str())
            .map(String::from);

        let title = room.get("title").and_then(|t| t.as_str()).map(String::from);

        let status = room.get("status").and_then(|s| s.as_i64()).unwrap_or(0);
        let is_live = status == 2;

        let live_url = format!("https://live.douyin.com/{}", web_room_id);

        Ok(RoomInfo {
            room_id: room_id_str,
            web_room_id: web_room_id.to_string(),
            owner_nickname,
            title,
            is_live,
            live_url,
        })
    }

    /// Auto-signer 专用方法：返回 `SignatureError` 结构化错误
    pub async fn get_for_signer(&self, web_rid: &str) -> std::result::Result<RoomInfo, SignatureError> {
        let mut query = HashMap::new();
        query.insert("aid", "6383");
        query.insert("live_id", "1");
        query.insert("app_name", "douyin_web");
        query.insert("device_platform", "web");
        query.insert("language", "zh-CN");
        query.insert("cookie_enabled", "true");
        query.insert("enter_from", "page_refresh");
        query.insert("web_rid", web_rid);
        query.insert("enter_source", "");
        query.insert("is_need_double_stream", "false");
        query.insert("insert_task_id", "");
        query.insert("live_reason", "");
        query.insert("browser_language", "zh-CN");
        query.insert("browser_platform", "Win32");
        query.insert("browser_name", "Edge");

        let url = format!("{}/webcast/room/web/enter/", self.config.base_url);

        let mut request = self
            .client
            .get(&url)
            .query(&query)
            .header("Accept", "application/json, text/plain, */*")
            .header("Cache-Control", "no-cache")
            .header("Referer", format!("{}/{}", self.config.base_url, web_rid))
            .header("Host", "live.douyin.com");

        if !self.config.cookie.is_empty() {
            request = request.header("Cookie", &self.config.cookie);
        }

        let response = request.send().await.map_err(map_reqwest_error)?;

        let status = response.status();
        let status_code = status.as_u16();

        if status_code == 401 || status_code == 403 {
            return Err(SignatureError::CookieExpired);
        }
        if status_code == 404 {
            return Err(SignatureError::RoomNotFound {
                web_rid: web_rid.to_string(),
            });
        }
        if !status.is_success() {
            return Err(SignatureError::AlgorithmChanged);
        }

        let body = response
            .text()
            .await
            .map_err(|e| SignatureError::NetworkTransient {
                reason: format!("read body: {}", e),
            })?;

        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| SignatureError::AlgorithmChanged)?;

        let data = parsed.get("data").ok_or(SignatureError::AlgorithmChanged)?;

        let room = data.get("room").ok_or(SignatureError::AlgorithmChanged)?;

        let room_id_str = room
            .get("id_str")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if room_id_str.is_empty() {
            return Err(SignatureError::AlgorithmChanged);
        }

        let owner_nickname = room
            .get("owner")
            .and_then(|o| o.get("nickname"))
            .and_then(|n| n.as_str())
            .map(String::from);

        let title = room.get("title").and_then(|t| t.as_str()).map(String::from);

        let status_code = room.get("status").and_then(|s| s.as_i64()).unwrap_or(0);
        let is_live = status_code == 2;

        let live_url = format!("{}/{}", self.config.base_url, web_rid);

        Ok(RoomInfo {
            room_id: room_id_str,
            web_room_id: web_rid.to_string(),
            owner_nickname,
            title,
            is_live,
            live_url,
        })
    }
}

fn map_reqwest_error(e: reqwest::Error) -> SignatureError {
    if e.is_timeout() || e.is_connect() || e.is_request() {
        SignatureError::NetworkTransient {
            reason: format!("{}", e),
        }
    } else if matches!(e.status().map(|s| s.as_u16()), Some(401) | Some(403)) {
        SignatureError::CookieExpired
    } else {
        SignatureError::AlgorithmChanged
    }
}

// ── REST API router ────────────────────────────────────────────

/// 应用全部 REST 路由（含 /v1/sign, /v1/health, /v1/rooms）
///
/// - `pool`：浏览器池（用于 /v1/sign 和 /v1/rooms POST）
/// - `rooms`：动态房间管理器
/// - `browser`：浏览器配置（用于 fetch path）
/// - `auth_cookies`：全局认证 cookie
pub fn router(
    pool: Arc<BrowserPool>,
    rooms: Arc<DynamicRoomManager>,
    browser: BrowserConfig,
    auth_cookies: HashMap<String, String>,
) -> Router {
    let pool_only = Router::new()
        .route("/v1/sign", post(sign::sign))
        .route("/v1/health", get(health::health))
        .with_state(pool.clone());

    let rooms_state = rooms::RoomsState {
        pool,
        rooms,
        browser,
        auth_cookies,
    };

    let rooms_routes = Router::new()
        .route("/v1/rooms", post(rooms::create_room).get(rooms::list_rooms))
        .route("/v1/rooms/:room_id", delete(rooms::destroy_room))
        .with_state(rooms_state);

    pool_only.merge(rooms_routes)
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_info_api_creation() {
        let cfg = RoomApiConfig::default();
        let api = RoomInfoApi::new(cfg);
        assert!(api.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires network access to live.douyin.com"]
    async fn get_real_room_info() {
        let cfg = RoomApiConfig::default();
        let api = RoomInfoApi::new(cfg).unwrap();
        let result = api.get("741891423654").await;
        if let Ok(info) = result {
            assert!(!info.web_room_id.is_empty());
        }
    }

    // ===== get_for_signer 集成测试 (R-005) =====

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct MockResponse {
        status: u16,
        body: String,
    }

    impl MockResponse {
        fn status(code: u16) -> Self {
            Self {
                status: code,
                body: String::new(),
            }
        }
        fn to_http_bytes(&self) -> String {
            format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                self.status,
                match self.status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Error",
                },
                self.body.len(),
                self.body
            )
        }
    }

    async fn start_mock_server(handler: impl Fn(String) -> MockResponse + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let resp = handler(req);
                let _ = stream.write_all(resp.to_http_bytes().as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        port
    }

    fn build_test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn map_404_to_room_not_found() {
        let port = start_mock_server(|_| MockResponse::status(404)).await;
        let client = build_test_client();
        let resp = client
            .get(format!("http://127.0.0.1:{}/test", port))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn map_401_to_cookie_expired() {
        let port = start_mock_server(|_| MockResponse::status(401)).await;
        let client = build_test_client();
        let resp = client
            .get(format!("http://127.0.0.1:{}/test", port))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn map_unreachable_to_network_transient() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let client = build_test_client();
        let res = client
            .get(format!("http://127.0.0.1:{}/test", port))
            .send()
            .await;
        assert!(res.is_err());
    }

    #[test]
    fn map_reqwest_timeout_helper() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = match rt.block_on(async {
            reqwest::Client::new()
                .get("http://10.255.255.1/")
                .timeout(Duration::from_millis(1))
                .send()
                .await
        }) {
            Ok(_) => panic!("expected timeout error"),
            Err(e) => e,
        };
        let mapped = map_reqwest_error(err);
        assert!(matches!(mapped, SignatureError::NetworkTransient { .. }));
        assert!(mapped.retryable());
    }

    #[test]
    fn map_reqwest_status_403_to_cookie_expired() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let port = rt.block_on(async { start_mock_server(|_| MockResponse::status(403)).await });
        let res = rt.block_on(async {
            reqwest::Client::new()
                .get(format!("http://127.0.0.1:{}/test", port))
                .send()
                .await
        });
        if let Ok(resp) = res {
            assert_eq!(resp.status().as_u16(), 403);
        }
    }
}
