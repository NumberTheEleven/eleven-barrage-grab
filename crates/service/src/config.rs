//! 配置加载（TOML + 环境变量 + CLI flags 三层覆盖）
//!
//! # 三层覆盖优先级
//! 1. **CLI flags**：最高优先级（如 `--room-id`）
//! 2. **环境变量**：`ELEVEN_BARRAGE_*` 前缀
//! 3. **TOML 配置文件**：默认 `config.toml`
//!
//! # 设计参考
//! 借鉴原项目 `AppConfig.json` 的结构，但重命名为更适合 Rust 生态的格式。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use eleven_barrage_collector::SignatureError;

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub service: ServiceConfig,

    #[serde(default)]
    pub wss: WssConfig,

    #[serde(default)]
    pub events: EventsConfig,

    #[serde(default)]
    pub room_api: RoomApiConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub mitm: MitmConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub browser: BrowserConfig,

    #[serde(default)]
    pub rest: RestConfig,

    #[serde(default)]
    pub signer: SignerConfig,
}

/// service 主体配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// 抖音直播间标识（web_room_id，如 "741891423654"）
    pub room_id: String,

    /// WS 服务端监听地址（下游消费者接入）
    #[serde(default = "default_ws_addr")]
    pub ws_listen_addr: SocketAddr,

    /// gRPC 服务端监听地址
    #[serde(default = "default_grpc_addr")]
    pub grpc_listen_addr: SocketAddr,

    /// Prometheus metrics 端点地址
    #[serde(default = "default_metrics_addr")]
    pub metrics_listen_addr: SocketAddr,
}

/// WSS 上游连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssConfig {
    /// wss URL（空 → 必须由 collector 提供）
    #[serde(default)]
    pub url: String,

    /// 必需的 HTTP headers（如 Cookie、签名等）
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// 5s 心跳保底（参考原项目 commit 85d9514）
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,

    /// 重连初始延迟
    #[serde(default = "default_reconnect_initial_secs")]
    pub reconnect_initial_secs: u64,

    /// 重连最大延迟
    #[serde(default = "default_reconnect_max_secs")]
    pub reconnect_max_secs: u64,

    /// 最大重连次数（达到后停止）
    #[serde(default = "default_max_reconnect_attempts")]
    pub max_reconnect_attempts: u32,
}

/// 事件推送配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsConfig {
    /// 允许推送的事件 method 列表
    /// 默认：Chat/Gift/Like
    #[serde(default = "default_push_event_methods")]
    pub push_event_methods: Vec<String>,
}

/// 房间元数据 API 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomApiConfig {
    /// 登录态 Cookie（可选）
    #[serde(default)]
    pub cookie: String,

    /// User-Agent
    #[serde(default = "default_user_agent")]
    pub user_agent: String,

    /// 是否在启动时调用 room info API（用于获取 room_id_str）
    #[serde(default = "default_room_api_enabled")]
    pub enabled: bool,

    /// 自定义 API base URL（默认 live.douyin.com，用于测试）
    #[serde(default = "default_room_api_base_url")]
    pub base_url: String,
}

fn default_room_api_base_url() -> String {
    "https://live.douyin.com".to_string()
}

/// 鉴权配置（auto-sign-fetcher R-002）
///
/// 用户从浏览器复制的登录态 cookie，用于调用
/// `webcast/room/web/enter/` 和 `webcast/im/fetch/`。
///
/// 至少需要提供一个非空字段，否则 `validate()` 返回错误。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// ttwid cookie（必需，至少一个）
    #[serde(default)]
    pub ttwid: String,

    /// sessionid cookie（可选）
    #[serde(default)]
    pub sessionid: String,
}

impl AuthConfig {
    /// 校验：至少一个 cookie 字段非空
    pub fn validate(&self) -> Result<(), SignatureError> {
        if self.ttwid.trim().is_empty() && self.sessionid.trim().is_empty() {
            return Err(SignatureError::ConfigMissing {
                field: "auth.ttwid or auth.sessionid".to_string(),
            });
        }
        Ok(())
    }

    /// 拼接 cookie header 值（`ttwid=xxx; sessionid=yyy`）
    pub fn to_cookie_header(&self) -> String {
        let mut parts = Vec::new();
        if !self.ttwid.trim().is_empty() {
            parts.push(format!("ttwid={}", self.ttwid.trim()));
        }
        if !self.sessionid.trim().is_empty() {
            parts.push(format!("sessionid={}", self.sessionid.trim()));
        }
        parts.join("; ")
    }

