# Browser-Based Auto Signer — Design Spec

**Date**: 2026-06-28
**Status**: Draft (awaiting user review)
**Target feature**: Replace HTTP-based `ImFetcher` with Edge headless browser auto-signer

## Context

The current `auto-sign-fetcher` (implemented in commit `c61f294`) uses HTTP calls to
`webcast/im/fetch/` to obtain signed WSS materials. Empirical testing on 2026-06-28
(user-provided ttwid cookie `1%7CNEDLo3vQLdD-FWpb...`) revealed that this endpoint
now returns empty bodies (`Content-Length: 0`) when called from a pure HTTP client.
Douyin's anti-bot layer has added dynamic tokens (`X-Ms-Token`, `X-Janus-Info`) that
require JavaScript execution to compute — meaning the HTTP-only approach is no longer
viable.

This design replaces the HTTP signer with a headless-browser-based signer that drives
a real Edge instance via the Chrome DevTools Protocol (CDP), extracts the signed
WSS URL by observing real WebSocket handshake requests, and returns it via a clean
REST API.

## Goals

1. **Standardized service**: external programs call one HTTP endpoint to obtain signed WSS materials
2. **Anti-bot resilient**: ride on real browser fingerprint, no protocol reverse-engineering needed
3. **High QPS**: support concurrent sign requests via a browser pool
4. **Graceful degradation**: clear error codes, health endpoint, Prometheus metrics
5. **Testable**: mock CDP client allows full unit/integration coverage without real Edge

## Non-Goals

- Auto-passing captcha / human-verification challenges (return error, let user handle)
- Cross-platform browser support in v1 (Windows Edge only; portable Chrome is post-v1)
- Session-cookie (`sessionid`) login automation (out of scope; manual injection only)
- WebSocket-downstream event streaming from this service (existing WS server remains unchanged)

## Architectural Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Browser source | System Edge | Pre-installed on Win10/11; no extra download |
| CDP client impl | Native (tokio-tungstenite + serde_json) | 8 commands subset; full control; decoupled from Edge version |
| API contract | `POST /v1/sign` returns `SignedWssMaterial` | Caller connects to WSS themselves; service stays stateless and horizontally scalable |
| Pool model | Lazy start + 3 browsers, 2 concurrent each | 6 concurrent sign = ~12 req/s theoretical, ~6-8 req/s practical |
| Tab lifecycle | Open-per-sign, close-after-extract | Avoids page state pollution between rooms; ~700ms overhead per sign |
| Failure recovery | Single-sign failure → reset tab; browser death → health-check restart | Isolated blast radius |
| Backward compat | gRPC `ProvideSignedWss` still works, internally calls `BrowserSigner` | Old clients keep working |

---

## 1. Architecture Overview

### 1.1 Crate Changes

```
crates/
├── proto/           (unchanged)
├── core/            (unchanged; reuses BarrageEvent type)
├── collector/       (modified: replace im_fetch.rs with browser-based signer)
│   ├── url_parser.rs    (existing, kept)
│   ├── error.rs        (existing + new BrowserError variants)
│   ├── cdp/            (NEW submodule)
│   │   ├── client.rs   — CDP WebSocket client (native impl)
│   │   ├── commands.rs — CdpCommand / CdpEvent serde types
│   │   ├── frame.rs    — WebSocket message ↔ CdpEvent
│   │   └── mock.rs     — MockCdpClient for tests
│   ├── browser.rs      (NEW: Browser process management)
│   ├── pool.rs         (NEW: BrowserPool with round-robin scheduling)
│   ├── signer.rs       (NEW: BrowserSigner — URL → SignedWssMaterial)
│   └── im_fetch.rs     (deprecated, kept for [signer] mode = "http" fallback)
├── service/         (modified: add REST layer, keep gRPC for compat)
│   ├── api/             (NEW)
│   │   ├── mod.rs       — axum router factory
│   │   ├── sign.rs      — POST /v1/sign handler
│   │   └── health.rs    — GET /v1/health handler
│   ├── rest_server.rs   (NEW: axum startup, lifecycle)
│   ├── grpc_signed.rs   (modified: route to BrowserSigner)
│   ├── run.rs           (modified: start BrowserPool + REST + gRPC)
│   └── config.rs        (modified: add [browser] section)
└── cli/             (modified: add `ebg sign` subcommand)

tests/
├── cdp_commands.rs       — CdpCommand/CdpEvent serde tests
├── browser_pool.rs       — pool scheduling with MockBrowser
└── e2e_url_to_wss.rs     — real Edge + real Douyin (#[ignore])
```

