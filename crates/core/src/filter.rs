//! 事件过滤器 — 控制哪些 BarrageEvent 推送给下游
//!
//! MVP 阶段：仅推送 Chat/Gift/Like 三种事件（参考原项目 `AppConfig.json` 的 `pushFilter` 字段）
//! 通过配置可调整推送的事件类型集合

use crate::event::{message_method, BarrageEvent};

/// 事件过滤器
#[derive(Debug, Clone)]
pub struct EventFilter {
    /// 允许推送的事件 method 集合
    allow_methods: Vec<String>,
}

impl EventFilter {
    /// 创建 MVP 默认过滤器（仅 Chat/Gift/Like）
    pub fn mvp_default() -> Self {
        Self {
            allow_methods: message_method::MVP_PUSH.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 自定义过滤器
    pub fn new(allow_methods: Vec<String>) -> Self {
        Self { allow_methods }
    }

    /// 创建"全部允许"过滤器（用于调试 / 内部测试）
    pub fn allow_all() -> Self {
        Self {
            allow_methods: vec![
                message_method::CHAT.to_string(),
                message_method::GIFT.to_string(),
                message_method::LIKE.to_string(),
                message_method::MEMBER.to_string(),
                message_method::SOCIAL.to_string(),
                message_method::CONTROL.to_string(),
                message_method::ROOM_USER_SEQ.to_string(),
                message_method::FANSCLUB.to_string(),
            ],
        }
    }

    /// 判断事件是否允许推送
    pub fn allows(&self, event: &BarrageEvent) -> bool {
        let method = event.method();
        self.allow_methods.iter().any(|m| m == method)
    }

    /// 过滤事件列表，返回允许推送的事件
    pub fn filter<'a>(&self, events: &'a [BarrageEvent]) -> Vec<&'a BarrageEvent> {
        events.iter().filter(|e| self.allows(e)).collect()
    }

    /// 获取当前配置的允许事件 method 列表
    pub fn allowed_methods(&self) -> &[String] {
        &self.allow_methods
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ChatMessage, GiftMessage, LikeMessage, MemberMessage};

    #[test]
    fn mvp_default_filters_correctly() {
        let filter = EventFilter::mvp_default();
        assert!(filter.allows(&BarrageEvent::ChatMessage(ChatMessage::default())));
        assert!(filter.allows(&BarrageEvent::GiftMessage(GiftMessage::default())));
        assert!(filter.allows(&BarrageEvent::LikeMessage(LikeMessage::default())));
        assert!(!filter.allows(&BarrageEvent::MemberMessage(MemberMessage::default())));
    }

    #[test]
    fn custom_filter() {
        let filter = EventFilter::new(vec![message_method::CHAT.to_string()]);
        assert!(filter.allows(&BarrageEvent::ChatMessage(ChatMessage::default())));
        assert!(!filter.allows(&BarrageEvent::GiftMessage(GiftMessage::default())));
    }

    #[test]
    fn allow_all_filter() {
        let filter = EventFilter::allow_all();
        assert!(filter.allows(&BarrageEvent::ChatMessage(ChatMessage::default())));
        assert!(filter.allows(&BarrageEvent::MemberMessage(MemberMessage::default())));
    }

    #[test]
    fn filter_vec() {
        let filter = EventFilter::mvp_default();
        let events = vec![
            BarrageEvent::ChatMessage(ChatMessage::default()),
            BarrageEvent::MemberMessage(MemberMessage::default()),
            BarrageEvent::GiftMessage(GiftMessage::default()),
        ];

        let filtered: Vec<String> = filter
            .filter(&events)
            .iter()
            .map(|e| e.method().to_string())
            .collect();

        assert_eq!(
            filtered,
            vec!["WebcastChatMessage".to_string(), "WebcastGiftMessage".to_string()]
        );
    }
}