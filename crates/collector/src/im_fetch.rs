//! im_fetch 调用（auto-sign-fetcher R-004）
//!
//! 调用抖音 `webcast/im/fetch/` 端点，拿到**已签名**的 wss URL + headers。
//!
//! # 重要
//!
//! **本模块不实现签名算法**（X-MS-STUB / signature 逆向是后续 feature 范围）。
//! 本次只做：HTTP 调用框架 + 响应解析 + 错误映射。实际签名调用是占位实现。
//!
//! # 流程
//!
//! 1. 构造请求 URL：`{base_url}/webcast/im/fetch/?room_id={room_id}&...`
//! 2. 设置 headers：User-Agent + Cookie（ttwid + sessionid）
//! 3. 解析 JSON 响应 → `SignedWssMaterial`
//! 4. 错误映射：401/403 → CookieExpired, 超时 → NetworkTransient
//!
//! # 错误映射
//!
//! | reqwest 错误 | SignatureError |
//! |---|---|
//! | timeout / connect / request | NetworkTransient |
//! | HTTP 401 / 403 | CookieExpired |
//! | HTTP 5xx | AlgorithmChanged |
//! | JSON 解析失败 | AlgorithmChanged |

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::SignatureError;
use crate::SignedWssMaterial;

/// im_fetch 配置
#[derive(Debug, Clone)]
pub struct ImFetchConfig {
    /// 抖音 API base URL
    pub base_url: String,
    /// User-Agent（模拟浏览器）
    pub user_agent: String,
    /// 请求超时
    pub timeout: Duration,
}

impl Default for ImFetchConfig {
    fn default() -> Self {
        Self {
            base_url: "https://live.douyin.com".to_string(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0".to_string(),
            timeout: Duration::from_secs(10),
        }
    }
}

/// im_fetch 响应结构（抖音实际响应字段可能更多，这里只关注必要的）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImFetchResponse {
    /// 签名后的 wss URL
    pub wss_url: String,
    /// 必需的 HTTP headers（Cookie、签名等）
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// 过期时间（Unix epoch 秒）
    pub expires_at: u64,
}

/// im_fetch 客户端
#[derive(Debug, Clone)]
pub struct ImFetcher {
    config: ImFetchConfig,
    client: Client,
}

