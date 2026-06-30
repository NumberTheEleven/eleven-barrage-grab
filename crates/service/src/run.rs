//! 主服务运行入口（可被 main.rs 和 cli 复用）

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::signal;
use tracing::{error, info};

use crate::{
    config::AppConfig, dynamic_room::DynamicRoomManager, grpc_server, logging,
    metrics::MetricsExporter, rest_server, watchdog::Watchdog, ws_server::WsServer,
};

#[derive(Parser, Debug)]
#[command(
    name = "eleven-barrage-grab",
    version,
    about = "高性能抖音直播弹幕服务 (Rust 全栈)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// 配置文件路径（默认 config.toml）
    #[arg(short, long, global = true)]
    pub config: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 启动服务（默认命令）
    Start,

    /// 显示当前配置（不启动服务）
    ShowConfig,

    /// 验证配置合法性
    Validate,
}

/// 主入口函数（被 main.rs 和 cli crate 调用）
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    // 1. 加载配置
    let mut config = match &cli.config {
        Some(path) => AppConfig::from_file(path)?,
        None => AppConfig::load_or_default(),
    };

    // 2. 应用环境变量覆盖
    config.apply_env_overrides();

    // 处理子命令
    match cli.command.as_ref().unwrap_or(&Command::Start) {
        Command::ShowConfig => {
            println!("{}", toml::to_string_pretty(&config)?);
            return Ok(());
        }
        Command::Validate => {
            config.validate()?;
            println!("✓ configuration is valid");
            return Ok(());
        }
        Command::Start => {
            // 继续启动流程
        }
    }

    // 3. 校验配置
    config
        .validate()
        .context("configuration validation failed")?;

    // 4. 初始化日志
    logging::init(&config.logging)?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "starting eleven-barrage-grab (dynamic room subscription mode)"
    );

    // 5. 安装 metrics
    let _metrics_exporter = MetricsExporter::install(&config.service)
        .context("failed to install metrics exporter")?;

    // 6. 构建 auth cookies map
    let mut auth_cookies = std::collections::HashMap::new();
    if !config.auth.ttwid.is_empty() {
        auth_cookies.insert("ttwid".to_string(), config.auth.ttwid.clone());
    }
    if !config.auth.sessionid.is_empty() {
        auth_cookies.insert("sessionid".to_string(), config.auth.sessionid.clone());
    }

    // 7. 启动 BrowserPool
    let browser_config = eleven_barrage_collector::pool::BrowserPoolConfig {
        pool_size: config.browser.pool_size,
        max_concurrent_per_browser: config.browser.max_concurrent_per_browser,
        sign_timeout: std::time::Duration::from_secs(config.browser.sign_timeout_secs),
        health_check_interval: std::time::Duration::from_secs(
            config.browser.health_check_interval_secs,
        ),
        edge_path: config.browser.edge_path.clone(),
        user_data_dir_template: config.browser.user_data_dir_template.clone(),
        extra_args: config.browser.extra_args.clone(),
        cdp_port_base: config.browser.cdp_port_base,
        auth_cookies: auth_cookies.clone(),
    };
    let browser_pool = eleven_barrage_collector::pool::BrowserPool::start(browser_config)
        .await
        .context("failed to start browser pool")?;

    // 8. 创建动态房间管理器
    let rooms = Arc::new(DynamicRoomManager::new());

    // 9. 启动 REST server
    let rest_addr = config.rest.listen_addr;
    let rest_pool = browser_pool.clone();
    let rest_rooms = rooms.clone();
    let rest_browser = config.browser.clone();
    let rest_cookies = auth_cookies.clone();
    let _rest_handle = tokio::spawn(async move {
        if let Err(e) = rest_server::run_rest_server(
            rest_addr,
            rest_pool,
            rest_rooms,
            rest_browser,
            rest_cookies,
        )
        .await
        {
            tracing::error!(error = %e, "REST server exited");
        }
    });

    // 10. 启动 Watchdog
    let watchdog = Watchdog::default();
    watchdog.start();

    // 11. 启动 WS 下游服务端
    let ws_server = Arc::new(WsServer::new(config.service.ws_listen_addr, rooms.clone()));
    let ws_server_handle = {
        let ws_server = ws_server.clone();
        tokio::spawn(async move {
            if let Err(e) = ws_server.run().await {
                error!(error = %e, "WS server exited with error");
            }
        })
    };

    // 12. 启动 gRPC 服务端
    let grpc_addr = config.service.grpc_listen_addr;
    let (_grpc_tx, grpc_rx) =
        tokio::sync::mpsc::channel::<eleven_barrage_core::BarrageEvent>(1024);
    drop(_grpc_tx);
    let grpc_pool = browser_pool.clone();
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) =
            grpc_server::run_grpc_server_with_pool(grpc_addr, grpc_rx, grpc_pool).await
        {
            error!(error = %e, "gRPC server exited with error");
        }
    });

    // 13. 主循环
    info!(
        "service started, waiting for shutdown signal (POST /v1/rooms to create rooms)"
    );
    let shutdown_reason = wait_for_shutdown().await;
    info!(reason = ?shutdown_reason, "shutdown signal received");

    // 14. 优雅关闭
    info!("shutting down...");
    watchdog.stop().await;
    ws_server_handle.abort();
    grpc_handle.abort();

    info!("shutdown complete");
    Ok(())
}

/// 等待 shutdown 信号（SIGTERM / Ctrl+C）
async fn wait_for_shutdown() -> ShutdownReason {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM signal")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => ShutdownReason::CtrlC,
        _ = terminate => ShutdownReason::Sigterm,
    }
}

#[derive(Debug)]
pub enum ShutdownReason {
    CtrlC,
    Sigterm,
}