### 1.2 Data Flow: One `/v1/sign` Call

```
External client
   │ POST /v1/sign {"url": "https://live.douyin.com/664637748606"}
   ▼
axum router (rest_server.rs)
   │ 1. parse URL → web_rid ("664637748606")
   ▼
BrowserPool::sign(web_rid)
   │ 2. round-robin select browser handle
   │ 3. acquire semaphore permit (non-blocking)
   ▼
Browser::sign(web_rid)
   │ 4. acquire tab from idle pool (or createTarget)
   │ 5. CDP attach to target session
   ▼
BrowserSigner::extract_wss
   │ 6. Network.enable (subscribe to requestWillBeSent)
   │ 7. Page.navigate to live.douyin.com/{web_rid}
   │ 8. wait for wss://webcast/im/push request event (≤ 10s timeout)
   │ 9. extract request.url + request.headers
   ▼
Target.closeTarget (cleanup tab)
   ▼
Return SignedWssMaterial { url, headers, expires_at }
   │
External client uses material to connect to WSS directly
```

---

## 2. BrowserPool Design

### 2.1 Pool Structure

```rust
pub struct BrowserPool {
    browsers: Vec<BrowserHandle>,
    next_index: AtomicUsize,
    config: BrowserPoolConfig,
}

pub struct BrowserHandle {
    id: usize,
    inner: Arc<Browser>,
    semaphore: Arc<Semaphore>,  // permits = max_concurrent_per_browser
}

#[derive(Clone)]
pub struct BrowserPoolConfig {
    pub pool_size: usize,                  // default 3
    pub max_concurrent_per_browser: usize, // default 2
    pub sign_timeout: Duration,            // default 10s
    pub edge_path: PathBuf,
    pub extra_args: Vec<String>,
    pub user_data_dir_template: String,    // e.g., "./data/browser-{id}"
    pub health_check_interval: Duration,   // default 30s
}
```

### 2.2 Scheduling Algorithm: Round-Robin + Semaphore

```rust
async fn sign(&self, web_rid: &str) -> Result<SignedWssMaterial, BrowserError> {
    let attempts = self.browsers.len();
    for i in 0..attempts {
        let idx = self.next_index.fetch_add(1, Ordering::Relaxed) % self.browsers.len();
        let handle = &self.browsers[idx];

        if let Ok(permit) = handle.semaphore.clone().try_acquire_owned() {
            let result = handle.inner.sign(web_rid).await;
            drop(permit);
            return result;
        }
        // this browser busy; try next
    }
    Err(BrowserError::PoolBusy)
}
```

- Non-blocking: returns `PoolBusy` immediately if all browsers saturated
- `try_acquire_owned` so permit travels with the future

### 2.3 Browser State Machine

```
            start()
              ↓
         [Starting]        — spawn Edge, wait for CDP port
              ↓ (success)
    ┌───────[Idle]←─────────────────┐
    │         │                      │
    │         │ sign()               │ sign() done
    │         ▼                      │
    │      [Signing]─────────────────┘
    │         │
    │         ├─ success → [Idle]
    │         ├─ tab-level error → [Recovering] ─reset tab─→ [Idle]
    │         └─ CDP disconnect → [Dead]
    │                                  │
    │         health_check detects     │ (every 30s)
    │         ↓                        │
    │     [Starting]                   │
    └──────────────────────────────────┘
```

