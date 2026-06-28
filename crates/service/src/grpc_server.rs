//! gRPC 服务端 — 完整实现
//!
//! 提供 `BarrageService` 服务：
//! - `Subscribe`: 客户端订阅指定房间的弹幕事件（双向流）
//! - `Health`: 健康检查
//!
//! # 数据流
//! 上游 WssConnectionManager 推送给 mpsc::Sender<CoreBarrageEvent>，
//! 本服务从对应 Receiver 读取，转发为 gRPC stream。

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_stream::try_stream;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::info;

use eleven_barrage_core::BarrageEvent as CoreBarrageEvent;

// 引入 tonic-build 生成的代码
// .proto 文件名 `barrage.proto`，package `barrage`
// tonic::include_proto! 把生成的代码注入到当前模块，因此我们需要手动包一层
// （避免与 eleven_barrage_core::BarrageEvent 同名冲突）
pub mod barrage {
    tonic::include_proto!("barrage");
}

use self::barrage::barrage_service_server::{BarrageService, BarrageServiceServer};
use self::barrage::{
    BarrageEvent as GrpcBarrageEvent, HealthRequest, HealthResponse, SubscribeRequest,
};

/// gRPC 服务实现
pub struct BarrageServiceImpl {
    /// 共享的 CoreBarrageEvent 接收端（来自上游 WssConnectionManager）
    ///
    /// 实际生产环境应使用 `Arc<Mutex<...>>` 共享 channel 池
    /// MVP 阶段每次启动时绑定一个固定的 source channel
    event_source: Arc<Mutex<Option<mpsc::Receiver<CoreBarrageEvent>>>>,
    service_start_time: std::time::Instant,
}

impl std::fmt::Debug for BarrageServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BarrageServiceImpl")
            .field("service_start_time", &self.service_start_time)
            .finish()
    }
}

impl BarrageServiceImpl {
    /// 创建新服务实例
    pub fn new(event_source: mpsc::Receiver<CoreBarrageEvent>) -> Self {
        Self {
            event_source: Arc::new(Mutex::new(Some(event_source))),
            service_start_time: std::time::Instant::now(),
        }
    }
}

#[tonic::async_trait]
impl BarrageService for BarrageServiceImpl {
    type SubscribeStream =
        Pin<Box<dyn Stream<Item = Result<GrpcBarrageEvent, Status>> + Send + Sync>>;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let req = request.into_inner();
        info!(
            room_id = %req.room_id,
            event_types = ?req.event_types,
            "gRPC subscribe request"
        );

        // 取出共享的 event source（一次性）
        let event_rx = {
            let mut guard = self.event_source.lock();
            guard
                .take()
                .ok_or_else(|| Status::unavailable("event source already consumed"))?
        };

        // 转换为 gRPC stream
        let output = try_stream! {
            let mut rx = event_rx;
            while let Some(event) = rx.recv().await {
                let grpc_event = convert_barrage_event_to_grpc(event);
                yield grpc_event;
            }
        };

        Ok(Response::new(Box::pin(output) as Self::SubscribeStream))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let uptime_secs = self.service_start_time.elapsed().as_secs();

        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: uptime_secs.to_string(),
        }))
    }
}

/// 将内部 `CoreBarrageEvent` 转换为 gRPC Protobuf 消息
fn convert_barrage_event_to_grpc(event: CoreBarrageEvent) -> GrpcBarrageEvent {
    GrpcBarrageEvent {
        event_type: event.method().to_string(),
        timestamp_ms: event.timestamp_ms(),
        msg_id: event.msg_id(),
        // payload_json: MVP 阶段为简化，直接 JSON 序列化
        // 注：这里使用 serde_json 序列化 CoreBarrageEvent 本身
        //     而非内部 message type 的 JSON（保证字段命名一致）
        payload_json: serde_json::to_string(&event).unwrap_or_default(),
    }
}

/// 运行 gRPC 服务端（带上游事件源 + BrowserPool）
pub async fn run_grpc_server_with_source_and_signer(
    addr: SocketAddr,
    event_source: mpsc::Receiver<CoreBarrageEvent>,
    pool: Option<std::sync::Arc<eleven_barrage_collector::pool::BrowserPool>>,
) -> Result<()> {
    info!(addr = %addr, "gRPC server starting (with upstream source + signer)");

    let barrage_service = BarrageServiceImpl::new(event_source);
    let server = Server::builder().add_service(BarrageServiceServer::new(barrage_service));

    let server = if let Some(pool) = pool {
        let signed_service = crate::SignedBarrageServiceImpl::new(pool);
        server.add_service(signed_service.into_server())
    } else {
        server
    };

    server.serve(addr).await.context("gRPC server error")?;

    Ok(())
}

