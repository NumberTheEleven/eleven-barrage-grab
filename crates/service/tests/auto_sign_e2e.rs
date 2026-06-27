//! E2E 集成测试：URL → ProvideSignedWss → SignedWssMaterial（R-010）
//!
//! 本测试启动：
//! 1. mock HTTP server（响应 room_info 和 im_fetch）
//! 2. gRPC SignedBarrageService（使用 mock AutoSigner）
//! 3. gRPC 客户端调用 ProvideSignedWss
//!
//! 验证：最终返回 SignedWssMaterial 且 URL 正确。

use std::time::Duration;

use eleven_barrage_collector::{ImFetchConfig, ImFetcher};
use eleven_barrage_service::{
    AutoSigner, RoomApiConfig, RoomInfoApi, SignedBarrageServiceImpl,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 简单 mock HTTP response
struct MockResponse {
    status: u16,
    body: String,
}

impl MockResponse {
    fn ok(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
        }
    }
    fn to_http_bytes(&self) -> String {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            if self.status == 200 { "OK" } else { "Error" },
            self.body.len(),
            self.body
        )
    }
}

/// 启动 mock HTTP server（支持 room_info 和 im_fetch）
async fn start_mock_http_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            let mut buf = vec![0u8; 8192];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };
            let req = String::from_utf8_lossy(&buf[..n]).to_string();

            let resp = if req.contains("/webcast/room/web/enter/") {
                MockResponse::ok(r#"{"data":{"room":{"id_str":"123456789","title":"Test Room","owner":{"nickname":"Tester"},"status":2}}}"#)
            } else if req.contains("/webcast/im/fetch/") {
                MockResponse::ok(r#"{"wss_url":"wss://mock-wss.local/push","headers":{"X-MS-STUB":"stub123"},"expires_at":9999999999}"#)
            } else {
                MockResponse::ok(r#"{}"#)
            };

            let _ = stream.write_all(resp.to_http_bytes().as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    port
}

#[tokio::test]
async fn provide_signed_wss_e2e_returns_material() {
    let http_port = start_mock_http_server().await;

    // 构造 AutoSigner 指向 mock server
    let auth = eleven_barrage_service::config::AuthConfig {
        ttwid: "test_ttwid".to_string(),
        sessionid: String::new(),
    };

    let mut room_api_config = RoomApiConfig::default();
    room_api_config.base_url = format!("http://127.0.0.1:{}", http_port);

    let room_api = RoomInfoApi::new(room_api_config).unwrap();
    let im_config = ImFetchConfig {
        base_url: format!("http://127.0.0.1:{}", http_port),
        ..Default::default()
    };
    let im_fetcher = ImFetcher::new(im_config).unwrap();
    let signer = AutoSigner::new(room_api, im_fetcher, auth);

    // 启动 gRPC 服务
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_port = grpc_listener.local_addr().unwrap().port();
    let service = SignedBarrageServiceImpl::new(signer);
    let server = service.into_server();

    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(server)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(grpc_listener))
            .await;
    });

    // 等待服务启动
    tokio::time::sleep(Duration::from_millis(100)).await;

    // gRPC 客户端调用 ProvideSignedWss
    use eleven_barrage_service::signed_proto::signed_barrage_service_client::SignedBarrageServiceClient;
    use eleven_barrage_service::signed_proto::ProvideSignedWssRequest;

    let mut client = SignedBarrageServiceClient::connect(format!("http://127.0.0.1:{}", grpc_port))
        .await
        .expect("connect to grpc server");

    let request = ProvideSignedWssRequest {
        url: Some("https://live.douyin.com/test_room".to_string()),
        cookie_file: None,
    };

    let response = client
        .provide_signed_wss(tonic::Request::new(request))
        .await
        .expect("grpc call success")
        .into_inner();

    match response.result {
        Some(eleven_barrage_service::signed_proto::provide_signed_wss_response::Result::Material(m)) => {
            assert_eq!(m.url, "wss://mock-wss.local/push");
            assert_eq!(m.headers.get("X-MS-STUB"), Some(&"stub123".to_string()));
            assert_eq!(m.expires_at_unix, 9999999999);
        }
        other => panic!("expected material, got {:?}", other),
    }
}
