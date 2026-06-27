//! 主服务运行入口（可被 main.rs 和 cli 复用）

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use eleven_barrage_core::EventFilter;
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::{
    api::RoomInfoApi,
    config::AppConfig,
    grpc_server,
    logging, metrics::MetricsExporter,
    room::SingleRoomManager,
    signer::AutoSigner,
    watchdog::Watchdog,
    ws_server::WsServer,
    wss::WssConnectionManager,
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

    /// 抖音直播间 web_room_id
    #[arg(long)]
    pub room_id: Option<String>,

    /// WSS URL（覆盖配置）
    #[arg(long)]
    pub wss_url: Option<String>,

    /// 抖音登录态 Cookie（覆盖配置）
    #[arg(long)]
    pub cookie: Option<String>,
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

    // 3. 应用 CLI flags 覆盖（最高优先级）
    if let Some(room_id) = &cli.room_id {
        info!(new = %room_id, "override room_id from CLI");
        config.service.room_id = room_id.clone();
    }
    if let Some(wss_url) = &cli.wss_url {
        info!(new = %wss_url, "override wss_url from CLI");
        config.wss.url = wss_url.clone();
    }
    if let Some(cookie) = &cli.cookie {
        info!("override cookie from CLI");
        config.room_api.cookie = cookie.clone();
    }

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

    // 4. 校验配置
    config.validate().context("configuration validation failed")?;

    // 5. 初始化日志
    logging::init(&config.logging)?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        room_id = %config.service.room_id,
        "starting eleven-barrage-grab"
    );

    // 6. 安装 metrics
    let _metrics_exporter = MetricsExporter::install(&config.service)
        .context("failed to install metrics exporter")?;

    // 7. 启动 Watchdog
    let watchdog = Watchdog::default();
    watchdog.start();

    // 8. 调用房间元数据 API（可选）
    let room_info = if config.room_api.enabled {
        let api = RoomInfoApi::new(config.room_api.clone())
            .context("failed to create room info API client")?;
        match api.get(&config.service.room_id).await {
            Ok(info) => {
                info!(
                    room_id = %info.room_id,
                    title = ?info.title,
                    owner = ?info.owner_nickname,
                    is_live = info.is_live,
                    "room info fetched"
                );
                Some(info)
            }
            Err(e) => {
                warn!(error = %e, "failed to fetch room info, continuing without it");
                None
            }
        }
    } else {
        info!("room info API disabled by config");
        None
    };

    // 9. 创建 EventFilter（基于配置的 push_event_methods）
    let filter = EventFilter::new(config.push_event_methods().to_vec());

    // 10. 创建 WssConnectionManager + SingleRoomManager
    let wss = WssConnectionManager::new(config.wss.clone(), filter);
    let mut room_manager = SingleRoomManager::new(config.service.room_id.clone(), wss.clone());

    // 11. 创建 WS 下游服务端
    let ws_server = Arc::new(WsServer::new(config.service.ws_listen_addr));
    let ws_dispatcher = ws_server.dispatcher();

    // 12. 启动 WS 服务端 task
    let ws_server_handle = {
        let ws_server = ws_server.clone();
        tokio::spawn(async move {
            if let Err(e) = ws_server.run().await {
                error!(error = %e, "WS server exited with error");
            }
        })
    };

    // 13. 启动 gRPC 服务端 task（带上游事件源 + AutoSigner）
    let grpc_addr = config.service.grpc_listen_addr;
    let room_api = RoomInfoApi::new(config.room_api.clone())
        .context("failed to create room info API client")?;
    let im_config = eleven_barrage_collector::ImFetchConfig::default();
    let signer = AutoSigner::from_configs(room_api, im_config, config.auth.clone())?;
    let (grpc_event_tx, grpc_event_rx) = mpsc::channel::<eleven_barrage_core::BarrageEvent>(1024);
    drop(grpc_event_tx);
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = grpc_server::run_grpc_server_with_source_and_signer(
            grpc_addr,
            grpc_event_rx,
            Some(signer),
        )
        .await
        {
            error!(error = %e, "gRPC server exited with error");
        }
    });

    // 14. 启动房间事件 pump（room manager → ws dispatcher）
    // event_rx 已 moved into gRPC server, 所以需要从 room_manager 重新取
    let event_rx2 = room_manager
        .take_event_receiver()
        .ok_or_else(|| anyhow::anyhow!("event receiver already taken"))?;

    let _event_pump = tokio::spawn(async move {
        let mut rx = event_rx2;
        while let Some(event) = rx.recv().await {
            ws_dispatcher.dispatch(event).await;
        }
    });

    // 15. 启动房间拉流
    room_manager
        .start_in_task(room_info)
        .await
        .context("failed to start room manager")?;

    // 16. 主循环：等待 shutdown 信号
    info!("service started, waiting for shutdown signal");
    let shutdown_reason = wait_for_shutdown().await;
    info!(reason = ?shutdown_reason, "shutdown signal received");

    // 17. 优雅关闭
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
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
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