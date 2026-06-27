//! `eleven-barrage-core` — 弹幕解码与路由核心
//!
//! # 模块结构
//!
//! - [`decoder`]：WSS 帧解码（gzip + protobuf）
//! - [`dispatcher`]：按 `msg.Method` 路由到 8 种消息类型
//! - [`event`]：统一的 `BarrageEvent` 类型（下游接口共享 schema）
//! - [`filter`]：MVP 事件过滤（仅 Chat/Gift/Like）
//! - [`dedup`]：msgId 环形缓冲去重
//! - [`session`]：decoder 故障检测（连续失败触发 session 重连）
//! - [`resilience`]：重试/退避策略（指数退避 + 抖动）
//! - [`error`]：业务错误类型
//!
//! # 数据流
//!
//! ```text
//! binary frame
//!     → WssDecoder::decode()
//!     → (WssResponse, Response)
//!     → Dispatcher::dispatch()
//!     → Vec<BarrageEvent>
//!     → EventFilter::filter()         ← MVP: 仅 Chat/Gift/Like
//!     → MsgDedup::check_and_mark()    ← 去重
//!     → 下游推送
//! ```

#![deny(missing_debug_implementations)]
#![warn(clippy::all)]

pub mod decoder;
pub mod dedup;
pub mod dispatcher;
pub mod error;
pub mod event;
pub mod filter;
pub mod resilience;
pub mod session;

// 重新导出常用类型
pub use decoder::WssDecoder;
pub use dedup::MsgDedup;
pub use dispatcher::Dispatcher;
pub use error::{CoreError, CoreResult};
pub use event::{
    message_method, BarrageEvent, ChatMessage, ControlMessage, FansclubMessage, GiftMessage,
    LikeMessage, MemberMessage, RoomUserSeqMessage, SocialMessage,
};
pub use filter::EventFilter;
pub use resilience::{retry_with_backoff, RetryPolicy};
pub use session::{SessionFaultDetector, SessionFaultReason};

// 添加 rand 依赖（resilience 模块使用）
// 注意：rand 是 workspace.dependencies 中未列出的，需要单独加
// 为了不引入新依赖，这里使用一个最小的 LCG 伪随机
mod rand {
    use std::cell::Cell;
    use std::time::SystemTime;

    thread_local! {
        static SEED: Cell<u64> = Cell::new({
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(42)
        });
    }

    pub struct ThreadRng;

    impl ThreadRng {
        pub fn gen_range(&self, range: std::ops::RangeInclusive<u64>) -> u64 {
            SEED.with(|seed| {
                let mut s = seed.get();
                // simple LCG
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                seed.set(s);
                let span = range.end() - range.start() + 1;
                range.start() + (s % span)
            })
        }
    }

    pub fn thread_rng() -> ThreadRng {
        ThreadRng
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use prost::Message;

    #[test]
    fn full_pipeline_chat_message() {
        // 1. 构造 WssResponse（不压缩）
        let chat = ChatMessage {
            content: "test".to_string(),
            ..Default::default()
        };
        let chat_bytes = chat.encode_to_vec();

        let msg = eleven_barrage_proto::Message {
            method: message_method::CHAT.to_string(),
            payload: chat_bytes,
            msg_id: 1,
            ..Default::default()
        };

        let response = eleven_barrage_proto::Response {
            messages: vec![msg],
            ..Default::default()
        };

        let response_bytes = response.encode_to_vec();

        let wss = eleven_barrage_proto::WssResponse {
            payload: response_bytes,
            ..Default::default()
        };

        let frame = wss.encode_to_vec();

        // 2. 解码
        let decoder = WssDecoder::new();
        let (wss_response, response) = decoder.decode(&frame, false).unwrap();

        // 3. 分发
        let dispatcher = Dispatcher::new();
        let events = dispatcher.dispatch(&wss_response, &response).unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BarrageEvent::ChatMessage(_)));

        // 4. 过滤
        let filter = EventFilter::mvp_default();
        assert!(filter.allows(&events[0]));

        // 5. 去重
        let dedup = MsgDedup::new(100);
        let event = &events[0];
        assert!(dedup.check_and_mark(event.method(), event.msg_id()));
        assert!(!dedup.check_and_mark(event.method(), event.msg_id()));
    }
}