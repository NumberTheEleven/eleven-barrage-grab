//! Build script for `eleven-barrage-proto`.
//!
//! Compiles `proto/*.proto` files into Rust types using `prost-build`.
//! See: https://docs.rs/prost-build

use std::io::Result;

fn main() -> Result<()> {
    let mut config = prost_build::Config::new();
    // 在生成的代码中派生 serde::Serialize/Deserialize，
    // 便于 JSON 序列化（WS 通道输出）和 JSON 配置解析。
    // 注意：不能用 #[serde(default)] on enum（包括 oneof），所以只 derive。
    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");

    // 编译 .proto 文件（使用自定义 config，不是默认 config）
    config.compile_protos(
        &[
            "proto/wss.proto",
            "proto/messages.proto",
            "proto/signed.proto",
        ],
        &["proto/"],
    )?;

    println!("cargo:rerun-if-changed=proto/wss.proto");
    println!("cargo:rerun-if-changed=proto/messages.proto");
    println!("cargo:rerun-if-changed=proto/signed.proto");
    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
