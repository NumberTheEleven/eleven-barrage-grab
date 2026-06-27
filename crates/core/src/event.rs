//! 统一的 BarrageEvent 类型 — 下游接口（WS / gRPC）共享的事件 schema
//!
//! 设计要点：
//! - 内部用 protobuf 生成的强类型枚举
//! - WS 通道序列化为 JSON（`{"event_type": "ChatMessage", "data": {...}}`）
//! - gRPC 通道序列化为 Protobuf（tonic 生成代码）
//! - MVP 推送 Chat/Gift/Like 三种；其余 5 种保留 schema 但不订阅
//!
//! 字段命名：与 `messages.proto` 保持一致（snake_case），
//! 序列化时通过 serde 重命名为 lowerCamelCase（WS JSON 兼容前端）。

use serde::{Deserialize, Serialize};

use eleven_barrage_proto as proto;

/// 下游统一的事件类型（强类型枚举）
///
/// 与原项目 `WssBarrageGrab.OnChatMessage/OnLikeMessage/OnGiftMessage/...`
/// 多个独立事件处理器相比，统一为单个枚举便于多路复用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", content = "data")]
pub enum BarrageEvent {
    /// 弹幕消息（MVP 推送）
    ChatMessage(ChatMessage),
    /// 礼物消息（MVP 推送）
    GiftMessage(GiftMessage),
    /// 点赞消息（MVP 推送）
    LikeMessage(LikeMessage),

    /// 以下 5 种 schema 保留但不推送（架构预留）
    /// 不在 `#[serde(...)]` 标记为默认覆盖，避免 JSON 编码时遗漏字段
    #[serde(skip)]
    MemberMessage(MemberMessage),
    #[serde(skip)]
    SocialMessage(SocialMessage),
    #[serde(skip)]
    ControlMessage(ControlMessage),
    #[serde(skip)]
    RoomUserSeqMessage(RoomUserSeqMessage),
    #[serde(skip)]
    FansclubMessage(FansclubMessage),
}

// 类型别名 + serde rename 集中管理，便于将来修改字段名
pub use proto::{
    ChatMessage, GiftMessage, LikeMessage, MemberMessage, SocialMessage, ControlMessage,
    RoomUserSeqMessage, FansclubMessage,
};

/// `MessageType` 字符串常量（与原项目 msg.Method 保持一致）
pub mod message_method {
    pub const CHAT: &str = "WebcastChatMessage";
    pub const GIFT: &str = "WebcastGiftMessage";
    pub const LIKE: &str = "WebcastLikeMessage";
    pub const MEMBER: &str = "WebcastMemberMessage";
    pub const SOCIAL: &str = "WebcastSocialMessage";
    pub const CONTROL: &str = "WebcastControlMessage";
    pub const ROOM_USER_SEQ: &str = "WebcastRoomUserSeqMessage";
    pub const FANSCLUB: &str = "WebcastFansclubMessage";

    /// MVP 推送的 3 种事件类型
    pub const MVP_PUSH: &[&str] = &[CHAT, GIFT, LIKE];
}

impl BarrageEvent {
    /// 返回事件的 method 字符串（与 `msg.Method` 一致）
    pub fn method(&self) -> &'static str {
        match self {
            BarrageEvent::ChatMessage(_) => message_method::CHAT,
            BarrageEvent::GiftMessage(_) => message_method::GIFT,
            BarrageEvent::LikeMessage(_) => message_method::LIKE,
            BarrageEvent::MemberMessage(_) => message_method::MEMBER,
            BarrageEvent::SocialMessage(_) => message_method::SOCIAL,
            BarrageEvent::ControlMessage(_) => message_method::CONTROL,
            BarrageEvent::RoomUserSeqMessage(_) => message_method::ROOM_USER_SEQ,
            BarrageEvent::FansclubMessage(_) => message_method::FANSCLUB,
        }
    }

    /// 返回事件的 msg_id（用于去重）
    pub fn msg_id(&self) -> i64 {
        match self {
            BarrageEvent::ChatMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::GiftMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::LikeMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::MemberMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::SocialMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::ControlMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::RoomUserSeqMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
            BarrageEvent::FansclubMessage(m) => m.common.as_ref().map(|c| c.msg_id).unwrap_or(0),
        }
    }

    /// 返回事件时间戳（毫秒）
    pub fn timestamp_ms(&self) -> i64 {
        match self {
            BarrageEvent::ChatMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::GiftMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::LikeMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::MemberMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::SocialMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::ControlMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::RoomUserSeqMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
            BarrageEvent::FansclubMessage(m) => m.common.as_ref().map(|c| c.create_time).unwrap_or(0),
        }
    }

    /// 是否属于 MVP 推送范围（Chat/Gift/Like）
    pub fn is_mvp_push(&self) -> bool {
        matches!(
            self,
            BarrageEvent::ChatMessage(_) | BarrageEvent::GiftMessage(_) | BarrageEvent::LikeMessage(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_serde_json() {
        let event = BarrageEvent::ChatMessage(ChatMessage {
            content: "主播好棒".to_string(),
            ..Default::default()
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""event_type":"ChatMessage""#));
        assert!(json.contains(r#""content":"主播好棒""#));
    }

    #[test]
    fn event_method_returns_correct_string() {
        let chat = BarrageEvent::ChatMessage(ChatMessage::default());
        assert_eq!(chat.method(), "WebcastChatMessage");

        let gift = BarrageEvent::GiftMessage(GiftMessage::default());
        assert_eq!(gift.method(), "WebcastGiftMessage");
    }

    #[test]
    fn is_mvp_push_predicate() {
        let chat = BarrageEvent::ChatMessage(ChatMessage::default());
        let gift = BarrageEvent::GiftMessage(GiftMessage::default());
        let like = BarrageEvent::LikeMessage(LikeMessage::default());
        let member = BarrageEvent::MemberMessage(MemberMessage::default());

        assert!(chat.is_mvp_push());
        assert!(gift.is_mvp_push());
        assert!(like.is_mvp_push());
        assert!(!member.is_mvp_push());
    }
}