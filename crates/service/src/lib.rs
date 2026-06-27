//! `eleven-barrage-service` — 主服务 daemon
//!
//! 模块组织：
//! - [`config`]：配置加载
//! - [`logging`]：tracing 初始化
//! - [`metrics`]：Prometheus 指标
//! - [`api`]：房间元数据 API 客户端
//! - [`wss`]：WSS 上游连接管理（核心循环）
//! - [`room`]：单/多房间管理器
//! - [`ws_server`]：WS 下游服务端
//! - [`watchdog`]：后台健康监控
//! - [`signer`]：AutoSigner（auto-sign-fetcher 组合 RoomApi + ImFetcher）
//! - [`grpc_signed`]：SignedBarrageService gRPC 实现（auto-sign R-008）

pub mod api;
pub mod config;
pub mod grpc_server;
pub mod grpc_signed;
pub mod logging;
pub mod metrics;
pub mod room;
pub mod run;
pub mod signer;
pub mod watchdog;
pub mod ws_server;
pub mod wss;

pub use api::{RoomInfo, RoomInfoApi};
pub use config::AppConfig;
pub use grpc_signed::{SignedBarrageServiceImpl, SignedBarrageServiceServer};
pub use logging::init as init_logging;
pub use metrics::{MetricsExporter, WssState};
pub use room::{RoomManager, SingleRoomManager};
pub use run::run;
pub use signer::AutoSigner;
pub use watchdog::Watchdog;
pub use ws_server::{EventDispatcher, WsServer};
pub use wss::WssConnectionManager;