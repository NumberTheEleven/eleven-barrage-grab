//! `/v1/rooms` REST handlers
//!
//! - `POST /v1/rooms` — 创建/复用房间，同步等待签名成功
//! - `DELETE /v1/rooms/{room_id}` — 销毁房间
//! - `GET /v1/rooms` — 列出活跃房间
//!
//! 详见 `devflow/dynamic-room-subscription/requirements.md`、`design.md`

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};

use eleven_barrage_collector::{parse_url, pool::BrowserPool, WebRid};
use eleven_barrage_core::BarrageEvent;

use crate::collector_spawn::{spawn_collector, CollectorContext};
use crate::config::BrowserConfig;
use crate::dynamic_room::{DynamicRoomManager, RoomStatus};

#[derive(Clone)]
pub struct RoomsState {
    pub pool: Arc<BrowserPool>,
    pub rooms: Arc<DynamicRoomManager>,
    pub browser: BrowserConfig,
    pub auth_cookies: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct CreateRoomRequest {
    pub url: String,
}

#[derive(Serialize)]
pub struct RoomInfo {
    pub room_id: String,
    pub url: String,
    pub ws_url: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct ListRoomsResponse {
    pub rooms: Vec<ListRoomEntry>,
}

#[derive(Serialize)]
pub struct ListRoomEntry {
    pub room_id: String,
    pub url: String,
    pub status: String,
    pub client_count: usize,
    pub created_at_unix: i64,
}

#[derive(Debug)]
pub enum RoomsError {
    InvalidUrl(String),
    PoolBusy,
    BrowserDead,
    WssTimeout,
    NoSignedEndpointCaptured(String),
    SignFailed(String),
    Internal(String),
    RoomNotFound,
}

impl IntoResponse for RoomsError {
    fn into_response(self) -> Response {
        let (status, code, message, retryable) = match self {
            RoomsError::InvalidUrl(m) => (StatusCode::BAD_REQUEST, "INVALID_URL", m, false),
            RoomsError::PoolBusy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "POOL_BUSY",
                "all browsers saturated".into(),
                true,
            ),
            RoomsError::BrowserDead => (
                StatusCode::SERVICE_UNAVAILABLE,
                "BROWSER_DEAD",
                "browser restarting".into(),
                true,
            ),
            RoomsError::WssTimeout => (
                StatusCode::BAD_GATEWAY,
                "WSS_TIMEOUT",
                "timeout waiting for signed endpoint".into(),
                true,
            ),
            RoomsError::NoSignedEndpointCaptured(m) => (
                StatusCode::BAD_GATEWAY,
                "NO_SIGNED_ENDPOINT_CAPTURED",
                m,
                false,
            ),
            RoomsError::SignFailed(m) => (StatusCode::BAD_GATEWAY, "SIGN_FAILED", m, true),
            RoomsError::Internal(m) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                m,
                true,
            ),
            RoomsError::RoomNotFound => (
                StatusCode::NOT_FOUND,
                "ROOM_NOT_FOUND",
                "room not found".into(),
                false,
            ),
        };

        let body = serde_json::json!({
            "error": { "code": code, "message": message, "retryable": retryable }
        });
        (status, Json(body)).into_response()
    }
}

