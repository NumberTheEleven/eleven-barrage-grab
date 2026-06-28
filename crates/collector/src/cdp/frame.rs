//! WebSocket Message ↔ CdpEvent parsing helpers (auto-signer spec section 3.5)

use tokio_tungstenite::tungstenite::Message;

use crate::cdp::commands::{CdpEvent, CdpResponse};
use crate::cdp::error::{CdpError, Result};

#[derive(Debug)]
pub enum ParsedCdpMessage {
    Response(CdpResponse),
    Event(CdpEvent),
    Ignore,
}

/// Parse a WebSocket message into a typed CDP message.
/// Returns `CdpError::Json` if the payload is not valid JSON,
/// or if it doesn't match either Response or Event shape.
pub fn parse_message(msg: Message) -> Result<ParsedCdpMessage> {
    let bytes = match msg {
        Message::Text(t) => t.into_bytes(),
        Message::Binary(b) => b,
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) | Message::Frame(_) => {
            return Ok(ParsedCdpMessage::Ignore);
        }
    };

    let value: serde_json::Value = serde_json::from_slice(&bytes)?;

    if value.get("id").is_some() {
        let resp: CdpResponse = serde_json::from_value(value)?;
        Ok(ParsedCdpMessage::Response(resp))
    } else if value.get("method").is_some() {
        let event: CdpEvent = serde_json::from_value(value)?;
        Ok(ParsedCdpMessage::Event(event))
    } else {
        Ok(ParsedCdpMessage::Ignore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn parse_response_message() {
        let json = serde_json::json!({"id": 1, "result": {"ok": true}}).to_string();
        let msg = Message::Text(json);
        let parsed = parse_message(msg).unwrap();
        match parsed {
            ParsedCdpMessage::Response(r) => assert_eq!(r.id, 1),
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn parse_event_message() {
        let json = serde_json::json!({
            "method": "Network.requestWillBeSent",
            "params": {"requestId": "x", "request": {"url": "wss://x", "method": "GET"}}
        }).to_string();
        let msg = Message::Text(json);
        let parsed = parse_message(msg).unwrap();
        match parsed {
            ParsedCdpMessage::Event(CdpEvent::RequestWillBeSent { .. }) => {}
            _ => panic!("expected RequestWillBeSent event"),
        }
    }

    #[test]
    fn parse_binary_message() {
        let json = serde_json::to_vec(&serde_json::json!({"id": 2, "result": {}})).unwrap();
        let msg = Message::Binary(json);
        let parsed = parse_message(msg).unwrap();
        assert!(matches!(parsed, ParsedCdpMessage::Response(_)));
    }

    #[test]
    fn parse_ping_returns_ignore() {
        let msg = Message::Ping(vec![1, 2, 3]);
        let parsed = parse_message(msg).unwrap();
        assert!(matches!(parsed, ParsedCdpMessage::Ignore));
    }

    #[test]
    fn parse_malformed_json_returns_error() {
        let msg = Message::Text("{not valid json".to_string());
        let result = parse_message(msg);
        assert!(matches!(result, Err(CdpError::Json(_))));
    }
}
