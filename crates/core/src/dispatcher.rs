//! 消息分发器 — 按 `msg.Method` 字段路由到 8 种具体消息类型
//!
//! 设计要点（参考原项目 `WssBarrageGrab.cs:206-298`）：
//! - 单条消息失败不影响后续消息处理（continue to next message）
//! - 返回 `Vec<BarrageEvent>`，调用方自行处理去重/过滤
//! - 未知 method 静默忽略（原项目 default 分支行为）

use prost::Message;
use tracing::warn;

use eleven_barrage_proto as proto;
use eleven_barrage_proto::{Response, WssResponse};

use crate::error::CoreResult;
use crate::event::{message_method, BarrageEvent};

/// 消息分发器
#[derive(Debug, Clone, Default)]
pub struct Dispatcher;

impl Dispatcher {
    /// 创建新分发器
    pub fn new() -> Self {
        Self
    }

    /// 将 `Response.messages` 中的每条 `Message` 转换为 `BarrageEvent`
    ///
    /// # 参数
    /// - `_wss_response`: 外层 WssResponse（保留用于日志/调试）
    /// - `response`: 解压后的内层 Response
    ///
    /// # 返回
    /// - `Ok(Vec<BarrageEvent>)`：成功转换的事件列表
    /// - `Err(CoreError)`：致命错误（目前不会发生，单条消息失败已忽略）
    ///
    /// # 错误处理
    /// - 单条消息 protobuf 反序列化失败 → 跳过该条，记录 warn 日志
    /// - 未知 message method → 静默忽略（不计入错误）
    pub fn dispatch(
        &self,
        _wss_response: &WssResponse,
        response: &Response,
    ) -> CoreResult<Vec<BarrageEvent>> {
        let mut events = Vec::with_capacity(response.messages.len());

        for msg in &response.messages {
            match self.dispatch_single(msg) {
                Ok(Some(event)) => events.push(event),
                Ok(None) => {
                    // 未知 method，静默忽略
                }
                Err(e) => {
                    warn!(
                        method = %msg.method,
                        msg_id = msg.msg_id,
                        error = %e,
                        "failed to dispatch message, skipping"
                    );
                }
            }
        }

        Ok(events)
    }

    /// 分发单条消息
    fn dispatch_single(&self, msg: &proto::Message) -> CoreResult<Option<BarrageEvent>> {
        let event = match msg.method.as_str() {
            message_method::CHAT => {
                let m = proto::ChatMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::ChatMessage(m)
            }
            message_method::GIFT => {
                let m = proto::GiftMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::GiftMessage(m)
            }
            message_method::LIKE => {
                let m = proto::LikeMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::LikeMessage(m)
            }
            message_method::MEMBER => {
                let m = proto::MemberMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::MemberMessage(m)
            }
            message_method::SOCIAL => {
                let m = proto::SocialMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::SocialMessage(m)
            }
            message_method::CONTROL => {
                let m = proto::ControlMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::ControlMessage(m)
            }
            message_method::ROOM_USER_SEQ => {
                let m = proto::RoomUserSeqMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::RoomUserSeqMessage(m)
            }
            message_method::FANSCLUB => {
                let m = proto::FansclubMessage::decode(msg.payload.as_slice())?;
                BarrageEvent::FansclubMessage(m)
            }
            other => {
                warn!(method = %other, "unknown message method, skipping");
                return Ok(None);
            }
        };
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn dispatch_empty_response() {
        let dispatcher = Dispatcher::new();
        let wss = WssResponse::default();
        let response = Response::default();
        let events = dispatcher.dispatch(&wss, &response).unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn dispatch_chat_message() {
        let dispatcher = Dispatcher::new();

        // 构造一条 ChatMessage
        let chat = proto::ChatMessage {
            content: "hello".to_string(),
            ..Default::default()
        };
        let chat_bytes = chat.encode_to_vec();

        let msg = proto::Message {
            method: message_method::CHAT.to_string(),
            payload: chat_bytes,
            msg_id: 100,
            ..Default::default()
        };

        let response = Response {
            messages: vec![msg],
            ..Default::default()
        };

        let events = dispatcher.dispatch(&WssResponse::default(), &response).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BarrageEvent::ChatMessage(_)));
    }

    #[test]
    fn dispatch_unknown_method_skipped() {
        let dispatcher = Dispatcher::new();
        let msg = proto::Message {
            method: "WebcastUnknownMessage".to_string(),
            payload: vec![],
            ..Default::default()
        };
        let response = Response {
            messages: vec![msg],
            ..Default::default()
        };

        let events = dispatcher.dispatch(&WssResponse::default(), &response).unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn dispatch_corrupted_payload_continues() {
        let dispatcher = Dispatcher::new();

        let good_msg = proto::Message {
            method: message_method::CHAT.to_string(),
            payload: proto::ChatMessage {
                content: "ok".to_string(),
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        };

        let bad_msg = proto::Message {
            method: message_method::CHAT.to_string(),
            payload: vec![0xff; 100], // 损坏的 protobuf
            ..Default::default()
        };

        let response = Response {
            messages: vec![bad_msg, good_msg],
            ..Default::default()
        };

        let events = dispatcher.dispatch(&WssResponse::default(), &response).unwrap();
        // 第一条失败被跳过，第二条成功
        assert_eq!(events.len(), 1);
    }
}