### 2.4 Health Check Loop

```rust
async fn health_check_loop(pool: Arc<BrowserPool>) {
    let mut interval = tokio::time::interval(pool.config.health_check_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for handle in &pool.browsers {
            match handle.inner.state().await {
                BrowserState::Dead => {
                    warn!(browser_id = handle.id, "browser dead, restarting");
                    if let Err(e) = handle.inner.restart().await {
                        error!(browser_id = handle.id, error = %e, "restart failed");
                    }
                }
                _ => {}
            }
        }
    }
}
```

### 2.5 Pool Startup

```
1. Spawn N Edge processes in parallel
   for i in 0..pool_size:
     let browser = Browser::spawn(config, user_data_dir(i)).await?;
     browsers.push(browser);

2. Wait for all Edge instances to expose CDP port (poll http://127.0.0.1:{9222+i}/json/version)

3. Connect CdpClient to each browser

4. For each browser: create one warmup tab, navigate to live.douyin.com,
   let initial cookies/X-Ms-Token settle, then close tab

5. Spawn health_check_loop task

6. Return BrowserPool
```

---

## 3. CDP Client Core

### 3.1 Command Subset (only what we need)

| Domain | Command/Event | Purpose |
|---|---|---|
| `Target` | `setDiscoverTargets`, `createTarget`, `closeTarget` | tab lifecycle |
| `Target` | `attachedToTarget`, `detachedFromTarget` (events) | session management |
| `Page` | `enable`, `navigate` | page loading |
| `Page` | `loadEventFired` (event) | wait for page ready |
| `Network` | `enable`, `disable` | network observation |
| `Network` | `requestWillBeSent` (event) | **core: capture WSS request** |
| `Runtime` | `evaluate` | optional: read cookies, detect captcha |
| `Browser` | `getVersion` | health check |

### 3.2 Module Structure

```
crates/collector/src/cdp/
├── mod.rs           — public re-exports + BrowserContext trait
├── client.rs        — CdpClient struct (single CDP WebSocket connection)
├── commands.rs      — CdpCommand enum + CdpResponse + CdpEvent enum
├── frame.rs         — WebSocket Message ↔ parsed JSON helpers
├── error.rs         — CdpError
└── mock.rs          — MockCdpClient for testing
```

### 3.3 CdpClient API

```rust
pub struct CdpClient {
    write: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    read_task: JoinHandle<()>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<CdpResponse>>>>,
    event_tx: mpsc::UnboundedSender<CdpEvent>,
    next_id: AtomicI64,
}

impl CdpClient {
    pub async fn connect(ws_url: &str) -> Result<(Self, mpsc::UnboundedReceiver<CdpEvent>), CdpError>;

    /// Send command, await response (with timeout)
    pub async fn send<R: DeserializeOwned>(
        &self,
        build: impl FnOnce(i64) -> CdpCommand,
        timeout: Duration,
    ) -> Result<R, CdpError>;

    /// Subscribe to events on a specific session
    pub fn subscribe_session(&self, session_id: &str) -> mpsc::UnboundedReceiver<CdpEvent>;
}

/// Abstraction over CDP transport for testing
#[async_trait]
pub trait CdpTransport: Send + Sync {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd_json: serde_json::Value,
        timeout: Duration,
    ) -> Result<R, CdpError>;

    fn subscribe_events(&self) -> mpsc::UnboundedReceiver<CdpEvent>;
}
```

### 3.4 Command/Event Serde