    /// 是否有任一有效 cookie
    pub fn has_any(&self) -> bool {
        !self.ttwid.trim().is_empty() || !self.sessionid.trim().is_empty()
    }
}

/// MITM 兜底配置（参考原项目 TitaniumProxy 模式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MitmConfig {
    /// 启用 MITM 兜底
    #[serde(default)]
    pub fallback_enabled: bool,

    /// 系统代理端口
    #[serde(default = "default_mitm_proxy_port")]
    pub proxy_port: u16,
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// 日志级别（trace/debug/info/warn/error）
    #[serde(default = "default_log_level")]
    pub level: String,

    /// 是否使用 JSON 格式（生产）
    #[serde(default)]
    pub json_format: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default = "default_edge_path")]
    pub edge_path: PathBuf,

    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_per_browser: usize,

    #[serde(default = "default_sign_timeout")]
    pub sign_timeout_secs: u64,

    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,

    #[serde(default = "default_user_data_dir_template")]
    pub user_data_dir_template: String,

    #[serde(default)]
    pub extra_args: Vec<String>,

    #[serde(default = "default_cdp_port_base")]
    pub cdp_port_base: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestConfig {
    #[serde(default = "default_rest_addr")]
    pub listen_addr: SocketAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerConfig {
    #[serde(default = "default_signer_mode_str")]
    pub mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignerMode {
    Browser,
    Http,
    Auto,
}

// 默认值
fn default_ws_addr() -> SocketAddr {
    "0.0.0.0:8888".parse().unwrap()
}
fn default_grpc_addr() -> SocketAddr {
    "0.0.0.0:50051".parse().unwrap()
}
fn default_metrics_addr() -> SocketAddr {
    "0.0.0.0:9090".parse().unwrap()
}
fn default_heartbeat_interval_secs() -> u64 {
    5
}
fn default_reconnect_initial_secs() -> u64 {
    1
}
fn default_reconnect_max_secs() -> u64 {
    60
}
fn default_max_reconnect_attempts() -> u32 {
    0 // 0 = 无限重试
}
fn default_push_event_methods() -> Vec<String> {
    vec![
        "WebcastChatMessage".to_string(),
        "WebcastGiftMessage".to_string(),
        "WebcastLikeMessage".to_string(),
    ]
}
fn default_user_agent() -> String {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0".to_string()
}
fn default_room_api_enabled() -> bool {
    true
}
fn default_mitm_proxy_port() -> u16 {
    8827
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_edge_path() -> PathBuf {
    PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
}
fn default_pool_size() -> usize { 3 }
fn default_max_concurrent() -> usize { 2 }
fn default_sign_timeout() -> u64 { 10 }
fn default_health_check_interval() -> u64 { 30 }
fn default_user_data_dir_template() -> String {
    "./data/browser-{id}".into()
}
fn default_cdp_port_base() -> u16 { 9222 }
fn default_rest_addr() -> SocketAddr { "0.0.0.0:7878".parse().unwrap() }
fn default_signer_mode_str() -> String { "browser".into() }

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            room_id: String::new(),
            ws_listen_addr: default_ws_addr(),
            grpc_listen_addr: default_grpc_addr(),
            metrics_listen_addr: default_metrics_addr(),
        }
    }
}
impl Default for WssConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            headers: HashMap::new(),
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            reconnect_initial_secs: default_reconnect_initial_secs(),
            reconnect_max_secs: default_reconnect_max_secs(),
            max_reconnect_attempts: default_max_reconnect_attempts(),
        }
    }
}
impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            push_event_methods: default_push_event_methods(),
        }
    }
}
impl Default for RoomApiConfig {
    fn default() -> Self {
        Self {
            cookie: String::new(),
            user_agent: default_user_agent(),
            enabled: default_room_api_enabled(),
            base_url: default_room_api_base_url(),
        }
    }
}
impl Default for MitmConfig {
    fn default() -> Self {
        Self {
            fallback_enabled: false,
            proxy_port: default_mitm_proxy_port(),
        }
    }
}
impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            json_format: false,
        }
    }
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            edge_path: default_edge_path(),
            pool_size: default_pool_size(),
            max_concurrent_per_browser: default_max_concurrent(),
            sign_timeout_secs: default_sign_timeout(),
            health_check_interval_secs: default_health_check_interval(),
            user_data_dir_template: default_user_data_dir_template(),
            extra_args: vec![],
            cdp_port_base: default_cdp_port_base(),
        }
    }
}

impl Default for RestConfig {
    fn default() -> Self {
        Self { listen_addr: default_rest_addr() }
    }
}

