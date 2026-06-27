//! `ebg` — CLI 入口
//!
//! 子命令：
//! - `start` (默认) — 启动 service（沿用 custom-barrage 行为）
//! - `grab` — 自动签名模式：用户提供 URL，服务自动完成签名（R-007）

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::error;

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

    /// 自动签名并获取弹幕流（R-007）
    ///
    /// 用户提供抖音直播间 URL，服务自动调用 room_info + im_fetch 拿到签名后的 wss URL。
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
            // 默认行为：启动 service
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
            verbose: _,
        }) => match run_grab(&url, cookie_file.as_ref(), &grpc_addr).await {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                // 结构化错误输出（参考 design.md 5）
                eprintln!("Error: signature error");
                eprintln!("  code: {}", e.code());
                eprintln!("  retryable: {}", e.retryable());
                eprintln!("  message: {}", e);
                ExitCode::FAILURE
            }
        },
    }
}

/// ebg grab 实现：调 gRPC ProvideSignedWss
async fn run_grab(
    url: &str,
    _cookie_file: Option<&PathBuf>,
    _grpc_addr: &str,
) -> Result<eleven_barrage_collector::SignedWssMaterial, eleven_barrage_collector::SignatureError> {
    // 本次实现只验证 URL 解析 + AutoSigner 链路
    // 实际 gRPC 客户端调用留给后续 T-008 集成（与 service 一起部署）
    //
    // MVP 阶段：直接调 collector 的 parse_url + 通过 service 进程内部 AutoSigner
    // 完整方案：起本地 service，gRPC 调 ProvideSignedWss

    use eleven_barrage_collector::parse_url;
    let web_rid = parse_url(url)?;

    tracing::info!(web_rid = %web_rid, "URL parsed");

    // 占位返回（实际由 gRPC client 拿）
    Err(eleven_barrage_collector::SignatureError::AlgorithmChanged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_grab_with_url() {
        let cli = Cli::try_parse_from(["ebg", "grab", "--url", "https://live.douyin.com/test"]).unwrap();
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
            Some(EbgCommand::Grab { url, cookie_file, .. }) => {
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

    #[tokio::test]
    async fn run_grab_invalid_url_returns_structured_error() {
        let result = run_grab("https://v.douyin.com/abc", None, "http://127.0.0.1:50051").await;
        match result {
            Err(err) => {
                assert_eq!(err.code(), "URL_FORMAT_NOT_SUPPORTED");
                assert!(!err.retryable());
            }
            Ok(_) => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn run_grab_valid_url_returns_placeholder_error() {
        // 当前实现返回 AlgorithmChanged 作为占位（实际 gRPC 客户端留给后续集成）
        let result = run_grab(
            "https://live.douyin.com/664637748606",
            None,
            "http://127.0.0.1:50051",
        )
        .await;
        match result {
            Err(err) => {
                assert_eq!(err.code(), "ALGORITHM_CHANGED");
            }
            Ok(_) => panic!("expected placeholder error"),
        }
    }
}
