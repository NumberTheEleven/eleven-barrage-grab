//! 通用测试 fixture — 构造模拟的 Douyin 弹幕协议数据

use eleven_barrage_core::{ChatMessage, GiftMessage, LikeMessage};
use eleven_barrage_proto::{Message, Response, WssResponse};
use prost::Message as ProstMessage;

#[allow(unused_imports)]
use eleven_barrage_proto as proto;

/// 构造一个不含压缩的完整 WSS 帧（含 N 条消息）
///
/// # 用法
/// ```ignore
/// let frame = build_wss_frame(vec![chat_msg("hello"), gift_msg(...)]);
/// let (wss, response) = decoder.decode(&frame, false)?;
/// ```
pub fn build_wss_frame(messages: Vec<Message>) -> Vec<u8> {
    let response = Response {
        messages,
        ..Default::default()
    };
    let response_bytes = response.encode_to_vec();

    let wss = WssResponse {
        seqid: 1,
        logid: 1,
        service: 0,
        method: 0,
        headers: Default::default(),
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: response_bytes,
    };
    wss.encode_to_vec()
}

/// 构造一个 gzip 压缩的 WSS 帧
pub fn build_gzip_wss_frame(messages: Vec<Message>) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let response = Response {
        messages,
        ..Default::default()
    };
    let response_bytes = response.encode_to_vec();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&response_bytes).unwrap();
    let compressed = encoder.finish().unwrap();

    let mut headers = std::collections::HashMap::new();
    headers.insert("compress_type".to_string(), "gzip".to_string());

    let wss = WssResponse {
        seqid: 1,
        logid: 1,
        service: 0,
        method: 0,
        headers,
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: compressed,
    };
    wss.encode_to_vec()
}

/// 构造一条 ChatMessage
pub fn chat_msg(content: &str, msg_id: i64) -> Message {
    Message {
        method: "WebcastChatMessage".to_string(),
        payload: ChatMessage {
            content: content.to_string(),
            ..Default::default()
        }
        .encode_to_vec(),
        msg_id,
        ..Default::default()
    }
}

/// 构造一条 GiftMessage
pub fn gift_msg(gift_id: i64, count: i64, msg_id: i64) -> Message {
    Message {
        method: "WebcastGiftMessage".to_string(),
        payload: GiftMessage {
            gift_id,
            repeat_count: count,
            ..Default::default()
        }
        .encode_to_vec(),
        msg_id,
        ..Default::default()
    }
}

/// 构造一条 LikeMessage
pub fn like_msg(count: i64, total: i64, msg_id: i64) -> Message {
    Message {
        method: "WebcastLikeMessage".to_string(),
        payload: LikeMessage {
            count,
            total,
            ..Default::default()
        }
        .encode_to_vec(),
        msg_id,
        ..Default::default()
    }
}

/// 构造一条损坏的 protobuf payload（用于测试错误处理）
pub fn corrupted_chat_msg() -> Message {
    Message {
        method: "WebcastChatMessage".to_string(),
        payload: vec![0xff; 100],
        msg_id: 999,
        ..Default::default()
    }
}

/// 构造一条 wire_type = 7 的无效数据（首字节）
pub fn invalid_wire_type_bytes() -> Vec<u8> {
    vec![0x07, 0x00, 0x00, 0x00]
}

/// 构造 N 条 chat 消息（用于性能测试）
#[allow(dead_code)]
pub fn many_chat_msgs(count: usize) -> Vec<Message> {
    (0..count)
        .map(|i| chat_msg(&format!("msg-{}", i), i as i64))
        .collect()
}

// 抑制 unused 警告（部分函数只在测试中用）
#[allow(dead_code)]
fn _ensure_proto_used() {
    let _ = proto::Message::default();
}
