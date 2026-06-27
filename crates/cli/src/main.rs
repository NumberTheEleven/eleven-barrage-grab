//! `ebg` — CLI 入口
//!
//! 子命令：
//! - `start` (默认) — 启动 service（沿用 custom-barrage 行为）
//! - `grab` — 自动签名模式：用户提供 URL，服务自动完成签名并连接 WSS（R-007 / R-005）

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "ebg", about = "eleven-barrage-grab CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<EbgCommand>,
}

#[derive(Subcommand)]
enum EbgCommand {
    /// 启动 service daemon（沿用 custom-barrage 行为）
    Start,

    /// 自动签名并获取弹幕流（R-007 / R-005）
    ///
    /// 用户提供抖音直播间 URL，服务自动调用 room_info + im_fetch 拿到签名后的 wss URL，
    /// 然后直接连接 WSS 并输出弹幕事件。
    Grab {
        /// 抖音直播间 URL（如 https://live.douyin.com/664637748606）
        #[arg(long)]
        url: String,

        /// cookie 文件路径（覆盖 config.toml 中的 [auth] 段）
        #[arg(long)]
        cookie_file: Option<PathBuf>,

        /// gRPC 服务端地址
        #[arg(long, default_value = "http://127.0.0.1:50051")]
        grpc_addr: String,

        /// 详细日志
        #[arg(long, short)]
        verbose: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        None | Some(EbgCommand::Start) => {
            if let Err(e) = eleven_barrage_service::run().await {
                error!(error = %e, "service run failed");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Some(EbgCommand::Grab {
            url,
            cookie_file,
            grpc_addr,
            verbose,
        }) => match run_grab(&url, cookie_file.as_ref(), &grpc_addr, verbose).await {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: signature error");
                eprintln!("  code: {}", e.code());
                eprintln!("  retryable: {}", e.retryable());
                eprintln!("  message: {}", e);
                ExitCode::FAILURE
            }
        },
    }
}

/// ebg grab 实现：调 gRPC ProvideSignedWss，然后连接 WSS 并输出弹幕
async fn run_grab(
    url: &str,
    cookie_file: Option<&PathBuf>,
    grpc_addr: &str,
    verbose: bool,
) -> Result<(), eleven_barrage_collector::SignatureError> {
    // 1. URL 解析（本地快速失败）
    let _web_rid = eleven_barrage_collector::parse_url(url)?;
    info!(web_rid = %_web_rid, "URL parsed");

    // 2. 调 gRPC ProvideSignedWss
    let material = call_provide_signed_wss(grpc_addr, url, cookie_file).await?;

    info!(wss_url = %material.url, "signed wss material received");

    // 3. 连接 WSS 并输出弹幕
    if let Err(e) = connect_and_print(&material, verbose).await {
        return Err(eleven_barrage_collector::SignatureError::NetworkTransient {
            reason: format!("wss connection failed: {}", e),
        });
    }

    Ok(())
}

/// gRPC 调用 ProvideSignedWss
async fn call_provide_signed_wss(
    grpc_addr: &str,
    url: &str,
    _cookie_file: Option<&PathBuf>,
) -> Result<eleven_barrage_collector::SignedWssMaterial, eleven_barrage_collector::SignatureError> {
    use eleven_barrage_service::signed_proto::signed_barrage_service_client::SignedBarrageServiceClient;
    use eleven_barrage_service::signed_proto::ProvideSignedWssRequest;

    let mut client = SignedBarrageServiceClient::connect(grpc_addr.to_string())
        .await
        .map_err(
            |e| eleven_barrage_collector::SignatureError::NetworkTransient {
                reason: format!("grpc connect failed: {}", e),
            },
        )?;

    let request = ProvideSignedWssRequest {
        url: Some(url.to_string()),
        cookie_file: _cookie_file.map(|p| p.to_string_lossy().to_string()),
    };

    let response = client
        .provide_signed_wss(tonic::Request::new(request))
        .await
        .map_err(
            |e| eleven_barrage_collector::SignatureError::NetworkTransient {
                reason: format!("grpc call failed: {}", e),
            },
        )?;

    let inner = response.into_inner();
    match inner.result {
        Some(
            eleven_barrage_service::signed_proto::provide_signed_wss_response::Result::Material(m),
        ) => Ok(eleven_barrage_collector::SignedWssMaterial {
            url: m.url,
            headers: m.headers.into_iter().collect(),
            expires_at: std::time::SystemTime::UNIX_EPOCH
                + Duration::from_secs(m.expires_at_unix as u64),
        }),
        Some(eleven_barrage_service::signed_proto::provide_signed_wss_response::Result::Error(
            err,
        )) => Err(map_proto_error(err)),
        None => Err(eleven_barrage_collector::SignatureError::AlgorithmChanged),
    }
}

/// proto SignatureErrorInfo → collector SignatureError
fn map_proto_error(
    err: eleven_barrage_service::signed_proto::SignatureErrorInfo,
) -> eleven_barrage_collector::SignatureError {
    use eleven_barrage_service::signed_proto::signature_error_info::Code;
    let code = Code::try_from(err.code).unwrap_or(Code::Unknown);
    match code {
        Code::UrlFormatNotSupported => {
            eleven_barrage_collector::SignatureError::UrlFormatNotSupported { url: err.message }
        }
        Code::EmptyUrl => eleven_barrage_collector::SignatureError::EmptyUrl,
        Code::ConfigMissing => {
            eleven_barrage_collector::SignatureError::ConfigMissing { field: err.message }
        }
        Code::CookieExpired => eleven_barrage_collector::SignatureError::CookieExpired,
        Code::AlgorithmChanged => eleven_barrage_collector::SignatureError::AlgorithmChanged,
        Code::RoomNotFound => eleven_barrage_collector::SignatureError::RoomNotFound {
            web_rid: err.message,
        },
        Code::NetworkTransient => eleven_barrage_collector::SignatureError::NetworkTransient {
            reason: err.message,
        },
        Code::Unknown => eleven_barrage_collector::SignatureError::AlgorithmChanged,
    }
}

/// 用签名后的 material 连接 WSS，并打印弹幕事件
async fn connect_and_print(
    material: &eleven_barrage_collector::SignedWssMaterial,
    verbose: bool,
) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::http::Request;

