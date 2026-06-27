//! gRPC SignedBarrageService 实现（R-008）
//!
//! # 接口
//!
//! `SignedBarrageService::ProvideSignedWss(ProvideSignedWssRequest) -> ProvideSignedWssResponse`
//!
//! # 向后兼容
//!
//! - 旧客户端（不传 url）→ 返回 `Status::invalid_argument`，引导使用 `--wss-url`
//! - 新客户端（传 url）→ 调 AutoSigner，返回 SignedWssMaterial 或结构化错误

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use eleven_barrage_collector::{parse_url, SignatureError};
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::signer::AutoSigner;

// 引入 tonic-build 生成的代码（service/proto/signed.proto package="signed"）
pub mod signed_proto {
    tonic::include_proto!("signed");
}

use self::signed_proto::signed_barrage_service_server::SignedBarrageService;
pub use self::signed_proto::signed_barrage_service_server::SignedBarrageServiceServer;
use self::signed_proto::{
    ProvideSignedWssRequest, ProvideSignedWssResponse, SignatureErrorInfo, SignedWssMaterial,
};

/// gRPC SignedBarrageService 实现
pub struct SignedBarrageServiceImpl {
    signer: AutoSigner,
}

impl std::fmt::Debug for SignedBarrageServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedBarrageServiceImpl").finish()
    }
}

impl SignedBarrageServiceImpl {
    /// 创建新服务实例
    pub fn new(signer: AutoSigner) -> Self {
        Self { signer }
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

        // 向后兼容：url 缺省时返回 invalid_argument，引导旧客户端使用 --wss-url
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

        // 2. AutoSigner 签名
        match self.signer.sign(&web_rid).await {
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
                warn!(error = %e, "auto-sign failed");
                Ok(Response::new(ProvideSignedWssResponse {
                    result: Some(
                        self::signed_proto::provide_signed_wss_response::Result::Error(
                            to_error_info(&e),
                        ),
                    ),
                }))
            }
        }
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
    use crate::api::RoomInfoApi;
    use crate::config::{AuthConfig, RoomApiConfig};
    use eleven_barrage_collector::{ImFetchConfig, ImFetcher};

    fn make_test_signer() -> AutoSigner {
        let auth = AuthConfig {
            ttwid: "test_ttwid".to_string(),
            sessionid: String::new(),
        };
        let room_api = RoomInfoApi::new(RoomApiConfig::default()).unwrap();
        let im_fetcher = ImFetcher::new(ImFetchConfig::default()).unwrap();
        AutoSigner::new(room_api, im_fetcher, auth)
    }

    #[tokio::test]
    async fn provide_signed_wss_rejects_empty_url() {
        let signer = make_test_signer();
        let service = SignedBarrageServiceImpl::new(signer);

        let request = ProvideSignedWssRequest {
            url: None,
            cookie_file: None,
        };
        let result = service.provide_signed_wss(Request::new(request)).await;
        match result {
            Err(status) => assert_eq!(status.code(), tonic::Code::InvalidArgument),
            Ok(_) => panic!("expected InvalidArgument error"),
        }
    }

    #[tokio::test]
    async fn provide_signed_wss_rejects_invalid_url_with_structured_error() {
        let signer = make_test_signer();
        let service = SignedBarrageServiceImpl::new(signer);

        let request = ProvideSignedWssRequest {
            url: Some("https://v.douyin.com/abc".to_string()),
            cookie_file: None,
        };
        let response = service
            .provide_signed_wss(Request::new(request))
            .await
            .unwrap();
        let inner = response.into_inner();

        // 错误应该包装在 result 中
        match inner.result {
            Some(self::signed_proto::provide_signed_wss_response::Result::Error(err)) => {
                assert!(!err.retryable);
                assert!(err.message.contains("URL format"));
                // v.douyin.com → UrlFormatNotSupported
                use self::signed_proto::signature_error_info::Code;
                assert_eq!(err.code, Code::UrlFormatNotSupported as i32);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn provide_signed_wss_invalid_url_format() {
        let signer = make_test_signer();
        let service = SignedBarrageServiceImpl::new(signer);

        let request = ProvideSignedWssRequest {
            url: Some("not a url".to_string()),
            cookie_file: None,
        };
        let response = service
            .provide_signed_wss(Request::new(request))
            .await
            .unwrap();
        let inner = response.into_inner();
        match inner.result {
            Some(self::signed_proto::provide_signed_wss_response::Result::Error(err)) => {
                assert!(!err.retryable);
            }
            _ => panic!("expected Error"),
        }
    }

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
}
