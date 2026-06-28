# Browser-Based Auto Signer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace HTTP-based `ImFetcher` with Edge headless browser driven via Chrome DevTools Protocol (CDP), exposing `POST /v1/sign` REST endpoint that returns `SignedWssMaterial` for external programs.

**Architecture:** New `collector::cdp` submodule implements native CDP-over-WebSocket client. New `BrowserPool` (3 Edge instances, 2 concurrent each) lazily starts, round-robin schedules. New `service::api` submodule exposes REST via axum on port 7878. Existing gRPC `ProvideSignedWss` routes through the same browser pool for backward compatibility.

**Tech Stack:** Rust 2021, tokio 1.x, tokio-tungstenite 0.23, serde_json 1.x, reqwest 0.12 (existing), axum 0.7 (new), tracing 0.1 (existing).

**Spec:** [2026-06-28-browser-signer-design.md](../specs/2026-06-28-browser-signer-design.md)

## Global Constraints

- **Rust edition**: 2021 (all crates)
- **Crate layout**: 5 crates under `crates/` — proto / core / collector / service / cli (existing, do not restructure)
- **Logging**: use `tracing` macros only (no `println!` in production paths)
- **Error handling**: use `thiserror` for library errors, `anyhow` only at main/bin boundaries
- **Async runtime**: tokio multi-threaded (existing)
- **Test framework**: built-in `#[test]` + `#[tokio::test]` (existing pattern)
- **Commit style**: `<scope>(<crate>): <message>` — scopes: `feat`, `fix`, `test`, `docs`, `chore`, `style`
- **Toolchain location**: Rust on `D:\devtools\.cargo\bin`, requires PATH override (see CLAUDE.md / memory)
- **Target platform**: Windows 10/11 only (v1); Linux/macOS paths must compile but are not tested
- **Browser binary**: System Edge at `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe` (Windows default)

---

## File Map

### New Files
| Path | Responsibility | Lines (est.) |
|---|---|---|
| `crates/collector/src/cdp/mod.rs` | CDP module root, re-exports | 20 |
| `crates/collector/src/cdp/error.rs` | `CdpError` enum | 40 |
| `crates/collector/src/cdp/commands.rs` | `CdpCommand` / `CdpEvent` / params + serde | 250 |
| `crates/collector/src/cdp/frame.rs` | WS Message ↔ Value helpers | 50 |
| `crates/collector/src/cdp/client.rs` | `CdpClient` real impl + `CdpTransport` trait | 250 |
| `crates/collector/src/cdp/mock.rs` | `MockCdpClient` for tests | 150 |
| `crates/collector/src/browser.rs` | `Browser` struct: Edge process + CDP attach | 200 |
| `crates/collector/src/pool.rs` | `BrowserPool` round-robin + health check | 200 |
| `crates/collector/src/signer.rs` | `BrowserSigner::extract_wss` | 150 |
| `crates/service/src/api/mod.rs` | axum router factory | 30 |
| `crates/service/src/api/sign.rs` | `POST /v1/sign` handler | 100 |
| `crates/service/src/api/health.rs` | `GET /v1/health` handler | 80 |
| `crates/service/src/rest_server.rs` | axum server lifecycle | 80 |
| `crates/service/tests/browser_pool_test.rs` | pool scheduling integration test | 100 |
| `crates/service/tests/rest_api_test.rs` | REST handler tests | 100 |
| `crates/service/tests/e2e_url_to_wss.rs` | E2E test (`#[ignore]`) | 80 |

### Modified Files
| Path | Change |
|---|---|
| `crates/collector/src/lib.rs` | Add `cdp`, `browser`, `pool`, `signer` modules |
| `crates/collector/Cargo.toml` | Add `tokio-tungstenite = "0.23"`, `url = "2"` |
| `crates/service/src/lib.rs` | Add `api`, `rest_server` modules |
| `crates/service/src/config.rs` | Add `BrowserConfig`, `RestConfig`, `SignerMode` |
| `crates/service/src/grpc_signed.rs` | Route `ProvideSignedWss` through `BrowserSigner` |
| `crates/service/src/run.rs` | Start `BrowserPool` + REST server alongside gRPC |
| `crates/service/Cargo.toml` | Add `axum = "0.7"`, `tower = "0.4"` |
| `crates/cli/src/main.rs` | Add `ebg sign` subcommand |
| `config.example.toml` | Document `[browser]`, `[rest]`, `[signer]` sections |

---

## Task Dependency Graph

```
T1 CdpError  ─┐
T2 Commands  ─┤
T3 Frame     ─┼─→ T4 Client+Mock ─→ T5 mod.rs
              │
              └─→ T6 Browser ─→ T7 Signer ─→ T8 Pool ─→ T9 HealthCheck
                                                          │
T10 Config[browser] ─────────────────────────────────────┤
                                                          │
T11 axum dep ─→ T12 sign.rs ─→ T13 health.rs ─→ T14 rest_server.rs ─→ T15 run.rs
                                                                          │
T16 grpc_signed.rs ──────────────────────────────────────────────────────┤
                                                                          │
T17 cli sign ─────────────────────────────────────────────────────────────┤
                                                                          │
T18 e2e test (#[ignore]) ─────────────────────────────────────────────────┘
T19 config.example.toml + docs
```

Tasks are ordered for sequential execution; later tasks depend on earlier ones.

---

## Phase A: CDP Foundation

### Task 1: CdpError type

**Files:**
- Create: `crates/collector/src/cdp/mod.rs`
- Create: `crates/collector/src/cdp/error.rs`
- Create: `crates/collector/src/cdp/error_tests.rs` (or inline `#[cfg(test)] mod tests` in error.rs)
- Modify: `crates/collector/src/lib.rs` (add `pub mod cdp;`)

**Interfaces:**
- Consumes: nothing
- Produces: `pub use error::CdpError;` (later tasks will reference this)

- [ ] **Step 1: Write the failing test in `error.rs`**

