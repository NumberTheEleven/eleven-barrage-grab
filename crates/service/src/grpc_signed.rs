//! gRPC SignedBarrageService 实现（R-008）
//!
//! # 接口
//!
//! `SignedBarrageService::ProvideSignedWss(ProvideSignedWssRequest) -> ProvideSignedWssResponse`
//!
//! # 向后兼容
//!
//! - 旧客户端（不传 url）→ 返回 `Status::invalid_argument`，引导使用 `--wss-url`
//! - 新客户端（传 url）→ 调 BrowserPool，返回 SignedWssMaterial 或结构化错误

use std::collections::HashMap;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use eleven_barrage_collector::pool::{BrowserPool, PoolError};
use eleven_barrage_collector::{parse_url, SignatureError};
use tonic::{Request, Response, Status};
use tracing::{info, warn};

// 引入 tonic-build 生成的代码（service/proto/signed.proto package="signed"）
pub mod signed_proto {
    tonic::include_proto!("signed");
}

use self::signed_proto::signed_barrage_service_server::SignedBarrageService;
pub use self::signed_proto::signed_barrage_service_server::SignedBarrageServiceServer;
use self::signed_proto::{
    ProvideSignedWssRequest, ProvideSignedWssResponse, SignatureErrorInfo, SignedWssMaterial,
};

/// gRPC SignedBarrageService 实现（BrowserPool 后端）
pub struct SignedBarrageServiceImpl {
    pool: Arc<BrowserPool>,
}

impl std::fmt::Debug for SignedBarrageServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedBarrageServiceImpl").finish()
    }
}

impl SignedBarrageServiceImpl {
    /// 创建新服务实例
    pub fn new(pool: Arc<BrowserPool>) -> Self {
        Self { pool }
    }

    /// 注册到 tonic Server
    pub fn into_server(self) -> SignedBarrageServiceServer<Self> {
        SignedBarrageServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl SignedBarrageService for SignedBarrageServiceImpl {
    async fn provide_signed_wss(
        &self,
        request: Request<ProvideSignedWssRequest>,
    ) -> Result<Response<ProvideSignedWssResponse>, Status> {
        let req = request.into_inner();

        // 向后兼容：url 缺省时返回 invalid_argument
        let url = match req.url {
            Some(u) if !u.trim().is_empty() => u,
            _ => {
                return Err(Status::invalid_argument(
                    "url is required for auto-sign mode; \
                     for custom-barrage mode, use --wss-url flag instead",
                ));
            }
        };

        info!(url = %url, "gRPC ProvideSignedWss request");

        // 1. URL 解析
        let web_rid = match parse_url(&url) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "url parse failed");
                return Ok(Response::new(ProvideSignedWssResponse {
                    result: Some(
                        self::signed_proto::provide_signed_wss_response::Result::Error(
                            to_error_info(&e),
                        ),
                    ),
                }));
            }
        };

        // 2. BrowserPool 签名
        match self.pool.sign(&web_rid).await {
            Ok(material) => {
                let grpc_material = to_grpc_material(material);
                Ok(Response::new(ProvideSignedWssResponse {
                    result: Some(
                        self::signed_proto::provide_signed_wss_response::Result::Material(
                            grpc_material,
                        ),
                    ),
                }))
            }
            Err(e) => {
                let sig_err = map_pool_error_to_signature_error(&e);
                warn!(error = %e, "auto-sign failed");
                Ok(Response::new(ProvideSignedWssResponse {
                    result: Some(
                        self::signed_proto::provide_signed_wss_response::Result::Error(
                            to_error_info(&sig_err),
                        ),
                    ),
                }))
            }
        }
    }
}

/// PoolError → SignatureError 映射（保留 gRPC 客户端的结构化错误码）
fn map_pool_error_to_signature_error(e: &PoolError) -> SignatureError {
    if e.is_timeout() {
        return SignatureError::NetworkTransient {
            reason: "WSS timeout".into(),
        };
    }
    match e {
        PoolError::Busy => SignatureError::NetworkTransient {
            reason: "pool busy".into(),
        },
        PoolError::Sign(_) => SignatureError::AlgorithmChanged,
        PoolError::Browser(_) => SignatureError::NetworkTransient {
            reason: "browser dead".into(),
        },
    }
}

