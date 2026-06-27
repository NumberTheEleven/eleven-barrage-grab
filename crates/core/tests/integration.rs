//! 集成测试 — 完整管线（decode → dispatch → filter → dedup）
//!
//! 通过 `mod fixtures` 引用 tests/fixtures.rs

mod fixtures;

use eleven_barrage_core::{
    BarrageEvent, Dispatcher, EventFilter, MsgDedup, SessionFaultDetector, WssDecoder,
};

#[test]
fn full_pipeline_with_chat_message() {
    // 1. 构造 WSS 帧
    let frame = fixtures::build_wss_frame(vec![fixtures::chat_msg("hello", 1)]);

    // 2. 解码
    let decoder = WssDecoder::new();
    let (_wss, response) = decoder.decode(&frame, false).expect("decode failed");

    // 3. 分发
    let dispatcher = Dispatcher::new();
    let events = dispatcher
        .dispatch(&_wss, &response)
        .expect("dispatch failed");
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

#[test]
fn full_pipeline_with_mixed_events() {
    let frame = fixtures::build_wss_frame(vec![
        fixtures::chat_msg("hello", 1),
        fixtures::gift_msg(1001, 5, 2),
        fixtures::like_msg(1, 100, 3),
    ]);

    let decoder = WssDecoder::new();
    let (wss, response) = decoder.decode(&frame, false).expect("decode failed");

    let dispatcher = Dispatcher::new();
    let events = dispatcher.dispatch(&wss, &response).expect("dispatch failed");

    assert_eq!(events.len(), 3);

    // MVP filter: all 3 should pass
    let filter = EventFilter::mvp_default();
    let mvp_events: Vec<_> = events.iter().filter(|e| filter.allows(e)).collect();
    assert_eq!(mvp_events.len(), 3);
}

#[test]
fn gzip_compressed_frame_decodes_correctly() {
    let frame = fixtures::build_gzip_wss_frame(vec![fixtures::chat_msg("gzipped", 1)]);

    let decoder = WssDecoder::new();
    // gzip_hint = true 表示"这是压缩帧，强制尝试解压"
    let (_, response) = decoder
        .decode(&frame, true)
        .expect("gzip decode failed");

    assert_eq!(response.messages.len(), 1);
}

#[test]
fn decoder_rejects_invalid_wire_type() {
    let bytes = fixtures::invalid_wire_type_bytes();
    let decoder = WssDecoder::new();

    let result = decoder.decode(&bytes, false);
    assert!(matches!(
        result,
        Err(eleven_barrage_core::CoreError::InvalidWireType(7))
    ));
}

#[test]
fn dispatcher_continues_after_corrupted_message() {
    let frame = fixtures::build_wss_frame(vec![
        fixtures::corrupted_chat_msg(),
        fixtures::chat_msg("good", 2),
    ]);

    let decoder = WssDecoder::new();
    let (wss, response) = decoder.decode(&frame, false).expect("decode failed");

    let dispatcher = Dispatcher::new();
    let events = dispatcher.dispatch(&wss, &response).expect("dispatch failed");

    // 第一条损坏被跳过，第二条成功
    assert_eq!(events.len(), 1);
}

#[test]
fn session_fault_triggers_after_consecutive_failures() {
    let detector = SessionFaultDetector::new();

    // 模拟 5 次连续 decoder 错误
    for _ in 0..5 {
        detector.record_error();
    }

    // 验证 fault detector 已记录（实际触发需要回调）
    // 这里通过 wait_fault 异步验证
    let detector_clone = detector.clone();
    let handle = tokio::runtime::Runtime::new()
        .unwrap()
        .spawn(async move {
            tokio::time::timeout(std::time::Duration::from_millis(100), detector_clone.wait_fault())
                .await
        });

    let result = handle.unwrap();
    assert!(result.is_ok(), "should be notified of fault");
}

#[test]
fn session_fault_does_not_kill_process() {
    // R-024 验证：连续失败触发 fault 但 process 不死
    // 模拟：5 次错误 → 触发 fault → 但 process 仍可继续
    let detector = SessionFaultDetector::new();

    detector.record_error();
    detector.record_error();
    detector.record_error();
    detector.record_error();
    detector.record_error(); // 触发 fault

    // 立即尝试更多操作（不应 panic）
    detector.record_success();
    assert_eq!(detector.current_error_count(), 0);
}

#[test]
fn ring_buffer_dedup_capacity() {
    let dedup = MsgDedup::new(3);

    dedup.check_and_mark("WebcastChatMessage", 1);
    dedup.check_and_mark("WebcastChatMessage", 2);
    dedup.check_and_mark("WebcastChatMessage", 3);
    assert_eq!(dedup.buffer_size(), 3);

    // 超过容量，evict msgId=1
    dedup.check_and_mark("WebcastChatMessage", 4);
    assert_eq!(dedup.buffer_size(), 3);

    // msgId=1 现在是新的
    assert!(dedup.check_and_mark("WebcastChatMessage", 1));
}

#[test]
fn filter_mvp_excludes_member_social_control() {
    let filter = EventFilter::mvp_default();

    use eleven_barrage_core::{
        ControlMessage, MemberMessage, RoomUserSeqMessage, SocialMessage,
    };

    assert!(!filter.allows(&BarrageEvent::MemberMessage(MemberMessage::default())));
    assert!(!filter.allows(&BarrageEvent::SocialMessage(SocialMessage::default())));
    assert!(!filter.allows(&BarrageEvent::ControlMessage(ControlMessage::default())));
    assert!(!filter.allows(&BarrageEvent::RoomUserSeqMessage(
        RoomUserSeqMessage::default()
    )));
}

#[test]
fn event_method_is_stable() {
    use eleven_barrage_core::{
        ChatMessage, ControlMessage, FansclubMessage, GiftMessage, LikeMessage, MemberMessage,
        RoomUserSeqMessage, SocialMessage,
    };

    // 验证 method 字符串与原项目 msg.Method 完全一致
    assert_eq!(
        BarrageEvent::ChatMessage(ChatMessage::default()).method(),
        "WebcastChatMessage"
    );
    assert_eq!(
        BarrageEvent::GiftMessage(GiftMessage::default()).method(),
        "WebcastGiftMessage"
    );
    assert_eq!(
        BarrageEvent::LikeMessage(LikeMessage::default()).method(),
        "WebcastLikeMessage"
    );
    assert_eq!(
        BarrageEvent::MemberMessage(MemberMessage::default()).method(),
        "WebcastMemberMessage"
    );
    assert_eq!(
        BarrageEvent::SocialMessage(SocialMessage::default()).method(),
        "WebcastSocialMessage"
    );
    assert_eq!(
        BarrageEvent::ControlMessage(ControlMessage::default()).method(),
        "WebcastControlMessage"
    );
    assert_eq!(
        BarrageEvent::RoomUserSeqMessage(RoomUserSeqMessage::default()).method(),
        "WebcastRoomUserSeqMessage"
    );
    assert_eq!(
        BarrageEvent::FansclubMessage(FansclubMessage::default()).method(),
        "WebcastFansclubMessage"
    );
}

#[test]
fn msg_id_extraction() {
    use eleven_barrage_core::{ChatMessage, Common};

    let mut chat = ChatMessage::default();
    chat.common = Some(Common {
        msg_id: 12345,
        ..Default::default()
    });

    let event = BarrageEvent::ChatMessage(chat);
    assert_eq!(event.msg_id(), 12345);
}

#[test]
fn timestamp_extraction() {
    use eleven_barrage_core::{ChatMessage, Common};

    let mut chat = ChatMessage::default();
    chat.common = Some(Common {
        create_time: 1719475200000,
        ..Default::default()
    });

    let event = BarrageEvent::ChatMessage(chat);
    assert_eq!(event.timestamp_ms(), 1719475200000);
}

// 添加异步 runtime 依赖
// 注意：tokio 用于测试中的 spawn/timeout
// Cargo.toml 的 [dev-dependencies] 已配置 tokio with test-util