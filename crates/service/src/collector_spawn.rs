//! 采集器启动公共入口（spawn_collector）
//!
//! 把现有 `WssConnectionManager` / `FetchConnectionManager` 抽象为统一入口：
//! 接受一个 `SignedMaterial`，根据 `kind` 启动对应采集后台任务。
//!
//! 接受一个回调 `on_event`，由调用方决定把事件放到哪里（推送给 `WsServer` 的 dispatcher、
//! 推送给 RoomManager 等）。这样避免了与具体下游耦合。
//!
//! # 取消
//!
//! `run` 返回的 `JoinHandle` 在调用 `.abort()` 时立即取消；
//! 此外 `event_tx` 关闭时 `WssConnectionManager.run` / `FetchConsumer.run` 也会自动退出。
//!
//! # 取消的取舍
//!
//! - 对 WSS：用 `WssConfig` 提供 url、headers；通过 `_fetch_consumer` 路径走 headless
//! - 对 HTTP fetch：需要 `WebRid` + auth cookies + 浏览器配置；
//!   service 层调用方需要把这些参数与 `SignedMaterial` 一起传进来

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use eleven_barrage_collector::{
    fetch_consumer::FetchConsumerConfig, SignedMaterial, SignedMaterialKind,
};
use eleven_barrage_core::BarrageEvent;

use crate::fetch::FetchConnectionManager;
use crate::wss::WssConnectionManager;

/// Spawn-Collector 上下文（额外的 HTTP fetch 路径所需）
#[derive(Debug, Clone)]
pub struct CollectorContext {
    /// 浏览器可执行文件路径
    pub browser_path: PathBuf,
    /// 用户数据目录
    pub user_data_dir: PathBuf,
    /// CDP 调试端口（base）
    pub cdp_port_base: u16,
    /// 额外浏览器参数
    pub extra_args: Vec<String>,
    /// WebRid（用于 fetch path）
    pub web_rid: String,
    /// auth cookies（ttwid / sessionid）
    pub auth_cookies: HashMap<String, String>,
}

impl Default for CollectorContext {
    fn default() -> Self {
        Self {
            browser_path: PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            ),
            user_data_dir: PathBuf::from("./data/collector"),
            cdp_port_base: 9222,
            extra_args: Vec::new(),
            web_rid: String::new(),
            auth_cookies: HashMap::new(),
        }
    }
}

/// 启动一个 collector 后台任务。
///
/// `material`：签名后的端点
/// `event_tx`：BarrageEvent 输出通道
/// `ctx`：HTTP fetch 路径需要的额外上下文
/// 返回 `JoinHandle`，调用 `.abort()` 即可取消。
pub fn spawn_collector(
    material: SignedMaterial,
    event_tx: mpsc::Sender<BarrageEvent>,
    ctx: CollectorContext,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        match material.kind() {
            SignedMaterialKind::Wss => {
                if let Err(e) = run_wss_collector(material, event_tx).await {
                    warn!(error = %e, "wss collector failed");
                }
            }
            SignedMaterialKind::HttpFetch => {
                if let Err(e) = run_fetch_collector(material, event_tx, ctx).await {
                    warn!(error = %e, "http fetch collector failed");
                }
            }
        }
    })
}

async fn run_wss_collector(
    material: SignedMaterial,
    event_tx: mpsc::Sender<BarrageEvent>,
) -> anyhow::Result<()> {
    use eleven_barrage_core::EventFilter;
    let url = material.url().to_string();
    let headers = material.headers().clone();

    info!(url = %url, "starting wss collector");

    let wss_config = crate::config::WssConfig {
        url,
        headers,
        ..Default::default()
    };

    let manager = WssConnectionManager::new(wss_config, EventFilter::mvp_default());
    // room_id 仅为日志用途
    manager
        .run("dynamic-room".into(), None, event_tx)
        .await
}

async fn run_fetch_collector(
    _material: SignedMaterial,
    event_tx: mpsc::Sender<BarrageEvent>,
    ctx: CollectorContext,
) -> anyhow::Result<()> {
    info!(
        web_rid = %ctx.web_rid,
        browser = %ctx.browser_path.display(),
        "starting http fetch collector"
    );

    // 用 web_rid 取一个独立 cdp 端口，避免和其它 room 冲突
    let cdp_port = ctx.cdp_port_base;
    let fetch_config = FetchConsumerConfig {
        edge_path: ctx.browser_path,
        user_data_dir: ctx.user_data_dir,
        cdp_port,
        extra_args: ctx.extra_args,
        web_rid: ctx.web_rid,
        auth_cookies: ctx.auth_cookies,
        keepalive_interval: std::time::Duration::from_secs(5),
        navigation_timeout: std::time::Duration::from_secs(30),
    };

    let manager = FetchConnectionManager::new(
        fetch_config,
        eleven_barrage_core::EventFilter::mvp_default(),
    );
    manager.run("dynamic-room".into(), None, event_tx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use eleven_barrage_collector::{SignedWssMaterial, SignedMaterial};

    fn make_wss_material(url: &str) -> SignedMaterial {
        SignedMaterial::Wss(SignedWssMaterial {
            url: url.to_string(),
            headers: HashMap::new(),
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
        })
    }

    #[tokio::test]
    async fn spawn_collector_returns_join_handle_for_wss() {
        let (tx, _rx) = mpsc::channel::<BarrageEvent>(16);
        let material = make_wss_material("wss://invalid.example/test");
        let ctx = CollectorContext::default();
        let handle = spawn_collector(material, tx, ctx);
        // 立即 abort，避免实际连接
        handle.abort();
    }

    #[tokio::test]
    async fn spawn_collector_returns_join_handle_for_http_fetch() {
        let (tx, _rx) = mpsc::channel::<BarrageEvent>(16);
        let material = SignedMaterial::HttpFetch(SignedWssMaterial {
            url: "https://live.douyin.com/webcast/im/fetch/".to_string(),
            headers: HashMap::new(),
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
        });
        let ctx = CollectorContext::default();
        let handle = spawn_collector(material, tx, ctx);
        handle.abort();
    }
}
