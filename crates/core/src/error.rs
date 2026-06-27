//! 错误处理基础设施
//!
//! - 业务错误用 `thiserror` 强类型枚举（`CoreError`）
//! - 顶层错误用 `anyhow::Result` 链式传播
//!
//! 设计参考原项目 `WssBarrageGrab.cs:130-176` 中对无效数据的防御性处理。

use thiserror::Error;

/// core crate 的业务错误类型
#[derive(Error, Debug)]
pub enum CoreError {
    /// 无效的 protobuf wire_type（参考原项目前置校验逻辑）
    ///
    /// 原项目注释：
    /// > Protobuf 有效性前置检查：首字节 wire_type 必须合法 (0-5)
    /// > wire_type 6/7 为保留值，出现则说明数据不可能是 protobuf
    /// > 不增加错误计数，避免误判触发 session 重置
    #[error("invalid wire_type: {0} (must be 0-5)")]
    InvalidWireType(u8),

    /// gzip 解压失败
    #[error("gzip decompress failed: {0}")]
    GzipDecompress(String),

    /// 缺少必需的 headers 字段（如 compress_type != "gzip"）
    #[error("missing or invalid wss header: {0}")]
    InvalidWssHeader(String),

    /// 未知消息 method（msg.Method 不在 8 种已知类型中）
    #[error("unknown message method: {0}")]
    UnknownMethod(String),

    /// protobuf 解码失败
    #[error("protobuf decode failed: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    /// IO 错误
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// core crate 的 Result 别名
pub type CoreResult<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_wire_type_error_display() {
        let err = CoreError::InvalidWireType(7);
        assert_eq!(err.to_string(), "invalid wire_type: 7 (must be 0-5)");
    }

    #[test]
    fn unknown_method_error_display() {
        let err = CoreError::UnknownMethod("WebcastFooBarMessage".to_string());
        assert!(err.to_string().contains("WebcastFooBarMessage"));
    }
}
