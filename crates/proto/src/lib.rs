//! `eleven-barrage-proto` — Protobuf schema for Douyin barrage protocol.
//!
//! 由 `build.rs` 通过 `prost-build` 生成的 protobuf 类型。
//! 字段命名与原项目 (`DouyinBarrageGrab/BarrageGrab/proto/*.proto`) 保持 1:1 对应（snake_case）。
//!
//! # 模块结构
//!
//! - [`wss`]：外层 `WssResponse`（gzip 压缩前的容器）
//! - [`barrage`]：内层 8 种消息类型（Chat/Gift/Like/Member/Social/Control/RoomUserSeq/Fansclub）
//!
//! # 命名约定
//!
//! - `proto` 中字段用 snake_case，Rust 字段名与之一致
//! - 字段命名严格保持原项目，便于逆向对照调试
//!
//! # MVP 范围
//!
//! MVP 阶段推送的 3 种事件：`ChatMessage`、`GiftMessage`、`LikeMessage`。
//! 其余 5 种（Member/Social/Control/RoomUserSeq/Fansclub）保留 schema 但不订阅。

#![allow(clippy::all)]

/// 外层 WSS 帧解码后的 protobuf 类型
pub mod wss {
    include!(concat!(env!("OUT_DIR"), "/wss.v1.rs"));
}

/// 内层消息容器与 8 种消息类型
pub mod barrage {
    include!(concat!(env!("OUT_DIR"), "/barrage.v1.rs"));
}

/// 重新导出常用类型，便于 `use eleven_barrage_proto::*`
pub use barrage::{
    ChatMessage, Common, ControlMessage, FansclubMessage, GiftMessage, GiftStruct, Image,
    LikeMessage, MemberMessage, Message, PublicAreaCommon, Response, RoomUserSeqMessage,
    SocialMessage, User,
};
pub use wss::WssResponse;