```rust
#[derive(Serialize)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum CdpCommand {
    #[serde(rename = "Target.setDiscoverTargets")]
    SetDiscoverTargets { id: i64, params: SetDiscoverTargetsParams },

    #[serde(rename = "Target.createTarget")]
    CreateTarget { id: i64, params: CreateTargetParams },

    #[serde(rename = "Target.closeTarget")]
    CloseTarget { id: i64, params: CloseTargetParams },

    #[serde(rename = "Page.enable")]
    PageEnable { id: i64, params: PageEnableParams },  // { sessionId? }

    #[serde(rename = "Page.navigate")]
    PageNavigate { id: i64, params: NavigateParams },  // { url, sessionId? }

    #[serde(rename = "Network.enable")]
    NetworkEnable { id: i64, params: NetworkEnableParams },

    #[serde(rename = "Network.disable")]
    NetworkDisable { id: i64, params: NetworkDisableParams },

    #[serde(rename = "Runtime.evaluate")]
    RuntimeEvaluate { id: i64, params: RuntimeEvaluateParams },

    #[serde(rename = "Browser.getVersion")]
    GetVersion { id: i64 },
}

#[derive(Deserialize)]
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
    Unknown,  // forward-compat: unknown events don't break parsing
}
```

### 3.5 Read Loop

```rust
async fn read_loop(
    mut read: SplitStream<...>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<CdpResponse>>>>,
    event_tx: mpsc::UnboundedSender<CdpEvent>,
) {
    while let Some(msg) = read.next().await {
        let bytes = match msg? {
            Message::Binary(b) => b,
            Message::Text(t) => t.into_bytes(),
            _ => continue,
        };

        let parsed: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "failed to parse CDP message");
                continue;
            }
        };

        if let Some(id) = parsed.get("id").and_then(|v| v.as_i64()) {
            // Response: route to waiter
            let resp: CdpResponse = serde_json::from_value(parsed)?;
            let mut p = pending.lock().await;
            if let Some(tx) = p.remove(&id) {
                let _ = tx.send(resp);
            }
        } else if parsed.get("method").is_some() {
            // Event: broadcast to subscribers
            let event: CdpEvent = serde_json::from_value(parsed)?;
            let _ = event_tx.send(event);
        }
    }
}
```

### 3.6 Error Type

```rust
#[derive(thiserror::Error, Debug)]
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
```

### 3.7 Anti-Detection Browser Args

```rust
const DEFAULT_EDGE_ARGS: &[&str] = &[
    "--headless=new",
    "--disable-blink-features=AutomationControlled",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--window-size=1920,1080",
];
```

---

## 4. WSS Extraction Flow

### 4.1 Single Sign Sequence

```rust
pub async fn sign(&self, web_rid: &str) -> Result<SignedWssMaterial, BrowserError> {
    let tab = self.acquire_tab().await?;
    let result = self.extract_wss(&tab, web_rid).await;
    self.release_tab(tab).await;
    result
}

async fn extract_wss(
    &self,
    tab: &TabSession,
    web_rid: &str,
) -> Result<SignedWssMaterial, BrowserError> {
    let mut event_rx = self.cdp.subscribe_session(&tab.session_id);

    // Enable domains on this session
    self.cdp.send(|id| CdpCommand::PageEnable { id, params: PageEnableParams {
        session_id: Some(tab.session_id.clone()),
    }}, Duration::from_secs(2)).await?;

    self.cdp.send(|id| CdpCommand::NetworkEnable { id, params: NetworkEnableParams {
        session_id: Some(tab.session_id.clone()),
    }}, Duration::from_secs(2)).await?;

    // Navigate
    self.cdp.send(|id| CdpCommand::PageNavigate { id, params: NavigateParams {
        url: format!("https://live.douyin.com/{}", web_rid),
        session_id: Some(tab.session_id.clone()),
        referrer: Some("https://live.douyin.com/".into()),
    }}, Duration::from_secs(5)).await?;

    // Wait for WSS request event
    let deadline = Instant::now() + self.config.sign_timeout;
    while Instant::now() < deadline {
        let event = tokio::select! {
            evt = event_rx.recv() => evt,
            _ = tokio::time::sleep_until(deadline.into()) => {
                return Err(BrowserError::WssTimeout(self.config.sign_timeout));
            }
        };

        if let Some(CdpEvent::RequestWillBeSent { params }) = event {
            let url = &params.request.url;
            if url.starts_with("wss://") && url.contains("webcast/im/push") {
                // Capture and return (filter out other WS like analytics)
                let headers: HashMap<String, String> = params.request.headers.into_iter().collect();
                return Ok(SignedWssMaterial {
                    url: url.clone(),
                    headers,
                    expires_at: SystemTime::now() + Duration::from_secs(3600),
                });
            }
        }
    }

    Err(BrowserError::WssTimeout(self.config.sign_timeout))
}
```