/// POST /v1/rooms — 创建/复用房间
pub async fn create_room(
    State(state): State<RoomsState>,
    Json(req): Json<CreateRoomRequest>,
) -> Result<(StatusCode, Json<RoomInfo>), RoomsError> {
    let web_rid: WebRid = parse_url(&req.url).map_err(|e| RoomsError::InvalidUrl(e.to_string()))?;

    let web_rid_str: String = web_rid.clone();

    // 1. 检查 room 是否已存在（幂等）
    if let Some(existing) = state.rooms.get(&web_rid_str) {
        let status = existing.status();
        if matches!(status, RoomStatus::Connected) {
            let info = RoomInfo {
                room_id: existing.web_rid().to_string(),
                url: existing.url().to_string(),
                ws_url: format!("ws://{}/rooms/{}", state_ws_host(&state), existing.web_rid()),
                status: status.as_str().to_string(),
            };
            return Ok((StatusCode::OK, Json(info)));
        }
        // 已经在 connecting / failed：返回错误或继续，下面策略选择"幂等返回现有状态"
        let info = RoomInfo {
            room_id: existing.web_rid().to_string(),
            url: existing.url().to_string(),
            ws_url: format!("ws://{}/rooms/{}", state_ws_host(&state), existing.web_rid()),
            status: status.as_str().to_string(),
        };
        return Ok((StatusCode::ACCEPTED, Json(info)));
    }

    // 2. 走 BrowserPool 签名
    let material = state
        .pool
        .sign(&web_rid_str)
        .await
        .map_err(|e| {
            if e.is_timeout() {
                return RoomsError::WssTimeout;
            }
            if e.is_no_signed_endpoint_captured() {
                return RoomsError::NoSignedEndpointCaptured(
                    "room may not exist or be offline".into(),
                );
            }
            match e {
                eleven_barrage_collector::pool::PoolError::Busy => RoomsError::PoolBusy,
                eleven_barrage_collector::pool::PoolError::Sign(msg) => RoomsError::SignFailed(msg),
                eleven_barrage_collector::pool::PoolError::Browser(_) => RoomsError::BrowserDead,
            }
        })?;

    // 3. 在 RoomManager 中创建房间
    let url_for_room = req.url.clone();
    let rooms_for_callback = state.rooms.clone();
    let auth_cookies = state.auth_cookies.clone();
    let browser = state.browser.clone();
    let _ws_host = state_ws_host(&state);

    let web_rid_owned: String = web_rid_str.clone();
    let handle = state.rooms.create_or_get(&web_rid_owned, &url_for_room, move |h| {
        // on_create：在此启动 collector，并把 task handle 存到 RoomHandle
        let (tx, mut rx) = mpsc::channel::<BarrageEvent>(1024);

        // 构造 CollectorContext
        let ctx = CollectorContext {
            browser_path: PathBuf::from(&browser.edge_path),
            user_data_dir: PathBuf::from(
                browser
                    .user_data_dir_template
                    .replace("{id}", &format!("dynamic-{}", h.web_rid())),
            ),
            cdp_port_base: browser.cdp_port_base,
            extra_args: browser.extra_args.clone(),
            web_rid: web_rid_str.clone(),
            auth_cookies: auth_cookies.clone(),
        };

        // 启动 collector
        let mat = material.clone();
        let collector_handle = spawn_collector(mat, tx.clone(), ctx);
        *h.collector_handle_slot().lock() = Some(collector_handle);

        // 启动事件 pump：把事件投递给房间订阅者，并把 status 推到 Connected
        let rooms_for_pump = rooms_for_callback.clone();
        let web_rid_for_pump = h.web_rid().to_string();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                rooms_for_pump.dispatch(&web_rid_for_pump, event);
            }
            // rx 关闭：把所有订阅者的 channel 全部关闭
            // （已 destroy 调用者负责清空，这里不主动 close）
        });

        // 由于签名是同步调用，到这里可认为已经 connected
        rooms_for_callback.set_status(&web_rid_str, RoomStatus::Connected);
    });

    let info = RoomInfo {
        room_id: handle.web_rid().to_string(),
        url: handle.url().to_string(),
        ws_url: format!("ws://{}/rooms/{}", _ws_host, handle.web_rid()),
        status: handle.status().as_str().to_string(),
    };

    info!(room_id = %handle.web_rid(), "room created");
    Ok((StatusCode::CREATED, Json(info)))
}

/// DELETE /v1/rooms/{room_id}
pub async fn destroy_room(
    State(state): State<RoomsState>,
    Path(room_id): Path<String>,
) -> Result<StatusCode, RoomsError> {
    let result = state.rooms.destroy(&room_id, |h| {
        // 停止 collector task
        if let Some(handle) = h.collector_handle_slot().lock().take() {
            handle.abort();
        }
    });

    match result {
        Ok(()) => {
            info!(room_id = %room_id, "room destroyed");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(_) => {
            warn!(room_id = %room_id, "destroy failed: room not found");
            Err(RoomsError::RoomNotFound)
        }
    }
}

/// GET /v1/rooms
pub async fn list_rooms(
    State(state): State<RoomsState>,
) -> Json<ListRoomsResponse> {
    let snaps = state.rooms.list();
    let entries: Vec<ListRoomEntry> = snaps
        .into_iter()
        .map(|s| ListRoomEntry {
            room_id: s.room_id,
            url: s.url,
            status: s.status.as_str().to_string(),
            client_count: s.client_count,
            created_at_unix: s.created_at_unix,
        })
        .collect();
    Json(ListRoomsResponse { rooms: entries })
}

/// 计算 `ws_url` 的 host（后续可通过配置覆盖）
fn state_ws_host(_state: &RoomsState) -> String {
    // 简化：直接用 127.0.0.1:8888。如果需要外部访问，可注入配置。
    "127.0.0.1:8888".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_room_request_deserializes_url_field() {
        let json = r#"{"url":"https://live.douyin.com/664637748606"}"#;
        let req: CreateRoomRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.url,
            "https://live.douyin.com/664637748606"
        );
    }

    #[test]
    fn list_rooms_response_serializes_correctly() {
        let resp = ListRoomsResponse {
            rooms: vec![ListRoomEntry {
                room_id: "abc".into(),
                url: "https://live.douyin.com/abc".into(),
                status: "connected".into(),
                client_count: 2,
                created_at_unix: 12345,
            }],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["rooms"][0]["room_id"], "abc");
        assert_eq!(json["rooms"][0]["client_count"], 2);
    }

    #[test]
    fn room_info_serializes_to_expected_shape() {
        let info = RoomInfo {
            room_id: "abc".into(),
            url: "https://x".into(),
            ws_url: "ws://127.0.0.1:8888/rooms/abc".into(),
            status: "connected".into(),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["room_id"], "abc");
        assert_eq!(json["ws_url"], "ws://127.0.0.1:8888/rooms/abc");
    }
}
