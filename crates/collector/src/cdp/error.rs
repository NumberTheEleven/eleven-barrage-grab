//! CDP error types (auto-signer spec section 3.6)

use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CdpError {
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("CDP command timed out after {0:?}")]
    Timeout(Duration),

    #[error("CDP returned error: code={code}, message={message}")]
    Protocol { code: i64, message: String },

    #[error("connection closed unexpectedly")]
    ConnectionClosed,

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CdpError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_error_displays() {
        let e = CdpError::Timeout(Duration::from_secs(5));
        assert_eq!(e.to_string(), "CDP command timed out after 5s");
    }

    #[test]
    fn protocol_error_display() {
        let e = CdpError::Protocol { code: -32000, message: "internal".into() };
        assert_eq!(e.to_string(), "CDP returned error: code=-32000, message=internal");
    }

    #[test]
    fn connection_closed_display() {
        let e = CdpError::ConnectionClosed;
        assert_eq!(e.to_string(), "connection closed unexpectedly");
    }

    #[test]
    fn json_error_display_chains() {
        // Construct a JSON parse error from invalid input
        let bad: std::result::Result<serde_json::Value, _> = serde_json::from_str("{invalid");
        let inner = bad.unwrap_err();
        let wrapped: CdpError = inner.into();
        // Just verify it produces a CDP error string
        assert!(wrapped.to_string().contains("JSON parse error"));
    }
}