### 4.2 Tab Lifecycle

```rust
async fn acquire_tab(&self) -> Result<TabSession, BrowserError> {
    if let Some(tab) = self.idle_tabs.lock().await.pop() {
        return Ok(tab);
    }
    self.create_tab().await
}

async fn create_tab(&self) -> Result<TabSession, BrowserError> {
    let target_id: CreateTargetResult = self.cdp.send(
        |id| CdpCommand::CreateTarget { id, params: CreateTargetParams {
            url: "about:blank".into(),
        }},
        Duration::from_secs(3),
    ).await?;

    // Wait for attachedToTarget event
    let session_id = wait_for_attach(&self.cdp, &target_id.target_id, Duration::from_secs(3)).await?;

    Ok(TabSession { target_id: target_id.target_id, session_id, url: "about:blank".into() })
}

async fn release_tab(&self, tab: TabSession) {
    let _ = self.cdp.send(
        |id| CdpCommand::CloseTarget { id, params: CloseTargetParams {
            target_id: tab.target_id.clone(),
        }},
        Duration::from_secs(2),
    ).await;
}
```

### 4.3 Required WSS Request Headers

Captured automatically by CDP; service does not synthesize:

```
Cookie: ttwid=...; UIFID_TEMP=...
User-Agent: Mozilla/5.0 ...
Origin: https://live.douyin.com
```

### 4.4 BrowserError → HTTP Status Mapping

| BrowserError | HTTP code | retryable |
|---|---|---|
| `PoolBusy` | 503 | true |
| `BrowserDead` | 503 | true |
| `WssTimeout` | 502 | true |
| `NoWssCaptured` | 502 | false |
| `Navigation` | 502 | true |
| `ChallengeRequired` | 503 | false |
| `Cdp(...)` | 500 | true |
| `Internal` | 500 | true |

---

## 5. REST API + Configuration

### 5.1 Endpoint: POST /v1/sign

**Request**:
```json
{ "url": "https://live.douyin.com/664637748606" }
```

**Response 200**:
```json
{
  "wss_url": "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?...",
  "headers": {
    "Cookie": "ttwid=1%7C...%7C...",
    "User-Agent": "Mozilla/5.0 ...",
    "Origin": "https://live.douyin.com"
  },
  "expires_at_unix": 1782627600,
  "captured_at_unix": 1782624000
}
```

**Response 4xx/5xx**:
```json
{
  "error": {
    "code": "WSS_TIMEOUT",
    "message": "timeout waiting for WSS request after 10s",
    "retryable": true
  }
}
```

### 5.2 Error Codes

| HTTP | error.code | Meaning | retryable |
|---|---|---|---|
| 400 | `INVALID_URL` | URL format wrong | false |
| 400 | `INVALID_REQUEST` | JSON parse error | false |
| 503 | `POOL_BUSY` | All browsers saturated | true |
| 503 | `BROWSER_DEAD` | Browser restart in progress | true |
| 503 | `CHALLENGE_REQUIRED` | Douyin captcha triggered | false |
| 502 | `WSS_TIMEOUT` | No WSS request in 10s | true |
| 502 | `NO_WSS_CAPTURED` | Room may not exist | false |
| 500 | `CDP_ERROR` | CDP protocol error | true |
| 500 | `INTERNAL` | Unexpected error | true |

