//! Build script for `eleven-barrage-service`
//!
//! 通过 tonic-build 将 `proto/barrage.proto` 和 `proto/signed.proto` 编译为 Rust 类型

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(
            &["proto/barrage.proto", "proto/signed.proto"],
            &["proto"],
        )?;

    println!("cargo:rerun-if-changed=proto/barrage.proto");
    println!("cargo:rerun-if-changed=proto/signed.proto");
    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}