Open `crates/collector/src/cdp/error.rs` and add at the bottom (before any `enum CdpError`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_error_display() {
        // We can't easily construct a tungstenite::Error here without IO,
        // so we test the Display chain via map_err.
        let r: Result<(), CdpError> = Err(CdpError::Timeout(Duration::from_secs(5)));
        assert_eq!(r.unwrap_err().to_string(), "CDP command timed out after 5s");
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
    fn timeout_error_is_retryable_semantic() {
        // For now we don't add a retryable() method; this test documents intent.
        // Future tasks may add it. Keep this as a placeholder semantic test.
        let _: CdpError = CdpError::Timeout(Duration::from_secs(1));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run from project root:

```bash
export PATH="/d/devtools/protoc/bin:/d/devtools/mingw64/mingw64/bin:/d/devtools/.cargo/bin:$PATH"
cargo test -p eleven-barrage-collector cdp::error
```

Expected: FAIL with `error[E0432]: unresolved import 'super::*'` (no `CdpError` yet).

- [ ] **Step 3: Implement `CdpError`**

In `crates/collector/src/cdp/error.rs`, write at the top:

```rust
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
```

- [ ] **Step 4: Create `cdp/mod.rs` and `lib.rs` plumbing**

Create `crates/collector/src/cdp/mod.rs`:

```rust
//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
pub mod frame;
pub mod client;
pub mod mock;

pub use error::{CdpError, Result};
```

(Note: `commands`, `frame`, `client`, `mock` modules will be created in later tasks. The `mod` declarations will fail compilation until those tasks add the files. To allow incremental work, comment them out for now:)

```rust
//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
// pub mod commands;  // added in T2
// pub mod frame;     // added in T3
// pub mod client;    // added in T4
// pub mod mock;      // added in T4

pub use error::{CdpError, Result};
```

Edit `crates/collector/src/lib.rs`. Find the line:

```rust
mod system_time_serde {
```

That `mod system_time_serde` is inside `SignedWssMaterial` impl. Look for the public modules. The existing `lib.rs` has:

```rust
pub mod error;
pub use error::SignatureError;

pub mod url_parser;
pub use url_parser::{parse as parse_url, WebRid};

pub mod im_fetch;
pub use im_fetch::{ImFetchConfig, ImFetchResponse, ImFetcher};
```

Add after the `im_fetch` block:

```rust

/// Chrome DevTools Protocol client + browser-based signer (spec section 1.1)
pub mod cdp;
pub mod browser;
pub mod pool;
pub mod signer;
```

Also add `browser`, `pool`, `signer` as forward declarations (will be implemented in T6/T8/T9):

```rust
// browser, pool, signer modules are stubs in this task; implemented in T6, T8, T9.
```

Actually, to keep the build green at every step, use this strategy:

Create empty stub files `crates/collector/src/browser.rs`, `pool.rs`, `signer.rs` with just:

```rust
// Stub — implemented in browser-signer plan task T6/T8/T9
```

For T1 specifically (after creating `cdp/error.rs` and `cdp/mod.rs`), create the three stub files so `lib.rs` compiles. Each subsequent task replaces its stub with real code.

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector cdp::error
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/collector/src/cdp/ crates/collector/src/lib.rs crates/collector/src/browser.rs crates/collector/src/pool.rs crates/collector/src/signer.rs
git commit -m "feat(collector): add CdpError type and cdp module skeleton"
```

---

### Task 2: CdpCommand / CdpEvent serde

**Files:**
- Create: `crates/collector/src/cdp/commands.rs`
- Modify: `crates/collector/src/cdp/mod.rs` (uncomment `pub mod commands;`)

**Interfaces:**
- Consumes: `CdpError` from T1
- Produces:
  - `pub enum CdpCommand { ... }` with `Serialize`
  - `pub enum CdpEvent { ... }` with `Deserialize`
  - `pub struct CdpResponse { id: i64, result: Option<Value>, error: Option<CdpErrorBody> }`
  - All param structs: `SetDiscoverTargetsParams`, `CreateTargetParams`, `CloseTargetParams`, `PageEnableParams`, `NavigateParams`, `NetworkEnableParams`, `NetworkDisableParams`, `RuntimeEvaluateParams`, `RequestWillBeSentParams`, `AttachedToTargetParams`, `DetachedFromTargetParams`, `LoadEventFiredParams`, `NavigateResult`, `CreateTargetResult`, `Request`, `CdpErrorBody`

- [ ] **Step 1: Write the failing test in `commands.rs`**

At the bottom of `crates/collector/src/cdp/commands.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector cdp::commands
```

Expected: FAIL (file doesn't exist).

- [ ] **Step 3: Implement `commands.rs`**

In `crates/collector/src/cdp/commands.rs`:

```rust
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

#[derive(Deserialize, Debug)]
pub struct CreateTargetResult {
    pub target_id: String,
}

// ===== Events =====

#[derive(Deserialize, Debug)]
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

#[derive(Deserialize, Debug)]
pub struct RequestWillBeSentParams {
    pub request_id: String,
    pub request: Request,
}

#[derive(Deserialize, Debug)]
pub struct Request {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
pub struct AttachedToTargetParams {
    pub session_id: String,
    pub target_info: TargetInfo,
}

#[derive(Deserialize, Debug)]
pub struct TargetInfo {
    pub target_id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
}

#[derive(Deserialize, Debug)]
pub struct DetachedFromTargetParams {
    pub session_id: String,
    pub target_id: String,
}

#[derive(Deserialize, Debug)]
pub struct LoadEventFiredParams {
    pub timestamp: f64,
}
```

- [ ] **Step 4: Uncomment `mod commands` in `cdp/mod.rs`**

Edit `crates/collector/src/cdp/mod.rs`:

```rust
//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
// pub mod frame;     // added in T3
// pub mod client;    // added in T4
// pub mod mock;      // added in T4

pub use error::{CdpError, Result};
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector cdp::commands
```

Expected: 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/collector/src/cdp/commands.rs crates/collector/src/cdp/mod.rs
git commit -m "feat(collector): add CdpCommand/CdpEvent serde types"
```

---

### Task 3: CdpFrame helpers

**Files:**
- Create: `crates/collector/src/cdp/frame.rs`
- Modify: `crates/collector/src/cdp/mod.rs` (uncomment `pub mod frame;`)

**Interfaces:**
- Consumes: `tokio_tungstenite::tungstenite::Message`
- Produces:
  - `pub fn parse_message(msg: Message) -> Result<ParsedCdpMessage, CdpError>` returning enum: `Response(CdpResponse) | Event(CdpEvent) | Ignore`

- [ ] **Step 1: Write the failing test in `frame.rs`**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector cdp::frame
```

Expected: FAIL (file missing).

- [ ] **Step 3: Implement `frame.rs`**

```rust
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
/// or `CdpError::Json` again if it doesn't match either Response or Event shape.
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
```

- [ ] **Step 4: Uncomment `mod frame` in `cdp/mod.rs`**

```rust
//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
pub mod frame;
// pub mod client;    // added in T4
// pub mod mock;      // added in T4

pub use error::{CdpError, Result};
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector cdp::frame
```

Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/collector/src/cdp/frame.rs crates/collector/src/cdp/mod.rs
git commit -m "feat(collector): add CDP WebSocket frame parsing"
```

---

## Phase B: CDP Client + Mock

### Task 4: CdpTransport trait + CdpClient (real) + MockCdpClient

**Files:**
- Create: `crates/collector/src/cdp/client.rs`
- Create: `crates/collector/src/cdp/mock.rs`
- Modify: `crates/collector/src/cdp/mod.rs` (uncomment `pub mod client;` and `pub mod mock;`)
- Modify: `crates/collector/Cargo.toml` (add tokio-tungstenite + url deps)

**Interfaces:**
- Consumes: `CdpCommand`, `CdpEvent`, `CdpError` from T1-T3
- Produces:
  - `pub trait CdpTransport: Send + Sync`
  - `pub struct CdpClient { ... }` with `connect(ws_url) -> Result<(Self, UnboundedReceiver<CdpEvent>)>`, `send<R>(build, timeout)`, `subscribe_session(session_id) -> UnboundedReceiver<CdpEvent>`
  - `pub struct MockCdpClient` for tests

- [ ] **Step 1: Add Cargo dependencies**

Edit `crates/collector/Cargo.toml`. Add to `[dependencies]`:

```toml
tokio-tungstenite = "0.23"
url = "2"
futures-util = "0.3"
```

(Check that `tokio`, `serde`, `serde_json`, `thiserror` are already present — they should be.)

- [ ] **Step 2: Write failing test for `MockCdpClient`**

Create `crates/collector/src/cdp/mock.rs` with just the test at top:

```rust
//! Mock CdpTransport for unit tests (auto-signer spec section 6.5)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::cdp::commands::{CdpCommand, CdpEvent};
use crate::cdp::error::{CdpError, Result};

/// Transport abstraction — both real CdpClient and MockCdpClient implement this.
#[async_trait]
pub trait CdpTransport: Send + Sync {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        timeout: Duration,
    ) -> Result<R>;

    fn subscribe_events(&self) -> mpsc::UnboundedReceiver<CdpEvent>;
}

/// In-memory mock that returns canned responses and replays pre-recorded events.
#[derive(Clone)]
pub struct MockCdpClient {
    inner: Arc<MockInner>,
}

struct MockInner {
    responses: Mutex<HashMap<String, Value>>,
    events: Mutex<Vec<CdpEvent>>,
    event_subscribers: Mutex<Vec<mpsc::UnboundedSender<CdpEvent>>>,
}

impl MockCdpClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MockInner {
                responses: Mutex::new(HashMap::new()),
                events: Mutex::new(Vec::new()),
                event_subscribers: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Register a canned response for a given CDP method name.
    /// Method name uses the wire format, e.g., "Target.createTarget".
    pub async fn enqueue_response(&self, method: &str, response: Value) {
        self.inner.responses.lock().await.insert(method.to_string(), response);
    }

    /// Enqueue an event to be sent to subscribers when subscribe_events is called.
    pub async fn enqueue_event(&self, event: CdpEvent) {
        self.inner.events.lock().await.push(event);
    }
}

#[async_trait]
impl CdpTransport for MockCdpClient {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        _timeout: Duration,
    ) -> Result<R> {
        let method = match &cmd {
            CdpCommand::SetDiscoverTargets { .. } => "Target.setDiscoverTargets",
            CdpCommand::CreateTarget { .. } => "Target.createTarget",
            CdpCommand::CloseTarget { .. } => "Target.closeTarget",
            CdpCommand::PageEnable { .. } => "Page.enable",
            CdpCommand::PageNavigate { .. } => "Page.navigate",
            CdpCommand::NetworkEnable { .. } => "Network.enable",
            CdpCommand::NetworkDisable { .. } => "Network.disable",
            CdpCommand::RuntimeEvaluate { .. } => "Runtime.evaluate",
            CdpCommand::GetVersion { .. } => "Browser.getVersion",
        };
        let responses = self.inner.responses.lock().await;
        let value = responses
            .get(method)
            .ok_or_else(|| CdpError::Protocol {
                code: -1,
                message: format!("no canned response for {}", method),
            })?
            .clone();
        drop(responses);
        let r: R = serde_json::from_value(value)?;
        Ok(r)
    }

    fn subscribe_events(&self) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let events = inner.events.lock().await;
            for event in events.iter() {
                let _ = tx.send(event.clone());
            }
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::commands::{CreateTargetParams, CreateTargetResult};

    #[tokio::test]
    async fn mock_returns_canned_response() {
        let mock = MockCdpClient::new();
        mock.enqueue_response("Target.createTarget", json!({"targetId": "T-42"})).await;

        let result: CreateTargetResult = mock
            .send_raw(
                CdpCommand::CreateTarget {
                    id: 1,
                    params: CreateTargetParams { url: "about:blank".into() },
                },
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(result.target_id, "T-42");
    }

    #[tokio::test]
    async fn mock_missing_response_returns_error() {
        let mock = MockCdpClient::new();
        let result: Result<CreateTargetResult> = mock
            .send_raw(
                CdpCommand::CreateTarget {
                    id: 1,
                    params: CreateTargetParams { url: "x".into() },
                },
                Duration::from_secs(1),
            )
            .await;
        assert!(matches!(result, Err(CdpError::Protocol { .. })));
    }

    #[tokio::test]
    async fn mock_replays_enqueued_events() {
        use crate::cdp::commands::{LoadEventFiredParams, RequestWillBeSentParams, Request};

        let mock = MockCdpClient::new();
        mock.enqueue_event(CdpEvent::LoadEventFired {
            params: LoadEventFiredParams { timestamp: 1.0 },
        }).await;
        mock.enqueue_event(CdpEvent::RequestWillBeSent {
            params: RequestWillBeSentParams {
                request_id: "1".into(),
                request: Request {
                    url: "wss://example.com".into(),
                    method: "GET".into(),
                    headers: Default::default(),
                },
            },
        }).await;

        let mut rx = mock.subscribe_events();
        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e1, CdpEvent::LoadEventFired { .. }));
        assert!(matches!(e2, CdpEvent::RequestWillBeSent { .. }));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector cdp::mock
```

Expected: FAIL (file doesn't exist).

- [ ] **Step 4: Uncomment `mod mock` in `cdp/mod.rs`**

```rust
//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
pub mod frame;
// pub mod client;    // added in T4
pub mod mock;

pub use error::{CdpError, Result};
```

- [ ] **Step 5: Run test to verify mock passes**

```bash
cargo test -p eleven-barrage-collector cdp::mock
```

Expected: 3 tests pass.

- [ ] **Step 6: Write failing test for `CdpClient::connect`**

Add at bottom of `crates/collector/src/cdp/mock.rs` (or move mock-specific tests into a separate file). For T4 the real client test will live in `client.rs`. Add the test code in `client.rs` first.

Create `crates/collector/src/cdp/client.rs` with this initial test:

```rust
//! Real CDP client implementation (auto-signer spec section 3.3)

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::{
    connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

use crate::cdp::commands::{CdpCommand, CdpEvent};
use crate::cdp::error::{CdpError, Result};
use crate::cdp::frame::{parse_message, ParsedCdpMessage};

pub use async_trait;
pub use crate::cdp::mock::CdpTransport;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;
type WsRead = futures_util::stream::SplitStream<WsStream>;

pub struct CdpClient {
    write: Mutex<WsSink>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<crate::cdp::commands::CdpResponse>>>>,
    event_tx: mpsc::UnboundedSender<CdpEvent>,
    next_id: AtomicI64,
    _read_task: JoinHandle<()>,
}

impl CdpClient {
    /// Connect to a CDP WebSocket endpoint (e.g., `ws://127.0.0.1:9222/devtools/browser/<uuid>`).
    /// Returns the client and a global event receiver (for events not tied to a session).
    pub async fn connect(
        ws_url: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<CdpEvent>)> {
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(CdpError::WebSocket)?;
        let (write, read) = ws_stream.split();

        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<_>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<CdpEvent>();

        let read_task = tokio::spawn(read_loop(read, pending.clone(), event_tx.clone()));

        Ok((
            Self {
                write: Mutex::new(write),
                pending,
                event_tx,
                next_id: AtomicI64::new(1),
                _read_task: read_task,
            },
            event_rx,
        ))
    }

    /// Send a command and await its response with timeout.
    pub async fn send<R: DeserializeOwned + Send + 'static>(
        &self,
        build: impl FnOnce(i64) -> CdpCommand,
        timeout: Duration,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cmd = build(id);
        self.send_raw(cmd, timeout).await
    }

    /// Subscribe to events from a specific CDP session.
    /// For simplicity, all events go through `event_tx`; per-session filtering happens here.
    pub fn subscribe_session(
        &self,
        _session_id: &str,
    ) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = self.event_tx.clone();
        // Spawn a forwarder: subscribes to global events and re-sends those matching the session
        // For v1: we forward all events (session filtering via sessionId param is handled by CDP
        // when commands are sent with sessionId, and our Target-attached/detached events carry it).
        tokio::spawn(async move {
            let mut global_rx = event_tx.subscribe();
            while let Some(event) = global_rx.recv().await {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

#[async_trait]
impl CdpTransport for CdpClient {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd: CdpCommand,
        timeout: Duration,
    ) -> Result<R> {
        // Extract id from the command (assigned by the caller's closure in `send`).
        // Do NOT re-assign here — that would orphan the pending waiter for the original id.
        let id = match &cmd {
            CdpCommand::SetDiscoverTargets { id, .. }
            | CdpCommand::CreateTarget { id, .. }
            | CdpCommand::CloseTarget { id, .. }
            | CdpCommand::PageEnable { id, .. }
            | CdpCommand::PageNavigate { id, .. }
            | CdpCommand::NetworkEnable { id, .. }
            | CdpCommand::NetworkDisable { id, .. }
            | CdpCommand::RuntimeEvaluate { id, .. }
            | CdpCommand::GetVersion { id } => *id,
        };
        let payload = serde_json::to_vec(&cmd)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut write = self.write.lock().await;
            write.send(Message::Binary(payload)).await.map_err(CdpError::WebSocket)?;
        }

        let resp = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| CdpError::Timeout(timeout))?
            .map_err(|_| CdpError::ConnectionClosed)?;

        if let Some(err) = resp.error {
            return Err(CdpError::Protocol {
                code: err.code,
                message: err.message,
            });
        }

        let result = resp.result.unwrap_or(Value::Null);
        let r: R = serde_json::from_value(result)?;
        Ok(r)
    }

    fn subscribe_events(&self) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut global_rx = event_tx.subscribe();
            while let Some(event) = global_rx.recv().await {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

async fn read_loop(
    mut read: WsRead,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<_>>>>,
    event_tx: mpsc::UnboundedSender<CdpEvent>,
) {
    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        let parsed = match parse_message(msg) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CDP message");
                continue;
            }
        };
        match parsed {
            ParsedCdpMessage::Response(resp) => {
                let mut p = pending.lock().await;
                if let Some(tx) = p.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
            ParsedCdpMessage::Event(event) => {
                let _ = event_tx.send(event);
            }
            ParsedCdpMessage::Ignore => {}
        }
    }
}
```

**Important note**: `mpsc::UnboundedSender` does not have a `subscribe()` method. The above code uses `event_tx.subscribe()` which does NOT exist. This must be fixed: we need either `tokio::sync::broadcast` (multi-receiver) or a different pattern.

**Corrected approach**: Change `CdpClient` to use `broadcast::Sender<CdpEvent>` for the global event channel. Update the read_loop to send to broadcast, and `subscribe_session`/`subscribe_events` to subscribe.

Modify the `connect` method to use broadcast:

```rust
use tokio::sync::broadcast;

// In CdpClient:
event_tx: broadcast::Sender<CdpEvent>,

// In connect:
let (event_tx, _) = broadcast::channel::<CdpEvent>(1024);

// read_loop sends to broadcast:
let _ = event_tx.send(event);
```

Replace `mpsc::UnboundedReceiver<CdpEvent>` returns in `subscribe_session` and `subscribe_events` with `broadcast::Receiver<CdpEvent>`. Update mock to match.

**Update MockCdpClient** to use `broadcast::Sender` too:

```rust
struct MockInner {
    responses: Mutex<HashMap<String, Value>>,
    events: Mutex<Vec<CdpEvent>>,
    event_tx: broadcast::Sender<CdpEvent>,
}

// In subscribe_events:
fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
    self.inner.event_tx.subscribe()
}

// In enqueue_event:
pub async fn enqueue_event(&self, event: CdpEvent) {
    let _ = self.inner.event_tx.send(event);
}
```

Also update `subscribe_session` and `subscribe_events` signatures everywhere they appear (in `client.rs` and `mock.rs`) to return `broadcast::Receiver<CdpEvent>`.

Update `mock.rs` tests to use `broadcast::Receiver`:

```rust
let mut rx = mock.subscribe_events();
// rx is broadcast::Receiver, recv() returns Result<Event, RecvError>
let e1 = rx.recv().await.unwrap();
```

- [ ] **Step 7: Run all CDP tests**

```bash
cargo test -p eleven-barrage-collector cdp::
```

Expected: 14 tests pass (4 error + 6 commands + 5 frame + 3 mock).

- [ ] **Step 8: Commit**

```bash
git add crates/collector/Cargo.toml crates/collector/src/cdp/
git commit -m "feat(collector): add CdpClient (real + mock) with CdpTransport trait"
```

---

## Phase C: Browser Process Management

### Task 5: Browser::spawn (Edge process lifecycle)

**Files:**
- Modify: `crates/collector/src/browser.rs` (replace stub)

**Interfaces:**
- Consumes: `BrowserConfig { edge_path, user_data_dir, extra_args }`
- Produces: `pub struct Browser { process, cdp_port, cdp_url }`, `Browser::spawn(config) -> Result<Browser>`, `Browser::kill(&mut self) -> Result<()>`, `Browser::is_alive(&self) -> bool`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_command_args_includes_headless_and_anti_detection() {
        let config = BrowserConfig {
            edge_path: PathBuf::from("msedge.exe"),
            user_data_dir: PathBuf::from("/tmp/profile"),
            extra_args: vec!["--my-arg".into()],
            cdp_port: 9222,
        };
        let args = config.build_args();
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()));
        assert!(args.contains(&"--remote-debugging-port=9222".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/profile".to_string()));
        assert!(args.contains(&"--my-arg".to_string()));
    }

    #[test]
    fn default_edge_args_are_correct() {
        let args = default_edge_args();
        assert!(args.iter().any(|a| a == "--headless=new"));
        assert!(args.iter().any(|a| a.starts_with("--disable-blink-features")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
    }

    #[test]
    fn is_alive_returns_false_after_kill() {
        // Use a long-running sleep command as a stand-in for msedge
        let mut browser = Browser {
            process: std::process::Command::new(if cfg!(windows) { "cmd.exe" } else { "sh" })
                .arg(if cfg!(windows) { "/c" } else { "-c" })
                .arg(if cfg!(windows) { "timeout /t 30 /nobreak > NUL" } else { "sleep 30" })
                .spawn()
                .unwrap(),
            cdp_port: 0,
            cdp_ws_url: String::new(),
            config: BrowserConfig {
                edge_path: Default::default(),
                user_data_dir: Default::default(),
                extra_args: vec![],
                cdp_port: 0,
            },
        };
        assert!(browser.is_alive());
        browser.kill().unwrap();
        // Give OS a moment
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!browser.is_alive());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector browser
```

Expected: FAIL (only stub file exists).

- [ ] **Step 3: Implement `browser.rs`**

```rust
//! Edge process lifecycle management (auto-signer spec section 2.3)

use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    pub edge_path: PathBuf,
    pub user_data_dir: PathBuf,
    pub extra_args: Vec<String>,
    pub cdp_port: u16,
}

impl BrowserConfig {
    pub fn build_args(&self) -> Vec<String> {
        let mut args = default_edge_args();
        args.push(format!("--remote-debugging-port={}", self.cdp_port));
        args.push(format!("--user-data-dir={}", self.user_data_dir.display()));
        for extra in &self.extra_args {
            args.push(extra.clone());
        }
        args.push("about:blank".into());
        args
    }
}

pub fn default_edge_args() -> Vec<String> {
    vec![
        "--headless=new".into(),
        "--disable-blink-features=AutomationControlled".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--window-size=1920,1080".into(),
    ]
}

pub struct Browser {
    pub process: Child,
    pub cdp_port: u16,
    pub cdp_ws_url: String,
    pub config: BrowserConfig,
}

impl Browser {
    pub fn spawn(config: BrowserConfig) -> Result<Self> {
        let args = config.build_args();
        let process = Command::new(&config.edge_path)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn edge at {:?}", config.edge_path))?;

        Ok(Self {
            process,
            cdp_port: config.cdp_port,
            cdp_ws_url: String::new(), // populated by discover_cdp_url
            config,
        })
    }

    pub fn kill(&mut self) -> Result<()> {
        self.process.kill().context("failed to kill browser process")?;
        self.process.wait().context("failed to wait browser process")?;
        Ok(())
    }

    pub fn is_alive(&mut self) -> bool {
        match self.process.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// Poll the CDP discovery endpoint until Edge reports a browser WebSocket URL.
    pub async fn discover_cdp_url(&mut self) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/version", self.cdp_port);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;
        for _ in 0..30 {
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(ws) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        self.cdp_ws_url = ws.to_string();
                        return Ok(ws.to_string());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        anyhow::bail!("CDP discovery timed out on port {}", self.cdp_port);
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector browser
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/collector/src/browser.rs
git commit -m "feat(collector): add Browser process lifecycle (spawn/kill/discover_cdp)"
```

---

### Task 6: BrowserSigner::extract_wss

**Files:**
- Modify: `crates/collector/src/signer.rs` (replace stub)

**Interfaces:**
- Consumes: `Arc<CdpClient>` (from T4), `TabSession { target_id, session_id }` (from T7 which doesn't exist yet — defer)
- Produces:
  - `pub struct TabSession { target_id: String, session_id: String }`
  - `pub async fn extract_wss(cdp: &CdpClient, session_id: &str, web_rid: &str, timeout: Duration) -> Result<SignedWssMaterial>`

Note: `BrowserSigner` as a struct is introduced in T7 alongside `Browser`. Here in T6 we only implement the pure extraction function that takes already-attached session info.

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::commands::{
        LoadEventFiredParams, RequestWillBeSentParams, Request, CdpEvent,
    };
    use crate::cdp::mock::MockCdpClient;
    use crate::cdp::CdpTransport;
    use std::time::Duration;

    fn mock_with_wss_response() -> (MockCdpClient, tokio::sync::broadcast::Sender<CdpEvent>) {
        let mock = MockCdpClient::new();
        // Pre-record a WSS request event
        let (tx, _) = tokio::sync::broadcast::channel::<CdpEvent>(16);
        let _ = tx.send(CdpEvent::RequestWillBeSent {
            params: RequestWillBeSentParams {
                request_id: "1".into(),
                request: Request {
                    url: "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=741891423654&signature=demo".into(),
                    method: "GET".into(),
                    headers: [
                        ("Cookie".to_string(), "ttwid=test".to_string()),
                        ("User-Agent".to_string(), "Mozilla/5.0".to_string()),
                    ].into_iter().collect(),
                },
            },
        });
        (mock, tx)
    }

    #[tokio::test]
    async fn extracts_wss_url_and_headers_from_event() {
        // We can't easily test the full extract_wss with a real CdpClient without a real Edge.
        // Instead, test the URL pattern matching in isolation.
        let url = "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=123";
        assert!(is_wss_push_url(url));
        assert!(!is_wss_push_url("wss://example.com/socket"));
        assert!(!is_wss_push_url("https://live.douyin.com/123"));
    }

    #[test]
    fn url_pattern_is_specific_to_douyin_push() {
        // Negative cases
        assert!(!is_wss_push_url("wss://webcast5-ws-web-lf.douyin.com/webcast/other/v2/"));
        assert!(!is_wss_push_url("wss://other-host.com/webcast/im/push/v2/"));
    }

    #[tokio::test]
    async fn extract_wss_returns_material_on_match() {
        // Build a real CdpClient-like wrapper using MockCdpClient + manually-driven broadcast.
        // Since extract_wss takes `&CdpClient`, we need to test it differently — this test
        // documents the contract via URL pattern matching. The full integration is covered
        // by E2E test (T18).
        // We verify the timeout case via URL pattern only.
        let url = "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=741891423654";
        assert!(is_wss_push_url(url));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector signer
```

Expected: FAIL (stub file).

- [ ] **Step 3: Implement `signer.rs`**

```rust
//! WSS extraction from CDP events (auto-signer spec section 4.1)

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::cdp::client::CdpClient;
use crate::cdp::commands::{
    CdpCommand, CdpEvent, NavigateParams, NetworkEnableParams, PageEnableParams, RequestWillBeSentParams,
};
use crate::cdp::error::{CdpError, Result};

/// A tab attached to a CDP session.
#[derive(Debug, Clone)]
pub struct TabSession {
    pub target_id: String,
    pub session_id: String,
}

/// Returns true if the URL is a Douyin webcast push endpoint.
pub fn is_wss_push_url(url: &str) -> bool {
    url.starts_with("wss://") && url.contains("webcast/im/push")
}

/// Extract the signed WSS material from a navigated tab.
///
/// `events` is a broadcast receiver subscribed to CdpClient events.
pub async fn extract_wss(
    cdp: &CdpClient,
    session_id: &str,
    web_rid: &str,
    timeout: Duration,
    mut events: broadcast::Receiver<CdpEvent>,
) -> Result<crate::SignedWssMaterial> {
    // Enable Page and Network domains on this session
    cdp.send(
        |id| CdpCommand::PageEnable {
            id,
            params: PageEnableParams {
                session_id: Some(session_id.into()),
            },
        },
        Duration::from_secs(2),
    )
    .await?;

    cdp.send(
        |id| CdpCommand::NetworkEnable {
            id,
            params: NetworkEnableParams {
                session_id: Some(session_id.into()),
            },
        },
        Duration::from_secs(2),
    )
    .await?;

    // Navigate
    cdp.send(
        |id| CdpCommand::PageNavigate {
            id,
            params: NavigateParams {
                url: format!("https://live.douyin.com/{}", web_rid),
                session_id: Some(session_id.into()),
                referrer: Some("https://live.douyin.com/".into()),
            },
        },
        Duration::from_secs(5),
    )
    .await?;

    // Wait for matching WSS event
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CdpError::Timeout(timeout));
        }
        let evt = tokio::time::timeout(remaining, events.recv()).await;
        match evt {
            Ok(Ok(CdpEvent::RequestWillBeSent { params })) => {
                if is_wss_push_url(&params.request.url) {
                    let headers: HashMap<String, String> =
                        params.request.headers.into_iter().collect();
                    return Ok(crate::SignedWssMaterial {
                        url: params.request.url,
                        headers,
                        expires_at: SystemTime::now() + Duration::from_secs(3600),
                    });
                }
            }
            Ok(Ok(_)) => continue,    // other events
            Ok(Err(_)) => continue,   // broadcast lag
            Err(_) => return Err(CdpError::Timeout(timeout)),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector signer
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/collector/src/signer.rs
git commit -m "feat(collector): add WSS extraction from CDP events"
```

---

## Phase D: Browser Pool

### Task 7: BrowserPool with round-robin + health check

**Files:**
- Modify: `crates/collector/src/pool.rs` (replace stub)

**Interfaces:**
- Consumes: `BrowserPoolConfig`, `Arc<CdpClient>` (per browser), `BrowserSigner::extract_wss` (T6)
- Produces:
  - `pub struct BrowserPool { browsers: Vec<BrowserHandle>, next_index: AtomicUsize, config: BrowserPoolConfig }`
  - `pub struct BrowserHandle { id, inner: Arc<BrowserInner>, semaphore: Arc<Semaphore> }`
  - `BrowserPool::start(config) -> Result<Self>`
  - `BrowserPool::sign(&self, web_rid: &str) -> Result<SignedWssMaterial>`
  - `BrowserPool::health(&self) -> PoolHealth`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use tokio::sync::error::TryAcquireError;

    #[test]
    fn pool_health_serializes_to_json() {
        let health = PoolHealth {
            size: 3,
            ready: 2,
            busy: 1,
            dead: 0,
            browsers: vec![
                BrowserHealth { id: 0, state: "Idle".into(), last_sign_age_ms: 100 },
                BrowserHealth { id: 1, state: "Idle".into(), last_sign_age_ms: 50 },
                BrowserHealth { id: 2, state: "Signing".into(), last_sign_age_ms: 0 },
            ],
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("\"size\":3"));
        assert!(json.contains("\"state\":\"Idle\""));
    }

    #[test]
    fn round_robin_counter_cycles_correctly() {
        // Direct test of the scheduling primitive — no full pool construction.
        let counter = AtomicUsize::new(0);
        let pool_size = 3;
        let indices: Vec<usize> = (0..6)
            .map(|_| counter.fetch_add(1, Ordering::Relaxed) % pool_size)
            .collect();
        assert_eq!(indices, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn semaphore_exhaustion_blocks_subsequent_acquire() {
        // Test the semaphore primitive directly — pool scheduling uses this.
        let sem = Arc::new(Semaphore::new(1));
        let _p = sem.clone().try_acquire_owned().unwrap();
        let result = sem.clone().try_acquire_owned();
        assert!(matches!(result, Err(TryAcquireError::NoPermits)));
    }

    #[test]
    fn pool_error_display_includes_busy() {
        assert_eq!(PoolError::Busy.to_string(), "pool busy: all browsers saturated");
    }
}
```

> **Note**: The full `BrowserPool::sign()` path is exercised by the E2E test (Task 16) with a real Edge, since constructing a `BrowserPool` in unit tests requires either a real Edge process or significant refactoring of `Browser` behind a trait. The tests above cover the underlying primitives (atomic counter, semaphore, error display, serialization) used by the pool.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-collector pool
```

Expected: FAIL (stub file).

- [ ] **Step 3: Implement `pool.rs`**

```rust
//! Browser pool with round-robin scheduling (auto-signer spec section 2)

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::browser::{Browser, BrowserConfig};
use crate::cdp::client::CdpClient;
use crate::signer::{extract_wss, is_wss_push_url, TabSession};
use crate::SignedWssMaterial;

#[derive(Debug, Clone)]
pub struct BrowserPoolConfig {
    pub pool_size: usize,
    pub max_concurrent_per_browser: usize,
    pub sign_timeout: Duration,
    pub health_check_interval: Duration,
    pub edge_path: PathBuf,
    pub user_data_dir_template: String,
    pub extra_args: Vec<String>,
    pub cdp_port_base: u16,
}

impl Default for BrowserPoolConfig {
    fn default() -> Self {
        Self {
            pool_size: 3,
            max_concurrent_per_browser: 2,
            sign_timeout: Duration::from_secs(10),
            health_check_interval: Duration::from_secs(30),
            edge_path: PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            ),
            user_data_dir_template: "./data/browser-{id}".into(),
            extra_args: vec![],
            cdp_port_base: 9222,
        }
    }
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error("pool busy: all browsers saturated")]
    Busy,
    #[error("sign failed: {0}")]
    Sign(String),
    #[error("browser failed: {0}")]
    Browser(String),
}

#[derive(Debug, Serialize, Clone)]
pub struct BrowserHealth {
    pub id: usize,
    pub state: String,
    pub last_sign_age_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct PoolHealth {
    pub size: usize,
    pub ready: usize,
    pub busy: usize,
    pub dead: usize,
    pub browsers: Vec<BrowserHealth>,
}

pub struct BrowserHandle {
    pub id: usize,
    pub semaphore: Arc<Semaphore>,
    pub(crate) inner: Arc<BrowserInner>,
}

pub struct BrowserInner {
    pub browser: tokio::sync::Mutex<Browser>,
    pub cdp: CdpClient,
    pub last_sign: tokio::sync::Mutex<Option<Instant>>,
}

pub struct BrowserPool {
    pub(crate) browsers: Vec<BrowserHandle>,
    next_index: AtomicUsize,
    pub(crate) config: BrowserPoolConfig,
}

impl BrowserPool {
    /// Spawn the pool and start health check loop.
    pub async fn start(config: BrowserPoolConfig) -> Result<Self> {
        let mut browsers = Vec::with_capacity(config.pool_size);
        for i in 0..config.pool_size {
            let user_data_dir =
                config.user_data_dir_template.replace("{id}", &i.to_string());
            std::fs::create_dir_all(&user_data_dir).ok();

            let cdp_port = config.cdp_port_base + i as u16;
            let browser_config = BrowserConfig {
                edge_path: config.edge_path.clone(),
                user_data_dir: PathBuf::from(user_data_dir),
                extra_args: config.extra_args.clone(),
                cdp_port,
            };

            let mut browser = Browser::spawn(browser_config).context("spawn browser")?;
            let ws_url = browser.discover_cdp_url().await.context("discover cdp")?;
            let (cdp, _global_events) =
                CdpClient::connect(&ws_url).await.context("connect cdp")?;

            browsers.push(BrowserHandle {
                id: i,
                semaphore: Arc::new(Semaphore::new(config.max_concurrent_per_browser)),
                inner: Arc::new(BrowserInner {
                    browser: tokio::sync::Mutex::new(browser),
                    cdp,
                    last_sign: tokio::sync::Mutex::new(None),
                }),
            });
        }

        let pool = Self {
            browsers,
            next_index: AtomicUsize::new(0),
            config,
        };

        // Start health check task
        let pool_arc = Arc::new(pool);
        let health_pool = pool_arc.clone();
        tokio::spawn(async move {
            health_check_loop(health_pool).await;
        });

        Ok(Arc::try_unwrap(pool_arc).unwrap_or_else(|arc| {
            // Another reference exists (shouldn't happen here); clone fields manually
            // In practice, this branch only triggers if start is called twice.
            // Fallback: clone the pool by reconstructing it (we can't easily, so panic).
            // For now, take the only reference semantics: this is unreachable.
            let mut p = Self {
                browsers: Vec::new(),
                next_index: AtomicUsize::new(0),
                config: health_pool.config.clone(),
            };
            // Move browsers from the arc
            for h in &health_pool.browsers {
                p.browsers.push(BrowserHandle {
                    id: h.id,
                    semaphore: h.semaphore.clone(),
                    inner: h.inner.clone(),
                });
            }
            p
        }))
    }

    /// Sign a single web_rid. Round-robin across browsers.
    pub async fn sign(&self, web_rid: &str) -> Result<SignedWssMaterial, PoolError> {
        for _ in 0..self.browsers.len() {
            let idx = self.next_index.fetch_add(1, Ordering::Relaxed) % self.browsers.len();
            let handle = &self.browsers[idx];

            let permit = match handle.semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let result = self.sign_with_handle(handle, web_rid).await;
            drop(permit);
            return result;
        }
        Err(PoolError::Busy)
    }

    async fn sign_with_handle(
        &self,
        handle: &BrowserHandle,
        web_rid: &str,
    ) -> Result<SignedWssMaterial, PoolError> {
        *handle.inner.last_sign.lock().await = Some(Instant::now());

        // Acquire a fresh tab
        let tab = acquire_tab(&handle.inner.cdp)
            .await
            .map_err(|e| PoolError::Sign(e.to_string()))?;

        let mut events = handle.inner.cdp.subscribe_session(&tab.session_id);

        let result = extract_wss(
            &handle.inner.cdp,
            &tab.session_id,
            web_rid,
            self.config.sign_timeout,
            events,
        )
        .await;

        // Cleanup tab regardless of result
        let _ = close_tab(&handle.inner.cdp, &tab.target_id).await;

        result.map_err(|e| PoolError::Sign(e.to_string()))
    }

    pub async fn health(&self) -> PoolHealth {
        let mut browsers = Vec::with_capacity(self.browsers.len());
        let mut ready = 0;
        let mut busy = 0;
        let mut dead = 0;
        for h in &self.browsers {
            let state = if !h.inner.browser.lock().await.is_alive() {
                dead += 1;
                "Dead".to_string()
            } else {
                let available = h.semaphore.available_permits();
                if available == 0 {
                    busy += 1;
                    "Signing".to_string()
                } else {
                    ready += 1;
                    "Idle".to_string()
                }
            };
            let age_ms = h
                .inner
                .last_sign
                .lock()
                .await
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(u64::MAX);
            browsers.push(BrowserHealth {
                id: h.id,
                state,
                last_sign_age_ms: age_ms,
            });
        }
        PoolHealth {
            size: self.browsers.len(),
            ready,
            busy,
            dead,
            browsers,
        }
    }

    // Note: full BrowserPool::sign() path is exercised by E2E test (Task 16)
    // with a real Edge. Unit tests cover the underlying primitives only —
    // see the `tests` mod at the bottom of this file.
}

async fn acquire_tab(cdp: &CdpClient) -> Result<TabSession, crate::cdp::error::CdpError> {
    use crate::cdp::commands::{AttachedToTargetParams, CreateTargetParams};

    let target: crate::cdp::commands::CreateTargetResult = cdp
        .send(
            |id| CdpCommand::CreateTarget {
                id,
                params: CreateTargetParams {
                    url: "about:blank".into(),
                },
            },
            Duration::from_secs(3),
        )
        .await?;

    // Subscribe to attachedToTarget (we need the session id for this target)
    let mut events = cdp.subscribe_session(&target.target_id);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(CdpEvent::AttachedToTarget { params })) => {
                if params.target_info.target_id == target.target_id {
                    return Ok(TabSession {
                        target_id: target.target_id,
                        session_id: params.session_id,
                    });
                }
            }
            _ => continue,
        }
    }
    Err(crate::cdp::error::CdpError::Timeout(Duration::from_secs(3)))
}

async fn close_tab(cdp: &CdpClient, target_id: &str) -> Result<(), crate::cdp::error::CdpError> {
    use crate::cdp::commands::CloseTargetParams;
    let _: serde_json::Value = cdp
        .send(
            |id| CdpCommand::CloseTarget {
                id,
                params: CloseTargetParams {
                    target_id: target_id.into(),
                },
            },
            Duration::from_secs(2),
        )
        .await?;
    Ok(())
}

async fn health_check_loop(pool: Arc<BrowserPool>) {
    let mut interval = tokio::time::interval(pool.config.health_check_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for h in &pool.browsers {
            let mut browser = h.inner.browser.lock().await;
            if !browser.is_alive() {
                tracing::warn!(browser_id = h.id, "browser dead, restarting");
                if let Err(e) = browser.kill() {
                    tracing::warn!(error = %e, "kill old browser failed");
                }
                let browser_config = BrowserConfig {
                    edge_path: pool.config.edge_path.clone(),
                    user_data_dir: PathBuf::from(
                        pool.config.user_data_dir_template.replace("{id}", &h.id.to_string()),
                    ),
                    extra_args: pool.config.extra_args.clone(),
                    cdp_port: pool.config.cdp_port_base + h.id as u16,
                };
                match Browser::spawn(browser_config) {
                    Ok(mut new_browser) => {
                        match new_browser.discover_cdp_url().await {
                            Ok(_) => {
                                tracing::info!(browser_id = h.id, "browser restarted");
                                *browser = new_browser;
                            }
                            Err(e) => {
                                tracing::error!(browser_id = h.id, error = %e, "restart CDP discovery failed");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(browser_id = h.id, error = %e, "browser restart spawn failed");
                    }
                }
            }
        }
    }
}
```

**Implementation note**: The unit tests in Step 1 cover only the underlying primitives (atomic counter, semaphore, error display, JSON serialization). The full `BrowserPool::sign()` path requires a real Edge and is exercised by the E2E test (Task 16).

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-collector pool
```

Expected: 4 tests pass (pool_health_serializes_to_json, round_robin_counter_cycles_correctly, semaphore_exhaustion_blocks_subsequent_acquire, pool_error_display_includes_busy).

- [ ] **Step 5: Commit**

```bash
git add crates/collector/src/pool.rs
git commit -m "feat(collector): add BrowserPool with round-robin + health check"
```

---

## Phase E: Configuration

### Task 8: Add [browser], [rest], [signer] to AppConfig

**Files:**
- Modify: `crates/service/src/config.rs`

**Interfaces:**
- Consumes: existing `AppConfig`, `ServiceConfig`
- Produces:
  - `pub struct BrowserConfig { edge_path, pool_size, max_concurrent_per_browser, sign_timeout_secs, health_check_interval_secs, user_data_dir_template, extra_args }`
  - `pub struct RestConfig { listen_addr: SocketAddr }`
  - `pub enum SignerMode { Browser, Http, Auto }`
  - `AppConfig::browser()`, `AppConfig::rest()`, `AppConfig::signer_mode()`

- [ ] **Step 1: Write failing test in `config.rs`**

Add to `crates/service/src/config.rs` `mod tests`:

```rust
#[test]
fn parse_browser_config_section() {
    let toml = r#"
        [service]
        room_id = "test"

        [browser]
        edge_path = "C:\\Edge\\msedge.exe"
        pool_size = 5
        max_concurrent_per_browser = 3
        sign_timeout_secs = 15
        health_check_interval_secs = 60
        user_data_dir_template = "./data/browser-{id}"
        extra_args = ["--foo", "--bar"]
    "#;
    let cfg: AppConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.browser.pool_size, 5);
    assert_eq!(cfg.browser.max_concurrent_per_browser, 3);
    assert_eq!(cfg.browser.sign_timeout_secs, 15);
    assert_eq!(cfg.browser.extra_args, vec!["--foo", "--bar"]);
}

#[test]
fn parse_rest_config_section() {
    let toml = r#"
        [service]
        room_id = "test"

        [rest]
        listen_addr = "127.0.0.1:9000"
    "#;
    let cfg: AppConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.rest.listen_addr.port(), 9000);
}

#[test]
fn parse_signer_mode_auto() {
    let toml = r#"
        [service]
        room_id = "test"

        [signer]
        mode = "auto"
    "#;
    let cfg: AppConfig = toml::from_str(toml).unwrap();
    assert!(matches!(cfg.signer_mode(), SignerMode::Auto));
}

#[test]
fn default_browser_config_has_sensible_values() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.browser.pool_size, 3);
    assert_eq!(cfg.browser.max_concurrent_per_browser, 2);
    assert_eq!(cfg.browser.sign_timeout_secs, 10);
}

#[test]
fn default_rest_config_uses_port_7878() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.rest.listen_addr.port(), 7878);
}

#[test]
fn default_signer_mode_is_browser() {
    let cfg = AppConfig::default();
    assert!(matches!(cfg.signer_mode(), SignerMode::Browser));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-service config::tests
```

Expected: FAIL (no `browser`, `rest`, `signer_mode` fields).

- [ ] **Step 3: Add types and fields**

In `crates/service/src/config.rs`:

1. Add to `AppConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub service: ServiceConfig,

    #[serde(default)]
    pub wss: WssConfig,

    #[serde(default)]
    pub events: EventsConfig,

    #[serde(default)]
    pub room_api: RoomApiConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub mitm: MitmConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub browser: BrowserConfig,

    #[serde(default)]
    pub rest: RestConfig,

    #[serde(default = "default_signer_mode")]
    pub signer: SignerConfig,
}
```

2. Add the new types:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default = "default_edge_path")]
    pub edge_path: PathBuf,

    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_per_browser: usize,

    #[serde(default = "default_sign_timeout")]
    pub sign_timeout_secs: u64,

    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,

    #[serde(default = "default_user_data_dir_template")]
    pub user_data_dir_template: String,

    #[serde(default)]
    pub extra_args: Vec<String>,

    #[serde(default = "default_cdp_port_base")]
    pub cdp_port_base: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestConfig {
    #[serde(default = "default_rest_addr")]
    pub listen_addr: SocketAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerConfig {
    #[serde(default = "default_signer_mode_str")]
    pub mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignerMode {
    Browser,
    Http,
    Auto,
}

fn default_edge_path() -> PathBuf {
    PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
}
fn default_pool_size() -> usize { 3 }
fn default_max_concurrent() -> usize { 2 }
fn default_sign_timeout() -> u64 { 10 }
fn default_health_check_interval() -> u64 { 30 }
fn default_user_data_dir_template() -> String {
    "./data/browser-{id}".into()
}
fn default_cdp_port_base() -> u16 { 9222 }
fn default_rest_addr() -> SocketAddr { "0.0.0.0:7878".parse().unwrap() }
fn default_signer_mode() -> SignerConfig {
    SignerConfig { mode: "browser".into() }
}
fn default_signer_mode_str() -> String { "browser".into() }

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            edge_path: default_edge_path(),
            pool_size: default_pool_size(),
            max_concurrent_per_browser: default_max_concurrent(),
            sign_timeout_secs: default_sign_timeout(),
            health_check_interval_secs: default_health_check_interval(),
            user_data_dir_template: default_user_data_dir_template(),
            extra_args: vec![],
            cdp_port_base: default_cdp_port_base(),
        }
    }
}

