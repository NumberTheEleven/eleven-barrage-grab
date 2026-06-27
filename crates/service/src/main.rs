//! Binary entry point — thin wrapper around `service::run()`
//!
//! 完整启动逻辑在 `service::run()` 中（`src/run.rs`）
//! 这里仅作为二进制入口

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eleven_barrage_service::run().await
}