impl ImFetcher {
    /// 创建新客户端
    pub fn new(config: ImFetchConfig) -> Result<Self, SignatureError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| SignatureError::NetworkTransient {
                reason: format!("failed to build reqwest client: {}", e),
            })?;
        Ok(Self { config, client })
    }

    /// 调用 im_fetch 拿到签名后的 wss 材料
    ///
    /// # 参数
    /// - `room_id`: 真实 room_id（来自 room_info 调用）
    /// - `cookie_header`: cookie header 值（来自 AuthConfig.to_cookie_header()）
    pub async fn fetch(
        &self,
        room_id: &str,
        cookie_header: &str,
    ) -> Result<SignedWssMaterial, SignatureError> {
        let url = format!("{}/webcast/im/fetch/", self.config.base_url);

        debug!(room_id = %room_id, "calling im_fetch");

        let response = self
            .client
            .get(&url)
            .query(&[("room_id", room_id), ("user_unique_id", room_id)])
            .header("Cookie", cookie_header)
            .header("Referer", "https://live.douyin.com/")
            .send()
            .await
            .map_err(|e| self.map_reqwest_error(e))?;

        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            warn!(status = %status, "im_fetch returned auth error");
            return Err(SignatureError::CookieExpired);
        }

        if !status.is_success() {
            warn!(status = %status, "im_fetch returned non-2xx");
            return Err(SignatureError::AlgorithmChanged);
        }

        let body: ImFetchResponse = response.json().await.map_err(|e| {
            warn!(error = %e, "im_fetch response parse failed");
            SignatureError::AlgorithmChanged
        })?;

        Ok(SignedWssMaterial {
            url: body.wss_url,
            headers: body.headers,
            expires_at: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(body.expires_at),
        })
    }

    /// reqwest::Error → SignatureError 映射
    fn map_reqwest_error(&self, e: reqwest::Error) -> SignatureError {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ===== 单元测试（不依赖 HTTP）=====

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = ImFetchConfig::default();
        assert!(cfg.base_url.contains("douyin.com"));
        assert!(cfg.user_agent.contains("Mozilla"));
        assert!(cfg.timeout >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn map_reqwest_timeout_to_transient() {
        // 模拟一个 timeout error
        let res = reqwest::Client::new()
            .get("http://10.255.255.1/")
            .timeout(Duration::from_millis(1))
            .send()
            .await;
        let err = match res {
            Err(e) => e,
            Ok(_) => panic!("expected timeout error"),
        };
        let fetcher = ImFetcher::new(ImFetchConfig::default()).unwrap();
        let mapped = fetcher.map_reqwest_error(err);
        assert!(matches!(mapped, SignatureError::NetworkTransient { .. }));
        assert!(mapped.retryable());
    }

    // ===== 集成测试（用 mock HTTP server）=====

    /// 启动一个 mock HTTP server，监听指定端口
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

    struct MockResponse {
        status: u16,
        body: String,
    }

    impl MockResponse {
        fn ok(body: &str) -> Self {
            Self {
                status: 200,
                body: body.to_string(),
            }
        }
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
                    500 => "Internal Server Error",
                    _ => "Error",
                },
                self.body.len(),
                self.body
            )
        }
    }

    fn fetcher_for(port: u16, timeout_ms: u64) -> ImFetcher {
        ImFetcher::new(ImFetchConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            user_agent: "test-agent".to_string(),
            timeout: Duration::from_millis(timeout_ms),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn fetch_success_returns_signed_material() {
        let port = start_mock_server(|req| {
            assert!(req.contains("GET /webcast/im/fetch/"));
            // reqwest sends Cookie header as lowercase (HTTP/2 spec)
            assert!(
                req.to_lowercase().contains("cookie: ttwid=abc"),
                "expected Cookie header in request, got: {}",
                req
            );
            assert!(req.contains("room_id=test_room"));
            MockResponse::ok(r#"{"wss_url":"wss://example.com/push","headers":{"X-MS-STUB":"stub123"},"expires_at":9999999999}"#)
        })
        .await;

        let fetcher = fetcher_for(port, 5000);
        let material = fetcher
            .fetch("test_room", "ttwid=abc")
            .await
            .expect("expected success");

        assert_eq!(material.url, "wss://example.com/push");
        assert_eq!(
            material.headers.get("X-MS-STUB"),
            Some(&"stub123".to_string())
        );
        assert!(!material.is_expired());
    }

    #[tokio::test]
    async fn fetch_401_returns_cookie_expired() {
        let port = start_mock_server(|_| MockResponse::status(401)).await;
        let fetcher = fetcher_for(port, 5000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::CookieExpired) => {}
            _ => panic!("expected CookieExpired, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn fetch_403_returns_cookie_expired() {
        let port = start_mock_server(|_| MockResponse::status(403)).await;
        let fetcher = fetcher_for(port, 5000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::CookieExpired) => {}
            _ => panic!("expected CookieExpired, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn fetch_500_returns_algorithm_changed() {
        let port = start_mock_server(|_| MockResponse::status(500)).await;
        let fetcher = fetcher_for(port, 5000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::AlgorithmChanged) => {}
            _ => panic!("expected AlgorithmChanged, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn fetch_malformed_json_returns_algorithm_changed() {
        let port = start_mock_server(|_| MockResponse::ok("not json at all")).await;
        let fetcher = fetcher_for(port, 5000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::AlgorithmChanged) => {}
            _ => panic!("expected AlgorithmChanged, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn fetch_missing_wss_url_field_returns_algorithm_changed() {
        let port =
            start_mock_server(|_| MockResponse::ok(r#"{"headers":{},"expires_at":999}"#)).await;
        let fetcher = fetcher_for(port, 5000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::AlgorithmChanged) => {}
            _ => panic!("expected AlgorithmChanged, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn fetch_timeout_returns_network_transient() {
        // Bind 但不 accept → 模拟连接挂起
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // 不 spawn accept → 连接会 hang/timeout

        let fetcher = fetcher_for(port, 200); // 200ms timeout
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::NetworkTransient { .. }) => {}
            _ => panic!("expected NetworkTransient, got {:?}", result),
        }

        drop(listener);
    }

    #[tokio::test]
    async fn fetch_unreachable_returns_network_transient() {
        // 绑定然后立刻 drop → port 拒绝连接
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let fetcher = fetcher_for(port, 1000);
        let result = fetcher.fetch("test_room", "ttwid=abc").await;
        match result {
            Err(SignatureError::NetworkTransient { .. }) => {}
            _ => panic!("expected NetworkTransient, got {:?}", result),
        }
    }
}