/// SignatureError → SignatureErrorInfo (proto)
fn to_error_info(err: &SignatureError) -> SignatureErrorInfo {
    let code = match err.code() {
        "URL_FORMAT_NOT_SUPPORTED" => {
            self::signed_proto::signature_error_info::Code::UrlFormatNotSupported as i32
        }
        "EMPTY_URL" => self::signed_proto::signature_error_info::Code::EmptyUrl as i32,
        "CONFIG_MISSING" => self::signed_proto::signature_error_info::Code::ConfigMissing as i32,
        "COOKIE_EXPIRED" => self::signed_proto::signature_error_info::Code::CookieExpired as i32,
        "ALGORITHM_CHANGED" => {
            self::signed_proto::signature_error_info::Code::AlgorithmChanged as i32
        }
        "ROOM_NOT_FOUND" => self::signed_proto::signature_error_info::Code::RoomNotFound as i32,
        "NETWORK_TRANSIENT" => {
            self::signed_proto::signature_error_info::Code::NetworkTransient as i32
        }
        _ => self::signed_proto::signature_error_info::Code::Unknown as i32,
    };

    SignatureErrorInfo {
        code,
        retryable: err.retryable(),
        message: err.to_string(),
    }
}

/// SignedWssMaterial → proto (with expires_at unix conversion)
fn to_grpc_material(material: eleven_barrage_collector::SignedWssMaterial) -> SignedWssMaterial {
    let expires_at_unix = material
        .expires_at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    SignedWssMaterial {
        url: material.url,
        headers: material.headers.into_iter().collect::<HashMap<_, _>>(),
        expires_at_unix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_error_info_maps_all_codes() {
        let errors = [
            SignatureError::UrlFormatNotSupported {
                url: "x".to_string(),
            },
            SignatureError::EmptyUrl,
            SignatureError::ConfigMissing {
                field: "x".to_string(),
            },
            SignatureError::CookieExpired,
            SignatureError::AlgorithmChanged,
            SignatureError::RoomNotFound {
                web_rid: "x".to_string(),
            },
            SignatureError::NetworkTransient {
                reason: "x".to_string(),
            },
        ];
        for err in &errors {
            let info = to_error_info(err);
            assert!(info.code > 0, "code should be mapped: {}", err);
        }
    }

    #[test]
    fn to_grpc_material_converts_expires_at() {
        use std::time::SystemTime;
        let material = eleven_barrage_collector::SignedWssMaterial {
            url: "wss://example.com".to_string(),
            headers: HashMap::new(),
            expires_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1700000000),
        };
        let grpc = to_grpc_material(material);
        assert_eq!(grpc.url, "wss://example.com");
        assert_eq!(grpc.expires_at_unix, 1700000000);
    }

    #[test]
    fn map_pool_error_busy_to_network_transient() {
        let e = PoolError::Busy;
        let sig = map_pool_error_to_signature_error(&e);
        assert!(sig.retryable());
        assert_eq!(sig.code(), "NETWORK_TRANSIENT");
    }

    #[test]
    fn map_pool_error_sign_timeout_to_network_transient() {
        let e = PoolError::Sign("CDP command timed out after 10s".into());
        let sig = map_pool_error_to_signature_error(&e);
        assert!(sig.retryable());
        assert_eq!(sig.code(), "NETWORK_TRANSIENT");
    }

    #[test]
    fn map_pool_error_sign_generic_to_algorithm_changed() {
        let e = PoolError::Sign("unexpected error".into());
        let sig = map_pool_error_to_signature_error(&e);
        assert!(!sig.retryable());
        assert_eq!(sig.code(), "ALGORITHM_CHANGED");
    }

    #[test]
    fn map_pool_error_browser_to_network_transient() {
        let e = PoolError::Browser("crashed".into());
        let sig = map_pool_error_to_signature_error(&e);
        assert!(sig.retryable());
        assert_eq!(sig.code(), "NETWORK_TRANSIENT");
    }
}
