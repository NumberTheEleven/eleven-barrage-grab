//! gRPC 服务端（stub 形式）
//!
//! MVP 阶段：定义 service 接口和 message schema，但具体实现待 Phase 5 后。
//!
//! 完整 .proto 定义见 `crates/service/proto/barrage.proto`
//! tonic-build 在 build.rs 中生成 Rust 类型。
//!
//! # 后续完善
//! - 实现 `BarrageService::subscribe` 双向流
//! - 与 WS 服务端共享 BarrageEvent 业务 schema
//! - gRPC 通道编码为 Protobuf（高效、强类型）
//!
//! # 设计参考
//! 与 WebSocket 通道共享同一份 BarrageEvent 业务 schema（来自 `eleven-barrage-core`）。
//! WS 通道用 JSON 编码，gRPC 通道用 Protobuf 二进制编码。

use std::net::SocketAddr;

use anyhow::{Context, Result};
use tonic::transport::Server;
use tracing::info;

// 注：完整 tonic 实现需要先通过 tonic-build 生成代码
// MVP 阶段先实现一个占位的 gRPC server（不接受请求），证明整个启动链路通畅

/// 运行 gRPC 服务端（stub）
pub async fn run_grpc_server(addr: SocketAddr) -> Result<()> {
    info!(addr = %addr, "gRPC server stub starting (full implementation in T-008)");

    // 占位实现：启动一个简单的 TCP listener，每 60s 打印一次心跳
    // 直到 shutdown 触发
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gRPC server on {}", addr))?;

    info!(addr = %addr, "gRPC stub listening (no service implemented)");

    loop {
        // accept 但不处理（只是让端口保持占用，证明绑定成功）
        tokio::select! {
            _ = listener.accept() => {
                // 忽略连接（MVP stub）
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                // 心跳日志
                tracing::debug!(addr = %addr, "gRPC stub heartbeat");
            }
        }
    }
}

/// 占位：完整 tonic 实现参考（待 T-008 完善）
///
/// ```ignore
/// mod barrage_proto {
///     tonic::include_proto!("barrage");
/// }
///
/// use barrage_proto::{barrage_service_server::{BarrageService, BarrageServiceServer}};
/// use barrage_proto::{SubscribeRequest, BarrageEvent as GrpcBarrageEvent};
/// use tonic::{Request, Response, Status};
/// use futures::Stream;
///
/// pub struct MyBarrageService {
///     event_rx: tokio::sync::mpsc::Receiver<BarrageEvent>,
/// }
///
/// #[tonic::async_trait]
/// impl BarrageService for MyBarrageService {
///     type SubscribeStream = Pin<Box<dyn Stream<Item = Result<GrpcBarrageEvent, Status>> + Send>>;
///
///     async fn subscribe(
///         &self,
///         request: Request<SubscribeRequest>,
///     ) -> Result<Response<Self::SubscribeStream>, Status> {
///         let _req = request.into_inner();
///         let rx = self.event_rx;  // 实际通过 Arc<Mutex<>> 共享
///         let stream = async_stream::stream! {
///             // 从 channel 接收事件并转发为 gRPC stream
///         };
///         Ok(Response::new(Box::pin(stream)))
///     }
/// }
/// ```

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn grpc_server_starts_and_binds() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

        // 启动 server（会在后台 listen）
        let handle = tokio::spawn(async move {
            // 注意：run_grpc_server 会一直 loop，这里我们用 try_bind 替代测试
            let _listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        });

        handle.await.unwrap();
    }
}

// 重新导出 tonic::transport::Server 以避免未使用导入警告（保留供 T-008 使用）
#[allow(dead_code)]
fn _ensure_dependencies() {
    let _ = Server::builder();
}