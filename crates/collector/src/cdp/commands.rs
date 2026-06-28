//! CDP command/event serde types (auto-signer spec section 3.4)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ===== Commands =====

#[derive(Serialize)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum CdpCommand {
    #[serde(rename = "Target.setDiscoverTargets")]
    SetDiscoverTargets {
        id: i64,
        params: SetDiscoverTargetsParams,
    },
    #[serde(rename = "Target.createTarget")]
    CreateTarget {
        id: i64,
        params: CreateTargetParams,
    },
    #[serde(rename = "Target.closeTarget")]
    CloseTarget {
        id: i64,
        params: CloseTargetParams,
    },
    #[serde(rename = "Page.enable")]
    PageEnable {
        id: i64,
        params: PageEnableParams,
    },
    #[serde(rename = "Page.navigate")]
    PageNavigate {
        id: i64,
        params: NavigateParams,
    },
    #[serde(rename = "Network.enable")]
    NetworkEnable {
        id: i64,
        params: NetworkEnableParams,
    },
    #[serde(rename = "Network.disable")]
    NetworkDisable {
        id: i64,
        params: NetworkDisableParams,
    },
    #[serde(rename = "Runtime.evaluate")]
    RuntimeEvaluate {
        id: i64,
        params: RuntimeEvaluateParams,
    },
    #[serde(rename = "Browser.getVersion")]
    GetVersion { id: i64 },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDiscoverTargetsParams {
    pub discover: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetParams {
    pub url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTargetParams {
    pub target_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEnableParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateParams {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEnableParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDisableParams {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvaluateParams {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
}

// ===== Response =====

#[derive(Deserialize, Debug)]
pub struct CdpResponse {
    pub id: i64,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<CdpErrorBody>,
}

#[derive(Deserialize, Debug)]
pub struct CdpErrorBody {
    pub code: i64,
    pub message: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetResult {
    pub target_id: String,
}

// ===== Events =====

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum CdpEvent {
    #[serde(rename = "Network.requestWillBeSent")]
    RequestWillBeSent { params: RequestWillBeSentParams },

    #[serde(rename = "Target.attachedToTarget")]
    AttachedToTarget { params: AttachedToTargetParams },

    #[serde(rename = "Target.detachedFromTarget")]
    DetachedFromTarget { params: DetachedFromTargetParams },

    #[serde(rename = "Page.loadEventFired")]
    LoadEventFired { params: LoadEventFiredParams },

    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RequestWillBeSentParams {
    pub request_id: String,
    pub request: Request,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Request {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AttachedToTargetParams {
    pub session_id: String,
    pub target_info: TargetInfo,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TargetInfo {
    pub target_id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DetachedFromTargetParams {
    pub session_id: String,
    pub target_id: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoadEventFiredParams {
    pub timestamp: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_create_target_command() {
        let cmd = CdpCommand::CreateTarget {
            id: 1,
            params: CreateTargetParams { url: "about:blank".into() },
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["method"], "Target.createTarget");
        assert_eq!(json["id"], 1);
        assert_eq!(json["params"]["url"], "about:blank");
    }

    #[test]
    fn serialize_page_navigate_with_session() {
        let cmd = CdpCommand::PageNavigate {
            id: 2,
            params: NavigateParams {
                url: "https://example.com".into(),
                session_id: Some("ABC123".into()),
                referrer: Some("https://ref.com".into()),
            },
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["method"], "Page.navigate");
        assert_eq!(json["params"]["sessionId"], "ABC123");
        assert_eq!(json["params"]["referrer"], "https://ref.com");
    }

    #[test]
    fn deserialize_request_will_be_sent_event() {
        let json = serde_json::json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "1",
                "request": {
                    "url": "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=123",
                    "method": "GET",
                    "headers": {
                        "Cookie": "ttwid=abc",
                        "User-Agent": "Mozilla/5.0"
                    }
                }
            }
        });
        let event: CdpEvent = serde_json::from_value(json).unwrap();
        match event {
            CdpEvent::RequestWillBeSent { params } => {
                assert_eq!(params.request.url, "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=123");
                assert_eq!(params.request.headers.get("Cookie"), Some(&"ttwid=abc".to_string()));
            }
            _ => panic!("expected RequestWillBeSent"),
        }
    }

    #[test]
    fn deserialize_unknown_event_returns_unknown_variant() {
        let json = serde_json::json!({
            "method": "SomeNewDomain.someEvent",
            "params": {}
        });
        let event: CdpEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(event, CdpEvent::Unknown));
    }

    #[test]
    fn deserialize_response_with_result() {
        let json = serde_json::json!({
            "id": 5,
            "result": { "targetId": "T1" }
        });
        let resp: CdpResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.id, 5);
        assert_eq!(resp.result.unwrap()["targetId"], "T1");
        assert!(resp.error.is_none());
    }

    #[test]
    fn deserialize_response_with_error() {
        let json = serde_json::json!({
            "id": 5,
            "error": { "code": -32000, "message": "internal error" }
        });
        let resp: CdpResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.error.unwrap().code, -32000);
    }
}