//! E2E 集成测试：URL → ProvideSignedWss → SignedWssMaterial（R-010）
//!
//! 本测试需要真实 Edge headless 浏览器才能运行 BrowserPool。
//! 在 CI 中使用 `#[ignore]` 标记，仅在手动执行时运行：
//!
//! ```bash
//! cargo test -p eleven-barrage-service -- --ignored
//! ```
//!
//! 单元测试（grpc_signed::tests）覆盖了 URL 解析和 PoolError→SignatureError 映射。

use std::sync::Arc;
use std::time::Duration;

use eleven_barrage_collector::pool::{BrowserPool, BrowserPoolConfig};
use eleven_barrage_service::SignedBarrageServiceImpl;

use tokio::net::TcpListener;

#[tokio::test]
#[ignore = "requires real Edge headless browser"]
async fn provide_signed_wss_e2e_returns_material() {
    // Start BrowserPool with real Edge
    let config = BrowserPoolConfig {
        pool_size: 1,
        max_concurrent_per_browser: 1,
        sign_timeout: Duration::from_secs(15),
        health_check_interval: Duration::from_secs(30),
        ..Default::default()
    };

    let pool = BrowserPool::start(config).await.expect("start browser pool");
    let pool = Arc::new(pool);

    // Start gRPC server
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_port = grpc_listener.local_addr().unwrap().port();
    let service = SignedBarrageServiceImpl::new(pool);
    let server = service.into_server();

    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(server)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // gRPC client call
    use eleven_barrage_service::signed_proto::signed_barrage_service_client::SignedBarrageServiceClient;
    use eleven_barrage_service::signed_proto::ProvideSignedWssRequest;

    let mut client = SignedBarrageServiceClient::connect(format!("http://127.0.0.1:{}", grpc_port))
        .await
        .expect("connect to grpc server");

    let request = ProvideSignedWssRequest {
        url: Some("https://live.douyin.com/741891423654".to_string()),
        cookie_file: None,
    };

    let response = client
        .provide_signed_wss(tonic::Request::new(request))
        .await
        .expect("grpc call success")
        .into_inner();

    match response.result {
        Some(
            eleven_barrage_service::signed_proto::provide_signed_wss_response::Result::Material(m),
        ) => {
            assert!(m.url.starts_with("wss://"));
            assert!(!m.headers.is_empty());
            assert!(m.expires_at_unix > 0);
        }
        other => panic!("expected material, got {:?}", other),
    }
}