### 5.3 Endpoint: GET /v1/health

**Response 200**:
```json
{
  "status": "ok",
  "pool": { "size": 3, "ready": 3, "busy": 0, "dead": 0 },
  "browsers": [
    { "id": 0, "state": "Idle", "last_sign_age_ms": 1234 },
    { "id": 1, "state": "Idle", "last_sign_age_ms": 567 },
    { "id": 2, "state": "Signing", "last_sign_age_ms": 0 }
  ]
}
```

Returns 503 if any browser has been `Dead` for > 60s.

### 5.4 Configuration Schema

```toml
[service]
room_id = ""                    # not required for browser signer mode
ws_listen_addr = "0.0.0.0:8888" # unchanged
grpc_listen_addr = "0.0.0.0:50051"  # unchanged, browser signer powers this too

[browser]
edge_path = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"
pool_size = 3
max_concurrent_per_browser = 2
sign_timeout_secs = 10
health_check_interval_secs = 30
user_data_dir_template = "./data/browser-{id}"
extra_args = [
    "--headless=new",
    "--disable-blink-features=AutomationControlled",
    "--no-first-run",
    "--no-default-browser-check",
    "--window-size=1920,1080",
]

[rest]
listen_addr = "0.0.0.0:7878"

[auth]
ttwid = "1%7C..."          # initial ttwid; injected at browser startup
sessionid = ""             # optional, manual injection only

[signer]
mode = "browser"           # "browser" | "http" | "auto"
                            # auto: try browser, fallback http on persistent failure
```

### 5.5 Port Layout

| Port | Service | Notes |
|---|---|---|
| 7878 | REST API | NEW — primary external interface |
| 50051 | gRPC | unchanged — backward compat |
| 8888 | WS downstream | unchanged — event push |
| 9090 | Prometheus | unchanged |
| 9222-9224 | Edge CDP | internal, 1 per browser in pool |

### 5.6 CLI Changes

```bash
ebg start        # unchanged: full daemon (REST + gRPC + WS + browser pool)
ebg grab --url   # unchanged: CLI mode (sign + connect WSS + print)
ebg sign --url   # NEW: calls REST /v1/sign, prints JSON, exits
```

---

## 6. Testing Strategy

### 6.1 Test Pyramid

```
                  ╱╲
                 ╱  ╲           E2E (slow, real env)
                ╱ 1-2╲          - Real Douyin + real Edge
               ╱──────╲         - Full flow
              ╱ 集成测试╲       - Mock CDP + real pool logic
             ╱  5-10 个  ╲
            ╱──────────────╲
           ╱    单元测试      ╲
          ╱     20-30 个      ╲  - CDP serde, URL parsing,
         ╱──────────────────────╲ error mapping, pool scheduling
```

### 6.2 Unit Tests

