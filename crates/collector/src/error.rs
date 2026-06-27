//! 统一签名错误类型（R-006）
//!
//! 设计原则：
//! - **结构化错误码**：7 个变体，每种错误有明确语义
//! - **Retryable 标志**：调用方根据 `retryable()` 决定是否重试
//! - **持久化 Display**：错误信息包含足够上下文（URL / web_rid / 字段名）
//!
//! # 错误分类
//!
//! | 变体 | 触发场景 | Retryable |
//! |------|---------|-----------|
//! | `UrlFormatNotSupported` | URL 不是 live.douyin.com | false |
//! | `EmptyUrl` | URL 为空字符串 | false |
//! | `ConfigMissing` | config.toml 缺 cookie | false |
//! | `CookieExpired` | HTTP 401/403 | false |
//! | `AlgorithmChanged` | im_fetch 响应格式异常 | false |
//! | `RoomNotFound` | room_info 404 | false |
//! | `NetworkTransient` | 网络超时/连接错误 | true |

use std::fmt;

/// 统一签名错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// URL 格式不支持（仅 live.douyin.com 可接受）
    UrlFormatNotSupported { url: String },

    /// URL 为空字符串
    EmptyUrl,

    /// 必要配置缺失
    ConfigMissing { field: String },

    /// Cookie 过期或无效（HTTP 401/403）
    CookieExpired,

    /// 签名算法变更（im_fetch 响应格式未知）
    AlgorithmChanged,

    /// 房间不存在或已下播
    RoomNotFound { web_rid: String },

    /// 网络瞬时错误（超时、连接失败等）
    NetworkTransient { reason: String },
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UrlFormatNotSupported { url } => {
                write!(
                    f,
                    "URL format not supported: {} (only live.douyin.com is accepted)",
                    url
                )
            }
            Self::EmptyUrl => write!(f, "URL is empty"),
            Self::ConfigMissing { field } => write!(f, "config missing: {}", field),
            Self::CookieExpired => write!(
                f,
                "cookie expired: please re-paste ttwid/sessionid in config.toml"
            ),
            Self::AlgorithmChanged => write!(
                f,
                "algorithm changed: im_fetch response format is unknown. \
                 Service may need a signature update."
            ),
            Self::RoomNotFound { web_rid } => write!(f, "room not found: {}", web_rid),
            Self::NetworkTransient { reason } => write!(f, "network transient error: {}", reason),
        }
    }
}

impl std::error::Error for SignatureError {}

impl SignatureError {
    /// 错误码（用于日志、指标、gRPC 响应）
    pub fn code(&self) -> &'static str {
        match self {
            Self::UrlFormatNotSupported { .. } => "URL_FORMAT_NOT_SUPPORTED",
            Self::EmptyUrl => "EMPTY_URL",
            Self::ConfigMissing { .. } => "CONFIG_MISSING",
            Self::CookieExpired => "COOKIE_EXPIRED",
            Self::AlgorithmChanged => "ALGORITHM_CHANGED",
            Self::RoomNotFound { .. } => "ROOM_NOT_FOUND",
            Self::NetworkTransient { .. } => "NETWORK_TRANSIENT",
        }
    }

    /// 错误是否可重试
    ///
    /// - `true`：调用方可根据策略重试（如网络抖动）
    /// - `false`：必须人工介入（如 cookie 过期、算法变更）
    pub fn retryable(&self) -> bool {
        match self {
            Self::UrlFormatNotSupported { .. } => false,
            Self::EmptyUrl => false,
            Self::ConfigMissing { .. } => false,
            Self::CookieExpired => false,
            Self::AlgorithmChanged => false,
            Self::RoomNotFound { .. } => false,
            Self::NetworkTransient { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== retryable() 行为测试 =====

    #[test]
    fn url_format_not_supported_not_retryable() {
        let err = SignatureError::UrlFormatNotSupported {
            url: "https://example.com/xxx".to_string(),
        };
        assert!(!err.retryable());
    }

    #[test]
    fn empty_url_not_retryable() {
        assert!(!SignatureError::EmptyUrl.retryable());
    }

    #[test]
    fn config_missing_not_retryable() {
        let err = SignatureError::ConfigMissing {
            field: "auth.ttwid".to_string(),
        };
        assert!(!err.retryable());
    }

    #[test]
    fn cookie_expired_not_retryable() {
        assert!(!SignatureError::CookieExpired.retryable());
    }

    #[test]
    fn algorithm_changed_not_retryable() {
        assert!(!SignatureError::AlgorithmChanged.retryable());
    }

    #[test]
    fn room_not_found_not_retryable() {
        let err = SignatureError::RoomNotFound {
            web_rid: "nonexistent".to_string(),
        };
        assert!(!err.retryable());
    }

    #[test]
    fn network_transient_is_retryable() {
        let err = SignatureError::NetworkTransient {
            reason: "connection timeout".to_string(),
        };
        assert!(err.retryable());
    }

    // ===== code() 行为测试 =====

    #[test]
    fn error_codes_are_stable_strings() {
        assert_eq!(
            SignatureError::UrlFormatNotSupported {
                url: "x".to_string()
            }
            .code(),
            "URL_FORMAT_NOT_SUPPORTED"
        );
        assert_eq!(SignatureError::EmptyUrl.code(), "EMPTY_URL");
        assert_eq!(
            SignatureError::ConfigMissing {
                field: "f".to_string()
            }
            .code(),
            "CONFIG_MISSING"
        );
        assert_eq!(SignatureError::CookieExpired.code(), "COOKIE_EXPIRED");
        assert_eq!(SignatureError::AlgorithmChanged.code(), "ALGORITHM_CHANGED");
        assert_eq!(
            SignatureError::RoomNotFound {
                web_rid: "x".to_string()
            }
            .code(),
            "ROOM_NOT_FOUND"
        );
        assert_eq!(
            SignatureError::NetworkTransient {
                reason: "x".to_string()
            }
            .code(),
            "NETWORK_TRANSIENT"
        );
    }

    // ===== Display 输出测试 =====

    #[test]
    fn display_includes_context() {
        let err = SignatureError::UrlFormatNotSupported {
            url: "https://v.douyin.com/abc".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("v.douyin.com"));
        assert!(msg.contains("live.douyin.com"));
    }

    #[test]
    fn display_room_not_found_includes_web_rid() {
        let err = SignatureError::RoomNotFound {
            web_rid: "123456".to_string(),
        };
        assert!(err.to_string().contains("123456"));
    }

    #[test]
    fn display_cookie_expired_suggests_re_paste() {
        let err = SignatureError::CookieExpired;
        let msg = err.to_string();
        assert!(msg.contains("re-paste") || msg.contains("re-paste"));
    }

    #[test]
    fn display_config_missing_includes_field() {
        let err = SignatureError::ConfigMissing {
            field: "auth.ttwid".to_string(),
        };
        assert!(err.to_string().contains("auth.ttwid"));
    }

    // ===== std::error::Error trait 测试 =====

    #[test]
    fn implements_error_trait() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        let err = SignatureError::CookieExpired;
        assert_error(&err);
    }

    #[test]
    fn equality_works() {
        let a = SignatureError::NetworkTransient {
            reason: "x".to_string(),
        };
        let b = SignatureError::NetworkTransient {
            reason: "x".to_string(),
        };
        assert_eq!(a, b);
    }
}
