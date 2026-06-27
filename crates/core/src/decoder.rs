//! WSS 帧解码器
//!
//! 解码流程（参考原项目 `WssBarrageGrab.cs:104-176`）：
//! 1. **wire_type 前置校验**：首字节的低 3 位（wire_type）必须在 0-5 范围
//!    - 6/7 为 protobuf 保留值，说明数据不可能是 protobuf
//!    - 校验失败不增加错误计数（避免误判触发 session 重置）
//! 2. **WssResponse 解码**：外层容器，包含 headers（gzip 标志）+ payload
//! 3. **gzip 解压**（如 headers 中 `compress_type=gzip`）
//! 4. **Response 解码**：内层消息列表
//! 5. **返回** `(wss_response, response)`

use std::io::Read;

use flate2::read::GzDecoder;

use eleven_barrage_proto::{Response, WssResponse};

use crate::error::{CoreError, CoreResult};

/// WSS 帧解码器
#[derive(Debug, Clone, Default)]
pub struct WssDecoder;

impl WssDecoder {
    /// 创建新解码器
    pub fn new() -> Self {
        Self
    }

    /// 解码一个 WSS 二进制帧
    ///
    /// # 参数
    /// - `frame`: WSS 帧原始字节（来自 `tokio-tungstenite` 的 binary message）
    /// - `gzip_hint`: 由调用方提供的 gzip 标志（基于 wss_headers["compress_type"]）
    ///   - 设为 `true` 时强制尝试 gzip 解压（适用于已确认是压缩帧的场景）
    ///   - 设为 `false` 时不进行 gzip 解压
    ///
    /// # 返回
    /// - `Ok((WssResponse, Response))`：解码成功
    /// - `Err(CoreError)`：解码失败
    ///
    /// # 设计要点
    /// - wire_type 校验失败 → 返回 `InvalidWireType` 错误（**不计入** session 错误计数）
    /// - 业务解码失败 → 返回对应错误（计入 session 错误计数）
    pub fn decode(&self, frame: &[u8], gzip_hint: bool) -> CoreResult<(WssResponse, Response)> {
        // 1. 空帧直接返回
        if frame.is_empty() {
            return Err(CoreError::InvalidWssHeader("empty frame".to_string()));
        }

        // 2. wire_type 前置校验（参考原项目逻辑）
        let wire_type = frame[0] & 0x07;
        if wire_type > 5 {
            return Err(CoreError::InvalidWireType(wire_type));
        }

        // 3. 解码外层 WssResponse
        let wss_response = WssResponse::decode(frame)?;

        // 4. 检查 gzip 标志
        let compress_type = wss_response
            .headers
            .get("compress_type")
            .map(|s| s.as_str())
            .unwrap_or("");

        let need_decompress = gzip_hint || compress_type == "gzip";

        // 5. 解压 payload（如果需要）
        let payload = if need_decompress {
            self.decompress_gzip(&wss_response.payload)?
        } else {
            wss_response.payload.clone()
        };

        // 6. 解码内层 Response
        let response = Response::decode(payload.as_slice())?;

        Ok((wss_response, response))
    }

    /// gzip 解压
    fn decompress_gzip(&self, data: &[u8]) -> CoreResult<Vec<u8>> {
        let mut decoder = GzDecoder::new(data);
        let mut out = Vec::with_capacity(data.len() * 2);
        decoder
            .read_to_end(&mut out)
            .map_err(|e| CoreError::GzipDecompress(e.to_string()))?;
        Ok(out)
    }
}

// 重新导出 prost::Message trait 以便用户使用
pub use prost::Message as _Message;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_frame_returns_error() {
        let decoder = WssDecoder::new();
        let result = decoder.decode(&[], false);
        assert!(matches!(
            result,
            Err(CoreError::InvalidWssHeader(_))
        ));
    }

    #[test]
    fn invalid_wire_type_returns_error() {
        let decoder = WssDecoder::new();
        // 首字节 0x07 → wire_type = 7（保留值）
        let result = decoder.decode(&[0x07, 0x00], false);
        assert!(matches!(result, Err(CoreError::InvalidWireType(7))));
    }

    #[test]
    fn decode_valid_wss_response() {
        let decoder = WssDecoder::new();
        // 构造一个最简单的 WssResponse（不压缩）
        let wss = WssResponse {
            seqid: 1,
            logid: 2,
            service: 3,
            method: 4,
            headers: Default::default(),
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: vec![], // 空 payload → Response::decode 返回空 Response
        };
        let bytes = wss.encode_to_vec();
        let result = decoder.decode(&bytes, false);
        assert!(result.is_ok(), "decode failed: {:?}", result.err());
        let (_, response) = result.unwrap();
        assert_eq!(response.messages.len(), 0);
    }
}