| Module | Test cases | Dependencies |
|---|---|---|
| `url_parser.rs` | (existing tests) | none |
| `error.rs` | error code/retryable mapping | none |
| `cdp/commands.rs` | CdpCommand serialize, CdpEvent deserialize, Unknown fallback | none |
| `cdp/frame.rs` | WS Message ↔ CdpEvent roundtrip | none |
| `pool.rs` | round-robin distribution, semaphore exhaustion, idle return | Mock Browser |
| `signer.rs` | URL pattern match (wss://...webcast/im/push), header filter | Mock CdpClient |

### 6.3 Integration Tests

`browser_pool_test.rs`:
- `pool_round_robin_distributes_load`: 6 requests across 3 mock browsers complete in ~100ms (parallel) not ~300ms (serial)
- `pool_returns_pool_busy_when_all_saturated`: semaphore = 1, fire 5 concurrent → 4 fail with PoolBusy

`rest_api_test.rs`:
- `post_sign_returns_material_on_success`
- `post_sign_returns_400_on_invalid_url`
- `post_sign_returns_503_on_pool_busy`
- `get_health_returns_pool_state`

### 6.4 E2E Test (`#[ignore]`)

`e2e_url_to_wss.rs`:
```rust
#[tokio::test]
#[ignore = "requires real Edge + valid ttwid"]
async fn e2e_real_room_returns_signed_wss() {
    let pool = BrowserPool::start(...).await.unwrap();
    let room = std::env::var("TEST_ROOM").unwrap_or_else(|_| "664637748606".into());
    let material = pool.sign(&room).await.expect("sign failed");

    assert!(material.url.starts_with("wss://webcast"));
    assert!(material.url.contains("webcast/im/push"));
    assert!(material.headers.contains_key("Cookie"));

    let valid = quick_wss_handshake_check(&material).await;
    assert!(valid, "WSS handshake failed - ttwid may be expired");
}
```

Manual run: `cargo test -- --ignored`

### 6.5 Mock Infrastructure

`MockCdpClient` (in `crates/collector/src/cdp/mock.rs`):
```rust
pub struct MockCdpClient {
    responses: HashMap<String, serde_json::Value>,  // method → canned response
    events: Vec<CdpEvent>,                          // pre-recorded event sequence
}

#[async_trait]
impl CdpTransport for MockCdpClient {
    async fn send_raw<R: DeserializeOwned + Send + 'static>(
        &self,
        cmd_json: serde_json::Value,
        timeout: Duration,
    ) -> Result<R, CdpError>;

    fn subscribe_events(&self) -> mpsc::UnboundedReceiver<CdpEvent>;
}
```

`MockBrowser` (in `crates/collector/src/browser/mock.rs`):
```rust
pub struct MockBrowser {
    sign_delay: Duration,
    fail_every_n: u32,        // test intermittent failure recovery
    canned_material: Option<SignedWssMaterial>,
}
```

### 6.6 Coverage Targets

| Module | Target |
|---|---|
| `cdp/commands.rs` | 100% |
| `cdp/frame.rs` | 100% |
| `pool.rs` | 90%+ |
| `signer.rs` | 80%+ |
| `rest_api/sign.rs` | 80%+ |
| Overall | 80%+ |

### 6.7 CI Matrix

| Test type | Local | CI |
|---|---|---|
| Unit + integration (mock) | always | always |
| E2E (real Edge) | manual `--ignored` | skipped (CI has no Edge) |

---

## 7. Risks and Mitigations

### 7.1 Risk Matrix

| # | Risk | Probability | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Edge upgrade breaks CDP compat | Medium | High | Small command subset; Unknown event fallback in CdpEvent |
| R2 | Douyin detects headless | **High** | High | Real Edge + `--headless=new` + fingerprint args; per-sign tab isolation; 1 retry on failure |
| R3 | ttwid expires | Medium | Medium | Parse Expires from cookie header; proactive refresh via home-page revisit |
| R4 | Edge memory leak | Medium | Medium | Health check monitors RSS; auto-restart > 1GB |
| R5 | Edge not installed at edge_path | Low | Medium | Pool::start returns clear error with actionable message |
| R6 | Sign race / cross-room pollution | Low | High | Open-per-sign tab strategy; session_id isolation |
| R7 | WSS URL token drift | High | Low | Designed for: caller always uses fresh material |
| R8 | Douyin captcha triggered | Medium | High | Detect captcha DOM element; return `CHALLENGE_REQUIRED` (non-retryable) |

### 7.2 R2: Douyin Anti-Bot — Detailed

**Detection signals**:
- `Network.responseReceived` returns 4xx for otherwise valid URL
- `Network.requestWillBeSent` shows `wss://` but no `Network.webSocketFrameSent` within 1s
- `room.status` is non-2 even though room appears online

**Mitigations (cumulative)**:
1. Base args (Section 3.7): `--headless=new`, `AutomationControlled` disabled, realistic viewport
2. Real Edge UA (auto from Edge itself)
3. Real Canvas/WebGL fingerprint (new headless mode renders normally)
4. Single-sign failure → close tab + reopen (new fingerprint); max 2 retries
5. Rate limiting: 6 sign/minute per browser (configurable)

### 7.3 R3: ttwid Expiry — Detailed

**Detection**: At browser startup, navigate once to home page, parse `Set-Cookie: ttwid=...; Expires=...` from response headers.

**Proactive refresh**: When remaining lifetime < 24h:
- Use `Browser.setPermission` not applicable (cookie can't be refreshed this way)
- Real mechanism: revisit home page; CDN re-issues ttwid with fresh `Expires` if user cookie still valid
- If refresh fails: log WARN, continue with old ttwid until it actually expires

**Helper**:
```rust
fn parse_cookie_expires(set_cookie: &str) -> Option<SystemTime> {
    // Extract `Expires=Wed, 23 Jun 2027 05:20:00 GMT`
    // Use `chrono` or `time` crate for RFC1123 parsing
}
```

### 7.4 R8: Captcha Challenge — Detailed

**Detection** (post-navigation, optional DOM check):
```rust
async fn detect_challenge(cdp: &CdpClient, session_id: &str) -> Result<bool, CdpError> {
    let result: serde_json::Value = cdp.send(|id| CdpCommand::RuntimeEvaluate {
        id, params: RuntimeEvaluateParams {
            expression: "document.querySelector('.captcha-slider, .verify-slider') !== null"
                .into(),
            session_id: Some(session_id.into()),
            ..Default::default()
        }
    }, Duration::from_secs(2)).await?;
    Ok(result.value.as_bool().unwrap_or(false))
}
```

**Response**: Return `ChallengeRequired` (HTTP 503, non-retryable) with message guiding user to change IP / wait / upload new cookie.

### 7.5 Fallback Path

Configuration `[signer] mode`:
- `"browser"`: only use browser-based signer
- `"http"`: only use legacy `ImFetcher` HTTP path (kept for development/debugging)
- `"auto"`: try browser first, fall back to HTTP after 3 consecutive failures in 30s window

### 7.6 Prometheus Metrics (new)

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `sign_total` | counter | `result={ok,err}` | Total sign calls |
| `sign_duration_seconds` | histogram | `result` | Sign latency distribution |
| `browser_state` | gauge | `browser_id, state` | 0=Dead, 1=Idle, 2=Signing, 3=Recovering |
| `browser_sign_inflight` | gauge | `browser_id` | In-flight sign requests |
| `wss_capture_timeout_total` | counter | — | WSS timeout occurrences |
| `pool_busy_reject_total` | counter | — | Pool-exhausted rejections |

### 7.7 Upgrade Strategy

When Edge major-version changes:
1. Run test suite with `mode = "auto"` (both paths active)
2. Compare browser-path vs http-path success rate over 24h
3. If browser-path < 99%, investigate CDP command subset
4. Promote `mode = "browser"` only after sustained stability

---

## Acceptance Criteria

The feature is considered complete when:

1. `cargo test` passes with ≥ 80% coverage on new modules
2. Manual E2E test (`cargo test -- --ignored`) returns valid `SignedWssMaterial` for at least one real Douyin room
3. `POST /v1/sign` returns HTTP 200 with `wss_url` starting with `wss://webcast` for valid input
4. `GET /v1/health` reflects accurate pool state
5. Pool maintains throughput ≥ 4 req/s sustained under 10 concurrent requests
6. No browser leaks (RSS bounded, restart count < 1/hour under normal load)
7. Backward compat: existing gRPC `ProvideSignedWss` clients continue to work (now powered by browser path internally)