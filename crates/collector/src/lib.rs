//! `eleven-barrage-collector` — 采集器子命令
//!
//! MVP 阶段：仅占位接口。具体实现见 Phase 5 后（OQ-1 spike）。
//!
//! 采集器的作用：
//! 1. 在能跑抖音客户端的环境（PC/浏览器）中提取**已签名**的 wss URL + headers
//! 2. 通过 IPC 推送给 service 主进程
//!
//! # 长期方案
//!
//! - **Windows**：通过 CDP (Chrome DevTools Protocol) 注入浏览器，截获真实 wss 请求
//! - **Linux + headless**：用 `chromium --remote-debugging-port` + 自定义脚本
//! - **macOS**：与 Linux 相同方案
//!
//! # MVP 接口
//!
//! ```ignore
//! // 由 collector 提取并推送给 service
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! pub struct SignedWssMaterial {
//!     pub url: String,
//!     pub headers: HashMap<String, String>,
//!     pub expires_at: SystemTime,
//! }
//! ```

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

/// 已签名的 wss 连接材料（collector 产出，service 消费）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedWssMaterial {
    /// 抖音 wss 端点 URL
    pub url: String,
    /// 必需的 HTTP headers（Cookie、User-Agent、签名等）
    pub headers: HashMap<String, String>,
    /// 过期时间（用于 service 定期轮换）
    #[serde(with = "system_time_serde")]
    pub expires_at: SystemTime,
}

impl SignedWssMaterial {
    /// 检查材料是否过期
    pub fn is_expired(&self) -> bool {
        SystemTime::now() >= self.expires_at
    }
}

mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(t: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        let duration = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

/// 统一签名错误类型（auto-sign-fetcher R-006）
pub mod error;
pub use error::SignatureError;

/// URL 解析器（auto-sign-fetcher R-001）
pub mod url_parser;
pub use url_parser::{parse as parse_url, WebRid};

/// im_fetch HTTP 客户端（auto-sign-fetcher R-004）
pub mod im_fetch;
pub use im_fetch::{ImFetchConfig, ImFetchResponse, ImFetcher};

/// Collector trait — MVP 仅占位
#[async_trait::async_trait]
pub trait Collector: Send + Sync {
    /// 提取已签名的 wss 材料
    async fn extract(&self, room_id: &str) -> anyhow::Result<SignedWssMaterial>;
}

/// 默认 collector（MVP 占位实现）
pub struct DefaultCollector;

#[async_trait::async_trait]
impl Collector for DefaultCollector {
    async fn extract(&self, _room_id: &str) -> anyhow::Result<SignedWssMaterial> {
        anyhow::bail!(
            "collector is not implemented yet. \
             see devflow/custom-barrage/requirements.md (R-011, R-012) for the design. \
             MVP must provide wss URL/headers via configuration (--wss-url / --wss-headers flags)."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_material_serde_roundtrip() {
        let material = SignedWssMaterial {
            url: "wss://example.com/webcast/im/push/v2/?room_id=test".to_string(),
            headers: [("Cookie".to_string(), "ttwid=test".to_string())]
                .into_iter()
                .collect(),
            expires_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(3600),
        };

        let json = serde_json::to_string(&material).unwrap();
        let parsed: SignedWssMaterial = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.url, material.url);
        assert_eq!(
            parsed.headers.get("Cookie"),
            Some(&"ttwid=test".to_string())
        );
    }

    #[tokio::test]
    async fn default_collector_returns_error() {
        let collector = DefaultCollector;
        let result = collector.extract("test_room").await;
        assert!(result.is_err());
    }
}
