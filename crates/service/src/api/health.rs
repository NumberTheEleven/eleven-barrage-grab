//! GET /v1/health handler (auto-signer spec section 5.3)

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use eleven_barrage_collector::pool::{BrowserHealth, BrowserPool, PoolHealth};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub pool: PoolHealthView,
    pub browsers: Vec<BrowserHealth>,
}

#[derive(Serialize)]
pub struct PoolHealthView {
    pub size: usize,
    pub ready: usize,
    pub busy: usize,
    pub dead: usize,
}

pub async fn health(
    State(pool): State<Arc<BrowserPool>>,
) -> impl IntoResponse {
    let h: PoolHealth = pool.health().await;
    let healthy = is_healthy(h.ready, h.dead);
    let response = HealthResponse {
        status: if healthy { "ok".into() } else { "degraded".into() },
        pool: PoolHealthView {
            size: h.size,
            ready: h.ready,
            busy: h.busy,
            dead: h.dead,
        },
        browsers: h.browsers,
    };
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

fn is_healthy(_ready: usize, dead: usize) -> bool {
    dead == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_includes_status_field() {
        let r = HealthResponse {
            status: "ok".into(),
            pool: PoolHealthView {
                size: 3,
                ready: 3,
                busy: 0,
                dead: 0,
            },
            browsers: vec![],
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[test]
    fn health_is_unhealthy_if_any_browser_dead() {
        assert!(!is_healthy(0, 1));
        assert!(is_healthy(1, 0));
    }
}