    let mut request_builder = Request::builder().method("GET").uri(&material.url);
    for (key, value) in &material.headers {
        request_builder = request_builder.header(key.as_str(), value.as_str());
    }
    let request = request_builder.body(()).expect("build wss request");

    let (ws_stream, response) = tokio_tungstenite::connect_async(request).await?;
    info!(status = ?response.status(), "wss connected");

    let (mut write, mut read) = ws_stream.split();
    let decoder = eleven_barrage_core::WssDecoder::new();
    let dispatcher = eleven_barrage_core::Dispatcher::new();

    while let Some(msg) = read.next().await {
        match msg? {
            tokio_tungstenite::tungstenite::Message::Binary(frame) => {
                let decode_result = decoder.decode(&frame, false);
                match decode_result {
                    Ok((wss_response, inner_response)) => {
                        let events = dispatcher.dispatch(&wss_response, &inner_response)?;
                        for event in events {
                            if verbose {
                                println!("{:#?}", event);
                            } else {
                                println!("{}: {}", event.method(), event.msg_id());
                            }
                        }
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("decode error: {}", e);
                        }
                    }
                }
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                info!("wss closed by server");
                break;
            }
            tokio_tungstenite::tungstenite::Message::Ping(data) => {
                write
                    .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                    .await?;
            }
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_grab_with_url() {
        let cli =
            Cli::try_parse_from(["ebg", "grab", "--url", "https://live.douyin.com/test"]).unwrap();
        match cli.command {
            Some(EbgCommand::Grab { url, .. }) => {
                assert_eq!(url, "https://live.douyin.com/test");
            }
            _ => panic!("expected Grab command"),
        }
    }

    #[test]
    fn cli_parses_grab_with_cookie_file() {
        let cli = Cli::try_parse_from([
            "ebg",
            "grab",
            "--url",
            "https://live.douyin.com/test",
            "--cookie-file",
            "/tmp/cookie.txt",
        ])
        .unwrap();
        match cli.command {
            Some(EbgCommand::Grab {
                url, cookie_file, ..
            }) => {
                assert_eq!(url, "https://live.douyin.com/test");
                assert_eq!(cookie_file, Some(PathBuf::from("/tmp/cookie.txt")));
            }
            _ => panic!("expected Grab command"),
        }
    }

    #[test]
    fn cli_defaults_to_start_without_subcommand() {
        let cli = Cli::try_parse_from(["ebg"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_parses_explicit_start() {
        let cli = Cli::try_parse_from(["ebg", "start"]).unwrap();
        assert!(matches!(cli.command, Some(EbgCommand::Start)));
    }

    #[test]
    fn cli_grab_requires_url() {
        let result = Cli::try_parse_from(["ebg", "grab"]);
        assert!(result.is_err());
    }

    #[test]
    fn map_proto_error_url_format() {
        let err = eleven_barrage_service::signed_proto::SignatureErrorInfo {
            code: eleven_barrage_service::signed_proto::signature_error_info::Code::UrlFormatNotSupported as i32,
            retryable: false,
            message: "bad url".to_string(),
        };
        let mapped = map_proto_error(err);
        assert_eq!(mapped.code(), "URL_FORMAT_NOT_SUPPORTED");
        assert!(!mapped.retryable());
    }

    #[test]
    fn map_proto_error_cookie_expired() {
        let err = eleven_barrage_service::signed_proto::SignatureErrorInfo {
            code: eleven_barrage_service::signed_proto::signature_error_info::Code::CookieExpired
                as i32,
            retryable: false,
            message: "cookie expired".to_string(),
        };
        let mapped = map_proto_error(err);
        assert_eq!(mapped.code(), "COOKIE_EXPIRED");
        assert!(!mapped.retryable());
    }

    #[test]
    fn map_proto_error_network_transient() {
        let err = eleven_barrage_service::signed_proto::SignatureErrorInfo {
            code: eleven_barrage_service::signed_proto::signature_error_info::Code::NetworkTransient
                as i32,
            retryable: true,
            message: "timeout".to_string(),
        };
        let mapped = map_proto_error(err);
        assert_eq!(mapped.code(), "NETWORK_TRANSIENT");
        assert!(mapped.retryable());
    }
}