impl Default for SignerConfig {
    fn default() -> Self {
        Self { mode: "browser".into() }
    }
}
impl AppConfig {
    /// 从 TOML 文件加载配置
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("failed to read config file: {:?}", path.as_ref()))?;
        let config: AppConfig = toml::from_str(&content)
            .with_context(|| format!("failed to parse TOML config: {:?}", path.as_ref()))?;
        Ok(config)
    }

    /// 尝试加载默认配置文件（不存在时返回默认值）
    pub fn load_or_default() -> Self {
        let paths = ["config.toml", "/etc/eleven-barrage-grab/config.toml"];
        for path in &paths {
            if std::path::Path::new(path).exists() {
                match Self::from_file(path) {
                    Ok(cfg) => {
                        tracing::info!(path = %path, "loaded config from file");
                        return cfg;
                    }
                    Err(e) => {
                        tracing::warn!(path = %path, error = %e, "failed to load config, using defaults");
                    }
                }
            }
        }
        tracing::info!("no config file found, using defaults");
        Self::default()
    }

    /// 应用环境变量覆盖（`ELEVEN_BARRAGE_*`）
    ///
    /// 支持的变量：
    /// - `ELEVEN_BARRAGE_ROOM_ID`：覆盖 `service.room_id`
    /// - `ELEVEN_BARRAGE_WSS_URL`：覆盖 `wss.url`
    /// - `ELEVEN_BARRAGE_COOKIE`：覆盖 `room_api.cookie`
    /// - `ELEVEN_BARRAGE_LOG_LEVEL`：覆盖 `logging.level`
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("ELEVEN_BARRAGE_ROOM_ID") {
            tracing::info!(old = %self.service.room_id, new = %v, "override room_id from env");
            self.service.room_id = v;
        }
        if let Ok(v) = std::env::var("ELEVEN_BARRAGE_WSS_URL") {
            tracing::info!(new = %v, "override wss.url from env");
            self.wss.url = v;
        }
        if let Ok(v) = std::env::var("ELEVEN_BARRAGE_COOKIE") {
            tracing::info!("override cookie from env");
            self.room_api.cookie = v;
        }
        if let Ok(v) = std::env::var("ELEVEN_BARRAGE_LOG_LEVEL") {
            tracing::info!(new = %v, "override log level from env");
            self.logging.level = v;
        }
    }

    /// 校验配置合法性
    pub fn validate(&self) -> Result<()> {
        if self.service.room_id.is_empty() {
            anyhow::bail!(
                "service.room_id is empty. \
                 Set via config.toml [service] section, \
                 ELEVEN_BARRAGE_ROOM_ID env var, or --room-id CLI flag."
            );
        }
        if self.wss.url.is_empty() {
            tracing::warn!(
                "wss.url is empty. Service will start but cannot connect to upstream \
                 until collector provides SignedWssMaterial. See R-011/R-012."
            );
        }
        if self.push_event_methods().is_empty() {
            anyhow::bail!("events.push_event_methods cannot be empty");
        }
        Ok(())
    }

    /// 获取 push_event_methods 列表
    pub fn push_event_methods(&self) -> &[String] {
        &self.events.push_event_methods
    }

    pub fn signer_mode(&self) -> SignerMode {
        match self.signer.mode.as_str() {
            "browser" => SignerMode::Browser,
            "http" => SignerMode::Http,
            "auto" => SignerMode::Auto,
            _ => SignerMode::Browser,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            [service]
            room_id = "741891423654"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.service.room_id, "741891423654");
        assert_eq!(cfg.service.ws_listen_addr.port(), 8888);
    }

    #[test]
    fn default_config_validates_room_id_required() {
        let cfg = AppConfig::default();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn full_config_parses() {
        let toml = r#"
            [service]
            room_id = "741891423654"
            ws_listen_addr = "127.0.0.1:9999"

            [wss]
            url = "wss://example.com/webcast"
            heartbeat_interval_secs = 5

            [events]
            push_event_methods = ["WebcastChatMessage"]

            [room_api]
            cookie = "ttwid=test"
            enabled = true

            [mitm]
            fallback_enabled = false
            proxy_port = 8827

            [logging]
            level = "debug"
            json_format = true
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.service.ws_listen_addr.port(), 9999);
        assert_eq!(cfg.wss.heartbeat_interval_secs, 5);
    }

    // ===== AuthConfig 测试 (R-002) =====

    #[test]
    fn auth_config_default_is_empty() {
        let auth = AuthConfig::default();
        assert_eq!(auth.ttwid, "");
        assert_eq!(auth.sessionid, "");
        assert!(!auth.has_any());
    }

    #[test]
    fn auth_config_validate_fails_when_empty() {
        let auth = AuthConfig::default();
        let result = auth.validate();
        assert!(matches!(result, Err(SignatureError::ConfigMissing { .. })));
    }

    #[test]
    fn auth_config_validate_passes_with_ttwid() {
        let auth = AuthConfig {
            ttwid: "test_ttwid".to_string(),
            sessionid: String::new(),
        };
        assert!(auth.validate().is_ok());
    }

    #[test]
    fn auth_config_validate_passes_with_sessionid_only() {
        let auth = AuthConfig {
            ttwid: String::new(),
            sessionid: "test_sessionid".to_string(),
        };
        assert!(auth.validate().is_ok());
    }

    #[test]
    fn auth_config_to_cookie_header_ttwid_only() {
        let auth = AuthConfig {
            ttwid: "abc".to_string(),
            sessionid: String::new(),
        };
        assert_eq!(auth.to_cookie_header(), "ttwid=abc");
    }

    #[test]
    fn auth_config_to_cookie_header_both() {
        let auth = AuthConfig {
            ttwid: "abc".to_string(),
            sessionid: "def".to_string(),
        };
        assert_eq!(auth.to_cookie_header(), "ttwid=abc; sessionid=def");
    }

    #[test]
    fn auth_config_to_cookie_header_trims_whitespace() {
        let auth = AuthConfig {
            ttwid: "  abc  ".to_string(),
            sessionid: String::new(),
        };
        assert_eq!(auth.to_cookie_header(), "ttwid=abc");
    }

    #[test]
    fn auth_config_parses_from_toml() {
        let toml = r#"
            [auth]
            ttwid = "test_ttwid"
            sessionid = "test_sessionid"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.auth.ttwid, "test_ttwid");
        assert_eq!(cfg.auth.sessionid, "test_sessionid");
        assert!(cfg.auth.validate().is_ok());
    }

    #[test]
    fn auth_config_parses_optional_section() {
        let toml = r#"
            [service]
            room_id = "test"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.auth.ttwid, "");
        assert_eq!(cfg.auth.sessionid, "");
    }

    #[test]
    fn auth_config_has_any() {
        let empty = AuthConfig::default();
        assert!(!empty.has_any());

        let ttwid = AuthConfig {
            ttwid: "x".to_string(),
            sessionid: String::new(),
        };
        assert!(ttwid.has_any());

        let sessionid = AuthConfig {
            ttwid: String::new(),
            sessionid: "y".to_string(),
        };
        assert!(sessionid.has_any());
    }

    #[test]
    fn parse_browser_config_section() {
        let toml = r#"
            [service]
            room_id = "test"

            [browser]
            edge_path = "C:\\Edge\\msedge.exe"
            pool_size = 5
            max_concurrent_per_browser = 3
            sign_timeout_secs = 15
            health_check_interval_secs = 60
            user_data_dir_template = "./data/browser-{id}"
            extra_args = ["--foo", "--bar"]
            cdp_port_base = 9333
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.browser.pool_size, 5);
        assert_eq!(cfg.browser.max_concurrent_per_browser, 3);
        assert_eq!(cfg.browser.sign_timeout_secs, 15);
        assert_eq!(cfg.browser.extra_args, vec!["--foo", "--bar"]);
        assert_eq!(cfg.browser.cdp_port_base, 9333);
    }

    #[test]
    fn parse_rest_config_section() {
        let toml = r#"
            [service]
            room_id = "test"

            [rest]
            listen_addr = "127.0.0.1:9000"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.rest.listen_addr.port(), 9000);
    }

    #[test]
    fn parse_signer_mode_auto() {
        let toml = r#"
            [service]
            room_id = "test"

            [signer]
            mode = "auto"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(matches!(cfg.signer_mode(), SignerMode::Auto));
    }

    #[test]
    fn default_browser_config_has_sensible_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.browser.pool_size, 3);
        assert_eq!(cfg.browser.max_concurrent_per_browser, 2);
        assert_eq!(cfg.browser.sign_timeout_secs, 10);
        assert_eq!(cfg.browser.cdp_port_base, 9222);
    }

    #[test]
    fn default_rest_config_uses_port_7878() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.rest.listen_addr.port(), 7878);
    }

    #[test]
    fn default_signer_mode_is_browser() {
        let cfg = AppConfig::default();
        assert!(matches!(cfg.signer_mode(), SignerMode::Browser));
    }

    #[test]
    fn invalid_signer_mode_defaults_to_browser() {
        let toml = r#"
            [service]
            room_id = "test"

            [signer]
            mode = "unknown"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(matches!(cfg.signer_mode(), SignerMode::Browser));
    }
}
