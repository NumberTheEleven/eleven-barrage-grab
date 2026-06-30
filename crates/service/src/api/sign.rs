//! POST /v1/sign handler (auto-signer spec section 5.1)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use eleven_barrage_collector::{parse_url, pool::PoolError, pool::BrowserPool, SignedMaterial, SignedMaterialKind};

#[derive(Deserialize)]
pub struct SignRequest {
    pub url: String,
}

#[derive(Serialize)]
pub struct SignResponse {
    /// 端点类型：`wss` 或 `http_fetch`
    pub kind: String,
    /// 签名后的端点 URL（WSS 或 HTTP fetch）
    pub url: String,
    /// 向后兼容的字段别名，等价于 `url`
    pub wss_url: String,
    pub headers: HashMap<String, String>,
    pub expires_at_unix: i64,
    pub captured_at_unix: i64,
}

impl From<SignedMaterial> for SignResponse {
    fn from(m: SignedMaterial) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let expires = m
            .expires_at()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let kind = match m.kind() {
            SignedMaterialKind::Wss => "wss".to_string(),
            SignedMaterialKind::HttpFetch => "http_fetch".to_string(),
        };
        let url = m.url().to_string();
        Self {
            kind,
            url: url.clone(),
            wss_url: url,
            headers: m.headers().clone(),
            expires_at_unix: expires,
            captured_at_unix: now,
        }
    }
}

#[derive(Debug)]
pub enum ApiError {
    InvalidUrl(String),
    InvalidRequest(String),
    PoolBusy,
    BrowserDead,
    WssTimeout,
    NoSignedEndpointCaptured,
    SignFailed(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, retryable, message) = match self {
            ApiError::InvalidUrl(m) => (StatusCode::BAD_REQUEST, "INVALID_URL", false, m),
            ApiError::InvalidRequest(m) => (StatusCode::BAD_REQUEST, "INVALID_REQUEST", false, m),
            ApiError::PoolBusy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "POOL_BUSY",
                true,
                "all browsers saturated".into(),
            ),
            ApiError::BrowserDead => (
                StatusCode::SERVICE_UNAVAILABLE,
                "BROWSER_DEAD",
                true,
                "browser restarting".into(),
            ),
            ApiError::WssTimeout => (
                StatusCode::BAD_GATEWAY,
                "WSS_TIMEOUT",
                true,
                "timeout waiting for signed endpoint".into(),
            ),
            ApiError::NoSignedEndpointCaptured => (
                StatusCode::BAD_GATEWAY,
                "NO_SIGNED_ENDPOINT_CAPTURED",
                false,
                "room may not exist or be offline".into(),
            ),
            ApiError::SignFailed(m) => (StatusCode::BAD_GATEWAY, "SIGN_FAILED", true, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", true, m),
        };

        let body = serde_json::json!({
            "error": { "code": code, "message": message, "retryable": retryable }
        });
        (status, Json(body)).into_response()
    }
}

pub async fn sign(
    State(pool): State<Arc<BrowserPool>>,
    Json(req): Json<SignRequest>,
) -> Result<Json<SignResponse>, ApiError> {
    let web_rid = parse_url(&req.url).map_err(|e| ApiError::InvalidUrl(e.to_string()))?;

    let material = pool.sign(&web_rid).await.map_err(|e| {
        if e.is_timeout() {
            return ApiError::WssTimeout;
        }
        if e.is_no_signed_endpoint_captured() {
            return ApiError::NoSignedEndpointCaptured;
        }
        match e {
            PoolError::Busy => ApiError::PoolBusy,
            PoolError::Sign(msg) => ApiError::SignFailed(msg),
            PoolError::Browser(_) => ApiError::BrowserDead,
        }
    })?;

    Ok(Json(SignResponse::from(material)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_response_serializes_expected_fields() {
        let resp = SignResponse {
            kind: "http_fetch".into(),
            url: "https://live.douyin.com/webcast/im/fetch/?x=1".into(),
            wss_url: "https://live.douyin.com/webcast/im/fetch/?x=1".into(),
            headers: [("Cookie".into(), "ttwid=y".into())].into_iter().collect(),
            expires_at_unix: 1000,
            captured_at_unix: 500,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["kind"], "http_fetch");
        assert_eq!(json["url"], "https://live.douyin.com/webcast/im/fetch/?x=1");
        assert_eq!(json["wss_url"], "https://live.douyin.com/webcast/im/fetch/?x=1");
        assert_eq!(json["expires_at_unix"], 1000);
        assert_eq!(json["headers"]["Cookie"], "ttwid=y");
    }

    #[test]
    fn api_error_invalid_url_returns_400() {
        let e = ApiError::InvalidUrl("not a url".into());
        let resp = axum::response::IntoResponse::into_response(e);
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn api_error_pool_busy_returns_503() {
        let e = ApiError::PoolBusy;
        let resp = axum::response::IntoResponse::into_response(e);
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn api_error_sign_failed_returns_502() {
        let e = ApiError::SignFailed("timeout".into());
        let resp = axum::response::IntoResponse::into_response(e);
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_GATEWAY);
    }
}
