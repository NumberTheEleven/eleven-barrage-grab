//! `ebg` — 简化命令入口（薄包装，等价于 `eleven-barrage-grab`）
//!
//! 设计目的：
//! - 短命令 `ebg` 便于开发者日常使用
//! - 完整命令 `eleven-barrage-grab` 用于文档和脚本
//!
//! 实际行为：直接调用 `eleven-barrage-service` 的入口点

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // 转发到 service crate 的 main 函数
    // 通过共享二进制入口简化（避免重复实现）
    eleven_barrage_service::run().await
}