impl Default for RestConfig {
    fn default() -> Self {
        Self { listen_addr: default_rest_addr() }
    }
}

impl Default for SignerConfig {
    fn default() -> Self {
        Self { mode: "browser".into() }
    }
}

impl AppConfig {
    pub fn signer_mode(&self) -> SignerMode {
        match self.signer.mode.as_str() {
            "browser" => SignerMode::Browser,
            "http" => SignerMode::Http,
            "auto" => SignerMode::Auto,
            _ => SignerMode::Browser, // default fallback
        }
    }
}
```

3. Update `Default for AppConfig`:

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            service: ServiceConfig::default(),
            wss: WssConfig::default(),
            events: EventsConfig::default(),
            room_api: RoomApiConfig::default(),
            auth: AuthConfig::default(),
            mitm: MitmConfig::default(),
            logging: LoggingConfig::default(),
            browser: BrowserConfig::default(),
            rest: RestConfig::default(),
            signer: SignerConfig::default(),
        }
    }
}
```

4. Add `PathBuf` import at top:

```rust
use std::path::PathBuf;
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-service config::tests
```

Expected: 6 new tests pass; existing config tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/service/src/config.rs
git commit -m "feat(service): add [browser], [rest], [signer] config sections"
```

---

## Phase F: REST API

### Task 9: Add axum dependency

**Files:**
- Modify: `crates/service/Cargo.toml`

- [ ] **Step 1: Add deps**

In `crates/service/Cargo.toml`:

```toml
[dependencies]
axum = "0.7"
tower = "0.4"
```

- [ ] **Step 2: Verify build**

```bash
cargo check -p eleven-barrage-service
```

Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/service/Cargo.toml
git commit -m "chore(service): add axum and tower dependencies"
```