/// 运行 gRPC 服务端（旧接口，不带 signer，保持向后兼容）
pub async fn run_grpc_server(addr: SocketAddr) -> Result<()> {
    let (tx, rx) = mpsc::channel::<CoreBarrageEvent>(1024);
    drop(tx);
    run_grpc_server_with_source_and_signer(addr, rx, None).await
}

/// 运行 gRPC 服务端（带上游事件源 + BrowserPool）
pub async fn run_grpc_server_with_pool(
    addr: SocketAddr,
    event_source: mpsc::Receiver<CoreBarrageEvent>,
    pool: std::sync::Arc<eleven_barrage_collector::pool::BrowserPool>,
) -> Result<()> {
    info!(addr = %addr, "gRPC server starting (with upstream source + browser pool)");

    let barrage_service = BarrageServiceImpl::new(event_source);
    let signed_service = crate::SignedBarrageServiceImpl::new(pool);
    let server = Server::builder()
        .add_service(BarrageServiceServer::new(barrage_service))
        .add_service(signed_service.into_server());

    server.serve(addr).await.context("gRPC server error")?;

    Ok(())
}

/// 运行 gRPC 服务端（带上游事件源）
pub async fn run_grpc_server_with_source(
    addr: SocketAddr,
    event_source: mpsc::Receiver<CoreBarrageEvent>,
) -> Result<()> {
    run_grpc_server_with_source_and_signer(addr, event_source, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use eleven_barrage_core::ChatMessage;

    #[test]
    fn convert_chat_message_to_grpc() {
        let event = CoreBarrageEvent::ChatMessage(ChatMessage {
            content: "test".to_string(),
            ..Default::default()
        });

        let grpc_event = convert_barrage_event_to_grpc(event);

        assert_eq!(grpc_event.event_type, "WebcastChatMessage");
        assert!(grpc_event.payload_json.contains("test"));
    }

    #[tokio::test]
    async fn health_check() {
        let (_tx, rx) = mpsc::channel::<CoreBarrageEvent>(1);
        let service = BarrageServiceImpl::new(rx);

        let response = service
            .health(Request::new(HealthRequest {}))
            .await
            .unwrap();
        let health = response.into_inner();
        assert!(health.healthy);
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn subscribe_streams_events() {
        let (tx, rx) = mpsc::channel::<CoreBarrageEvent>(16);
        let service = BarrageServiceImpl::new(rx);

        // 在另一个 task 中发送事件
        tokio::spawn(async move {
            for i in 0..3 {
                let event = CoreBarrageEvent::ChatMessage(ChatMessage {
                    content: format!("msg-{}", i),
                    ..Default::default()
                });
                tx.send(event).await.unwrap();
            }
        });

        // 调用 subscribe
        let request = SubscribeRequest {
            room_id: "test_room".to_string(),
            event_types: vec![],
        };

        let response = service.subscribe(Request::new(request)).await.unwrap();
        let mut stream = response.into_inner();

        // 接收事件
        let mut received = Vec::new();
        use tokio_stream::StreamExt;
        for _ in 0..3 {
            let event = stream.next().await.unwrap().unwrap();
            received.push(event.payload_json);
        }

        assert_eq!(received.len(), 3);
        assert!(received[0].contains("msg-0"));
        assert!(received[2].contains("msg-2"));
    }

    #[tokio::test]
    async fn event_source_consumed_only_once() {
        let (tx, rx) = mpsc::channel::<CoreBarrageEvent>(16);
        let service = BarrageServiceImpl::new(rx);
        drop(tx);

        // 第一次调用：成功
        let request = SubscribeRequest {
            room_id: "test".to_string(),
            event_types: vec![],
        };
        let _ = service
            .subscribe(Request::new(request.clone()))
            .await
            .unwrap();

        // 第二次调用：返回 Unavailable（source 已被消费）
        let result = service.subscribe(Request::new(request)).await;
        match result {
            Err(status) => assert_eq!(status.code(), tonic::Code::Unavailable),
            Ok(_) => panic!("expected Unavailable error on second subscribe"),
        }
    }
}
