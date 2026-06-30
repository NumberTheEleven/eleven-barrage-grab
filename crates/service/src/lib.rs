//! `eleven-barrage-service` — 主服务 daemon
//!
//! 模块组织（dynamic-room-subscription 重构后）：
//! - [`config`]：配置加载
//! - [`logging`]：tracing 初始化
//! - [`metrics`]：Prometheus 指标
//! - [`api`]：REST API（`/v1/sign`、`/v1/health`、`/v1/rooms`）
//! - [`wss`]：WSS 上游连接管理（核心循环）
//! - [`fetch`]：HTTP fetch fallback 上游连接管理
//! - [`dynamic_room`]：动态房间管理器（替代旧的 `room::SingleRoomManager`）
//! - [`collector_spawn`]：动态房间管理器调用的 collector 启动入口
//! - [`ws_server`]：WS 下游服务端（接受 `/rooms/<id>` 路由）
//! - [`ws_path`]：WS 路径解析工具
//! - [`watchdog`]：后台健康监控
//! - [`signer`]：AutoSigner（auto-sign-fetcher 组合 RoomApi + ImFetcher）
//! - [`grpc_signed`]：SignedBarrageService gRPC 实现（auto-sign R-008）

pub mod api;
pub mod collector_spawn;
pub mod config;
pub mod dynamic_room;
pub mod fetch;
pub mod grpc_server;
pub mod grpc_signed;
pub mod logging;
pub mod metrics;
pub mod rest_server;
pub mod run;
pub mod signer;
pub mod watchdog;
pub mod ws_path;
pub mod ws_server;
pub mod wss;

pub use api::{RoomInfo, RoomInfoApi};
pub use config::{AppConfig, AuthConfig, RoomApiConfig};
pub use dynamic_room::{
    DynamicRoomManager, RoomHandle, RoomManagerError, RoomSnapshot, RoomStatus,
};
pub use fetch::FetchConnectionManager;
pub use grpc_signed::{signed_proto, SignedBarrageServiceImpl, SignedBarrageServiceServer};
pub use logging::init as init_logging;
pub use metrics::{MetricsExporter, WssState};
pub use run::run;
pub use signer::AutoSigner;
pub use watchdog::Watchdog;
pub use ws_server::WsServer;
pub use wss::WssConnectionManager;