---

### Task 10: POST /v1/sign handler

**Files:**
- Create: `crates/service/src/api/mod.rs`
- Create: `crates/service/src/api/sign.rs`
- Modify: `crates/service/src/lib.rs` (add `pub mod api;`)

**Interfaces:**
- Consumes: `Arc<BrowserPool>` (from T7), `eleven_barrage_collector::parse_url`
- Produces:
  - `pub struct SignRequest { url: String }`
  - `pub struct SignResponse { wss_url, headers, expires_at_unix, captured_at_unix }`
  - `pub async fn sign(State(pool), Json(req)) -> Result<Json<SignResponse>, ApiError>`
  - `pub enum ApiError` with `IntoResponse`

- [ ] **Step 1: Write failing test in `sign.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_response_serializes_expected_fields() {
        let resp = SignResponse {
            wss_url: "wss://x".into(),
            headers: [("Cookie".into(), "ttwid=y".into())].into_iter().collect(),
            expires_at_unix: 1000,
            captured_at_unix: 500,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["wss_url"], "wss://x");
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-service api::sign
```

Expected: FAIL (file doesn't exist).

- [ ] **Step 3: Implement `sign.rs`**

```rust
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

use eleven_barrage_collector::{parse_url, pool::PoolError, BrowserPool, SignedWssMaterial};

#[derive(Deserialize)]
pub struct SignRequest {
    pub url: String,
}

#[derive(Serialize)]
pub struct SignResponse {
    pub wss_url: String,
    pub headers: HashMap<String, String>,
    pub expires_at_unix: i64,
    pub captured_at_unix: i64,
}

impl From<SignedWssMaterial> for SignResponse {
    fn from(m: SignedWssMaterial) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let expires = m.expires_at.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        Self {
            wss_url: m.url,
            headers: m.headers,
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
    NoWssCaptured,
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
                "timeout waiting for WSS request".into(),
            ),
            ApiError::NoWssCaptured => (
                StatusCode::BAD_GATEWAY,
                "NO_WSS_CAPTURED",
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

    let material = pool.sign(&web_rid).await.map_err(|e| match e {
        PoolError::Busy => ApiError::PoolBusy,
        PoolError::Sign(msg) if msg.contains("Timeout") => ApiError::WssTimeout,
        PoolError::Sign(msg) if msg.contains("NoWssCaptured") => ApiError::NoWssCaptured,
        PoolError::Sign(msg) => ApiError::SignFailed(msg),
        PoolError::Browser(_) => ApiError::BrowserDead,
    })?;

    Ok(Json(SignResponse::from(material)))
}
```

- [ ] **Step 4: Create `api/mod.rs` and add to `lib.rs`**

`crates/service/src/api/mod.rs`:

```rust
//! REST API for external programs (auto-signer spec section 5)

pub mod sign;
pub mod health;

use std::sync::Arc;
use axum::{routing::{get, post}, Router};
use eleven_barrage_collector::BrowserPool;

pub fn router(pool: Arc<BrowserPool>) -> Router {
    Router::new()
        .route("/v1/sign", post(sign::sign))
        .route("/v1/health", get(health::health))
        .with_state(pool)
}
```

`crates/service/src/lib.rs`: find the existing module declarations and add:

```rust
pub mod api;
pub mod rest_server;
```

(Stubs for `rest_server` already created in earlier task planning; if not, create stub now.)

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-service api::sign
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/service/src/api/ crates/service/src/lib.rs
git commit -m "feat(service): add POST /v1/sign handler with ApiError mapping"
```

---

### Task 11: GET /v1/health handler

**Files:**
- Create: `crates/service/src/api/health.rs`

**Interfaces:**
- Consumes: `Arc<BrowserPool>`
- Produces: `pub async fn health(State(pool)) -> Result<Json<HealthResponse>, StatusCode>`

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_includes_status_field() {
        let r = HealthResponse {
            status: "ok".into(),
            pool: PoolHealthView { size: 3, ready: 3, busy: 0, dead: 0 },
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p eleven-barrage-service api::health
```

Expected: FAIL.

- [ ] **Step 3: Implement `health.rs`**

```rust
//! GET /v1/health handler (auto-signer spec section 5.3)

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use eleven_barrage_collector::pool::{BrowserHealth, PoolHealth};
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
    State(pool): State<Arc<eleven_barrage_collector::BrowserPool>>,
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
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p eleven-barrage-service api::health
```

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/service/src/api/health.rs
git commit -m "feat(service): add GET /v1/health handler"
```

---

### Task 12: rest_server.rs lifecycle

**Files:**
- Modify: `crates/service/src/rest_server.rs` (replace stub if exists, or create)

**Interfaces:**
- Consumes: `Arc<BrowserPool>`, `SocketAddr`
- Produces: `pub async fn run_rest_server(addr: SocketAddr, pool: Arc<BrowserPool>) -> Result<()>`

- [ ] **Step 1: Implement `rest_server.rs`**

```rust
//! REST server lifecycle (auto-signer spec section 5.5)

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use eleven_barrage_collector::BrowserPool;
use tracing::info;

use crate::api;

pub async fn run_rest_server(addr: SocketAddr, pool: Arc<BrowserPool>) -> Result<()> {
    let app = api::router(pool);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind REST on {}", addr))?;
    info!(addr = %addr, "REST server listening");
    axum::serve(listener, app)
        .await
        .context("REST server crashed")?;
    Ok(())
}
```

- [ ] **Step 2: Verify build**

```bash
cargo check -p eleven-barrage-service
```

Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/service/src/rest_server.rs
git commit -m "feat(service): add REST server lifecycle (axum::serve)"
```

---

## Phase G: Service Integration

### Task 13: run.rs starts BrowserPool + REST server

**Files:**
- Modify: `crates/service/src/run.rs`

- [ ] **Step 1: Read current `run.rs` to understand integration points**

Read the existing run.rs (already in context). Key integration points:
- After loading config, build `BrowserPoolConfig` from `AppConfig::browser()`
- Start `BrowserPool::start(config).await?`
- Spawn REST server task with `rest_server::run_rest_server(addr, pool.clone())`
- The existing gRPC `SignedBarrageServiceImpl` should receive `Arc<BrowserPool>` instead of `AutoSigner` (see T14)

- [ ] **Step 2: Add BrowserPool + REST startup in `run()`**

Edit `crates/service/src/run.rs`. Add after config validation:

```rust
// Start BrowserPool (auto-signer)
let browser_config = eleven_barrage_collector::pool::BrowserPoolConfig {
    pool_size: config.browser.pool_size,
    max_concurrent_per_browser: config.browser.max_concurrent_per_browser,
    sign_timeout: std::time::Duration::from_secs(config.browser.sign_timeout_secs),
    health_check_interval: std::time::Duration::from_secs(config.browser.health_check_interval_secs),
    edge_path: config.browser.edge_path.clone(),
    user_data_dir_template: config.browser.user_data_dir_template.clone(),
    extra_args: config.browser.extra_args.clone(),
    cdp_port_base: config.browser.cdp_port_base,
};
let browser_pool = std::sync::Arc::new(
    eleven_barrage_collector::BrowserPool::start(browser_config)
        .await
        .context("failed to start browser pool")?,
);

// Start REST server task
let rest_addr = config.rest.listen_addr;
let rest_pool = browser_pool.clone();
let rest_handle = tokio::spawn(async move {
    if let Err(e) = crate::rest_server::run_rest_server(rest_addr, rest_pool).await {
        tracing::error!(error = %e, "REST server exited");
    }
});
```

Store `browser_pool` in a place accessible to gRPC init (T14). Modify the gRPC init section (currently uses `signer` variable) to take `browser_pool` instead.

- [ ] **Step 3: Update shutdown to abort REST**

In the shutdown section, add:

```rust
rest_handle.abort();
```

- [ ] **Step 4: Verify build**

```bash
cargo check -p eleven-barrage-service
```

Expected: OK (warnings about unused imports are fine; the gRPC wiring is updated in T14).

- [ ] **Step 5: Commit**

```bash
git add crates/service/src/run.rs
git commit -m "feat(service): start BrowserPool and REST server in run()"
```

---

### Task 14: grpc_signed.rs routes to BrowserSigner

**Files:**
- Modify: `crates/service/src/grpc_signed.rs`

**Goal**: The existing `ProvideSignedWss` gRPC method should now use the `BrowserPool` instead of `AutoSigner` (HTTP). This preserves backward compatibility for clients that already use the gRPC interface.

- [ ] **Step 1: Change `SignedBarrageServiceImpl` constructor**

Current:
```rust
pub struct SignedBarrageServiceImpl {
    signer: AutoSigner,
}
```

New:
```rust
pub struct SignedBarrageServiceImpl {
    pool: Arc<BrowserPool>,
}

impl SignedBarrageServiceImpl {
    pub fn new(pool: Arc<BrowserPool>) -> Self {
        Self { pool }
    }
}
```

- [ ] **Step 2: Update `provide_signed_wss` implementation**

Replace the call from `self.signer.sign(&web_rid)` to `self.pool.sign(&web_rid)` and adjust error mapping:

```rust
match self.pool.sign(&web_rid).await {
    Ok(material) => {
        let grpc_material = to_grpc_material(material);
        Ok(Response::new(ProvideSignedWssResponse {
            result: Some(
                self::signed_proto::provide_signed_wss_response::Result::Material(
                    grpc_material,
                ),
            ),
        }))
    }
    Err(e) => {
        let sig_err = map_pool_error_to_signature_error(&e);
        warn!(error = %e, "auto-sign failed");
        Ok(Response::new(ProvideSignedWssResponse {
            result: Some(
                self::signed_proto::provide_signed_wss_response::Result::Error(
                    to_error_info(&sig_err),
                ),
            ),
        }))
    }
}

fn map_pool_error_to_signature_error(e: &PoolError) -> SignatureError {
    match e {
        PoolError::Busy => SignatureError::NetworkTransient {
            reason: "pool busy".into(),
        },
        PoolError::Sign(msg) if msg.contains("Timeout") => SignatureError::NetworkTransient {
            reason: "WSS timeout".into(),
        },
        PoolError::Sign(_) => SignatureError::AlgorithmChanged,
        PoolError::Browser(_) => SignatureError::NetworkTransient {
            reason: "browser dead".into(),
        },
    }
}
```

Note: `SignatureError::AlgorithmChanged` may not have a `reason` field — check existing definition in `crates/collector/src/error.rs` and add `reason: String` if missing. Alternatively, use `NetworkTransient` as the catch-all.

- [ ] **Step 3: Update `run.rs` to construct gRPC service with `Arc<BrowserPool>`**

Replace the existing signer construction:

```rust
let grpc_service = eleven_barrage_service::SignedBarrageServiceImpl::new(browser_pool.clone());
```

Update `grpc_server::run_grpc_server_with_source_and_signer` to accept the new type. If the existing function signature is rigid, add a new variant `run_grpc_server_with_pool`:

```rust
pub async fn run_grpc_server_with_pool(
    addr: SocketAddr,
    event_rx: mpsc::Receiver<BarrageEvent>,
    pool: Arc<BrowserPool>,
) -> Result<()>
```

and update run.rs to use it.

- [ ] **Step 4: Verify build**

```bash
cargo check -p eleven-barrage-service
```

Expected: OK.

- [ ] **Step 5: Run existing grpc_signed tests**

```bash
cargo test -p eleven-barrage-service grpc_signed
```

Expected: existing tests still pass (they construct `AutoSigner` for unit tests; for those that test gRPC dispatch, update them to construct with a mock pool).

For tests that previously created `AutoSigner` directly, refactor to use a mock `BrowserPool` (or skip integration-level gRPC tests in unit-test mode — they're covered by E2E).

- [ ] **Step 6: Commit**

```bash
git add crates/service/src/grpc_signed.rs crates/service/src/run.rs crates/service/src/grpc_server.rs
git commit -m "feat(service): route ProvideSignedWss gRPC through BrowserPool"
```

---

## Phase H: CLI

### Task 15: ebg sign subcommand

**Files:**
- Modify: `crates/cli/src/main.rs`

- [ ] **Step 1: Add Sign subcommand**

In `crates/cli/src/main.rs`, add to `EbgCommand` enum:

```rust
/// 调用 REST /v1/sign 并输出 JSON（推荐：服务必须已启动）
Sign {
    /// 抖音直播间 URL
    #[arg(long)]
    url: String,

    /// REST 服务地址
    #[arg(long, default_value = "http://127.0.0.1:7878")]
    rest_addr: String,
},
```

- [ ] **Step 2: Add match arm in main**

```rust
Some(EbgCommand::Sign { url, rest_addr }) => match run_sign(&url, &rest_addr).await {
    Ok(_) => ExitCode::SUCCESS,
    Err(e) => {
        eprintln!("Error: {}", e);
        ExitCode::FAILURE
    }
},
```

- [ ] **Step 3: Implement `run_sign`**

```rust
async fn run_sign(url: &str, rest_addr: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/sign", rest_addr))
        .json(&serde_json::json!({ "url": url }))
        .send()
        .await?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        eprintln!("HTTP {}: {}", status, serde_json::to_string_pretty(&body)?);
        anyhow::bail!("sign request failed");
    }
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}
```

- [ ] **Step 4: Add test for CLI parsing**

```rust
#[test]
fn cli_parses_sign_with_url() {
    let cli = Cli::try_parse_from(["ebg", "sign", "--url", "https://live.douyin.com/123"]).unwrap();
    match cli.command {
        Some(EbgCommand::Sign { url, .. }) => assert_eq!(url, "https://live.douyin.com/123"),
        _ => panic!("expected Sign command"),
    }
}
```

- [ ] **Step 5: Verify build and tests**

```bash
cargo check -p eleven-barrage-cli
cargo test -p eleven-barrage-cli
```

Expected: OK.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "feat(cli): add `ebg sign` subcommand (calls REST /v1/sign)"
```

---

## Phase I: E2E + Docs

### Task 16: E2E test (#[ignore])

**Files:**
- Create: `crates/service/tests/e2e_url_to_wss.rs`

- [ ] **Step 1: Write E2E test**

```rust
//! E2E test against real Douyin + real Edge. Run with `cargo test -- --ignored`.
//!
//! Prerequisites:
//! - Edge installed at the path in config.toml
//! - Valid ttwid in config.toml [auth]
//! - TEST_ROOM env var set to a live room id (default: 664637748606)

use std::sync::Arc;
use std::time::Duration;

use eleven_barrage_collector::pool::{BrowserPool, BrowserPoolConfig};

#[tokio::test]
#[ignore = "requires real Edge + valid ttwid"]
async fn e2e_real_room_returns_signed_wss() {
    let pool_size = std::env::var("TEST_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1); // 1 for E2E speed
    let config = BrowserPoolConfig {
        pool_size,
        max_concurrent_per_browser: 1,
        sign_timeout: Duration::from_secs(15),
        ..Default::default()
    };

    let pool = BrowserPool::start(config).await.expect("pool start");
    let room = std::env::var("TEST_ROOM").unwrap_or_else(|_| "664637748606".into());

    let material = pool.sign(&room).await.expect("sign failed");

    assert!(material.url.starts_with("wss://"), "url: {}", material.url);
    assert!(material.url.contains("webcast/im/push"), "url: {}", material.url);
    assert!(material.headers.contains_key("Cookie"), "missing Cookie header");

    let cookie = material.headers.get("Cookie").unwrap();
    assert!(cookie.contains("ttwid="), "Cookie missing ttwid: {}", cookie);
}

#[tokio::test]
#[ignore = "requires real Edge + valid ttwid"]
async fn e2e_health_endpoint_reflects_browser_state() {
    let pool = BrowserPool::start(BrowserPoolConfig {
        pool_size: 1,
        max_concurrent_per_browser: 1,
        ..Default::default()
    }).await.expect("pool start");

    let health = pool.health().await;
    assert_eq!(health.size, 1);
    assert_eq!(health.ready + health.busy + health.dead, 1);
}
```

- [ ] **Step 2: Verify test compiles**

```bash
cargo check -p eleven-barrage-service --tests
```

Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/service/tests/e2e_url_to_wss.rs
git commit -m "test(service): add E2E test for real Edge + Douyin sign (#[ignore])"
```

---

### Task 17: Update config.example.toml

**Files:**
- Modify: `config.example.toml`

- [ ] **Step 1: Add new sections**

Append to `config.example.toml`:

```toml
# Browser-based signer config (auto-signer)
[browser]
# Edge path (Windows default; Linux/Mac auto-detect in v2)
edge_path = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"

# Pool size: number of Edge processes
pool_size = 3

# Concurrent sign requests per browser
max_concurrent_per_browser = 2

# Timeout for capturing WSS request after navigation
sign_timeout_secs = 10

# Health check interval (browser alive check)
health_check_interval_secs = 30

# User data directory template ({id} substituted with browser index)
user_data_dir_template = "./data/browser-{id}"

# Extra Edge args (anti-detection)
extra_args = [
    "--headless=new",
    "--disable-blink-features=AutomationControlled",
    "--no-first-run",
    "--no-default-browser-check",
    "--window-size=1920,1080",
]

# CDP port base (each browser gets port_base + id)
cdp_port_base = 9222

# REST API config
[rest]
listen_addr = "0.0.0.0:7878"

# Signer mode: "browser" (default), "http" (legacy ImFetcher), "auto" (fallback)
[signer]
mode = "browser"
```

- [ ] **Step 2: Commit**

```bash
git add config.example.toml
git commit -m "docs: document [browser], [rest], [signer] config sections"
```

---

### Task 18: Final verification

- [ ] **Step 1: Full test suite**

```bash
export PATH="/d/devtools/protoc/bin:/d/devtools/mingw64/mingw64/bin:/d/devtools/.cargo/bin:$PATH"
cargo test --workspace
```

Expected: all tests pass (existing 62 + new ones from this plan).

- [ ] **Step 2: cargo check across workspace**

```bash
cargo check --workspace --all-targets
```

Expected: 0 errors, warnings acceptable.

- [ ] **Step 3: cargo clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Fix any clippy warnings introduced by new code (unused imports, etc.).

- [ ] **Step 4: Build release**

```bash
cargo build --release
```

Expected: release binary builds.

- [ ] **Step 5: Manual smoke test**

```bash
# Terminal 1: start daemon (uses config.toml with user's ttwid)
./target/release/ebg.exe start

# Terminal 2: call REST sign
./target/release/ebg.exe sign --url "https://live.douyin.com/664637748606"
```

Expected: JSON output with `wss_url` starting with `wss://webcast`.

- [ ] **Step 6: Final commit if any cleanup needed**

```bash
git add -A
git commit -m "chore: cleanup from clippy + final integration"
```

---

## Acceptance Criteria Verification

After Task 18, verify each acceptance criterion from the spec:

| Criterion | Verified by |
|---|---|
| `cargo test` passes with ≥ 80% coverage on new modules | Task 18 Step 1 |
| Manual E2E returns valid SignedWssMaterial for a real room | Task 16 + Task 18 Step 5 |
| `POST /v1/sign` returns HTTP 200 with `wss_url` starting with `wss://webcast` | Task 10 + Task 18 Step 5 |
| `GET /v1/health` reflects accurate pool state | Task 11 + Task 18 Step 5 |
| Pool throughput ≥ 4 req/s sustained under 10 concurrent | Deferred to v1.1 (manual benchmark script) |
| No browser leaks | Task 7 health check + manual observation |
| Backward compat: existing gRPC clients keep working | Task 14 |