# Test Cases — auto-sign-fetcher

> Phase 3 产出。每个 R-xxx 至少 1 个 TC-xxx。按 L1/L2/L3 分层。

## 1. 层级说明

| 层级 | 自动化 | 工具 | 触发时机 |
|------|--------|------|---------|
| **L1 烟雾** | ✅ | `cargo test` / `cargo build` / `cargo clippy` | 每个 commit + Phase 5 入口 |
| **L2 交互** | ✅ | `tokio::test` + mock HTTP/WSS server | 每个 commit + Phase 5 |
| **L3 手工** | ❌ | 真实抖音直播间 + 真实 cookie | Phase 5 用户手工验收 |

## 2. TC 清单

### 2.1 L1 烟雾（编译 + 既有测试）

#### TC-L1-001：cargo build 通过（Windows + Linux）
- **关联 R-xxx**：R-001, R-002, R-003, R-004, R-005, R-006, R-008
- **步骤**：
  1. 在 Windows 执行 `cargo build --release --target x86_64-pc-windows-gnu`
  2. 在 Linux 执行 `cargo build --release --target x86_64-unknown-linux-gnu`
  3. 检查 exit code = 0
- **预期**：两个平台编译通过，无 warning（`#![deny(warnings)]` 已设）

#### TC-L1-002：cargo test 通过（既有 62/62 + 新增）
- **关联 R-xxx**：R-009 + 全部新 R-xxx
- **步骤**：
  1. 执行 `cargo test --release`
  2. 检查既有 62 个测试全部通过
  3. 检查新增测试全部通过（≥15 个新单测）
- **预期**：既有测试 0 regression；新增测试全 pass

#### TC-L1-003：cargo clippy 通过
- **关联 R-xxx**：R-001 ~ R-010
- **步骤**：
  1. 执行 `cargo clippy --all-targets --all-features -- -D warnings`
  2. 检查 exit code = 0
- **预期**：无 warning，无建议

#### TC-L1-004：cargo fmt --check 通过
- **关联 R-xxx**：R-001 ~ R-010
- **步骤**：
  1. 执行 `cargo fmt --check`
- **预期**：格式符合规范

#### TC-L1-005：prost-build 重新生成 stub
- **关联 R-xxx**：R-008
- **步骤**：
  1. 修改 `crates/proto/proto/signed.proto` 后执行 `cargo build -p eleven-barrage-proto`
  2. 检查 `target/debug/build/proto-*/out/signed.rs` 生成
  3. 检查 `ProvideSignedWssRequest` / `ProvideSignedWssResponse` 类型存在
- **预期**：prost-build 成功生成 gRPC stub

### 2.2 L2 交互（单元/集成测试）

#### TC-L2-001：UrlParser 接受完整 URL
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("https://live.douyin.com/664637748606")`
  2. 验证返回 `Ok("664637748606")`
- **预期**：成功提取 web_rid

#### TC-L2-002：UrlParser 接受无 scheme URL
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("live.douyin.com/664637748606")`
  2. 验证返回 `Ok("664637748606")`
- **预期**：自动补全 scheme 后成功

#### TC-L2-003：UrlParser 接受带 query 的 URL
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("https://live.douyin.com/xxx?foo=bar&baz=qux")`
  2. 验证返回 `Ok("xxx")`（query 被忽略）
- **预期**：提取 web_rid，query 不影响

#### TC-L2-004：UrlParser 接受带 fragment 的 URL
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("https://live.douyin.com/xxx#section")`
  2. 验证返回 `Ok("xxx")`（fragment 被忽略）
- **预期**：提取 web_rid，fragment 不影响

#### TC-L2-005：UrlParser 拒绝 v.douyin.com
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("https://v.douyin.com/abc123")`
  2. 验证返回 `Err(SignatureError::UrlFormatNotSupported)`
  3. 验证 `error.retryable() == false`
- **预期**：域名不匹配，返回结构化错误

#### TC-L2-006：UrlParser 拒绝空字符串
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("")`
  2. 验证返回 `Err(SignatureError::EmptyUrl)`
- **预期**：返回空 URL 错误

#### TC-L2-007：UrlParser 拒绝其他域名
- **关联 R-xxx**：R-001
- **步骤**：
  1. 调用 `UrlParser::parse("https://example.com/xxx")`
  2. 验证返回 `Err(SignatureError::UrlFormatNotSupported)`
- **预期**：域名白名单过滤生效

#### TC-L2-008：AuthConfig validate 缺失 cookie
- **关联 R-xxx**：R-002
- **步骤**：
  1. 构造 `AuthConfig { ttwid: "", sessionid: "" }`
  2. 调用 `auth.validate()`
  3. 验证返回 `Err(SignatureError::ConfigMissing)`
- **预期**：拒绝空 cookie

#### TC-L2-009：AuthConfig validate 完整 cookie
- **关联 R-xxx**：R-002
- **步骤**：
  1. 构造 `AuthConfig { ttwid: "test_ttwid", sessionid: "" }`
  2. 调用 `auth.validate()`
  3. 验证返回 `Ok(())`
- **预期**：至少一个非空即通过

#### TC-L2-010：AuthConfig cookie header 拼接
- **关联 R-xxx**：R-002
- **步骤**：
  1. 构造 `AuthConfig { ttwid: "abc", sessionid: "def" }`
  2. 调用 `auth.to_cookie_header()`
  3. 验证返回 `"ttwid=abc; sessionid=def"`
- **预期**：按正确顺序拼接

#### TC-L2-011：SignatureError::CookieExpired retryable=false
- **关联 R-xxx**：R-006
- **步骤**：
  1. 创建 `SignatureError::CookieExpired`
  2. 验证 `error.retryable() == false`
- **预期**：用户需重新粘贴，不应自动重试

#### TC-L2-012：SignatureError::NetworkTransient retryable=true
- **关联 R-xxx**：R-006
- **步骤**：
  1. 创建 `SignatureError::NetworkTransient { source: ... }`
  2. 验证 `error.retryable() == true`
- **预期**：网络抖动应允许调用方重试

#### TC-L2-013：SignatureError::AlgorithmChanged retryable=false
- **关联 R-xxx**：R-006
- **步骤**：
  1. 创建 `SignatureError::AlgorithmChanged`
  2. 验证 `error.retryable() == false`
- **预期**：算法失效不应自动重试

#### TC-L2-014：ImFetcher 成功路径（mock HTTP）
- **关联 R-xxx**：R-004
- **步骤**：
  1. 启动本地 mock HTTP server（返回预定义 im_fetch JSON 响应）
  2. 构造 `ImFetcher` 指向 mock server
  3. 调用 `im_fetcher.fetch("test_room_id")`
  4. 验证返回 `Ok(SignedWssMaterial { url: "wss://...", ... })`
- **预期**：成功解析响应

#### TC-L2-015：ImFetcher 401 → CookieExpired
- **关联 R-xxx**：R-004
- **步骤**：
  1. 启动 mock HTTP server 返回 401
  2. 调用 `im_fetcher.fetch(...)`
  3. 验证返回 `Err(SignatureError::CookieExpired)`
  4. 验证 `error.retryable() == false`
- **预期**：HTTP 401 映射为 CookieExpired

#### TC-L2-016：ImFetcher 网络超时 → NetworkTransient
- **关联 R-xxx**：R-004
- **步骤**：
  1. 启动 mock HTTP server 延迟响应（超过 5s）
  2. 配置 `ImFetcher` timeout = 3s
  3. 调用 `im_fetcher.fetch(...)`
  4. 验证返回 `Err(SignatureError::NetworkTransient)`
  5. 验证 `error.retryable() == true`
- **预期**：超时映射为 NetworkTransient

#### TC-L2-017：ImFetcher 响应格式异常 → AlgorithmChanged
- **关联 R-xxx**：R-004
- **步骤**：
  1. 启动 mock HTTP server 返回 200 + 错误 JSON（缺字段）
  2. 调用 `im_fetcher.fetch(...)`
  3. 验证返回 `Err(SignatureError::AlgorithmChanged)`
- **预期**：解析失败映射为 AlgorithmChanged

#### TC-L2-018：RoomInfoApi 404 → RoomNotFound
- **关联 R-xxx**：R-003
- **步骤**：
  1. 启动 mock HTTP server 返回 404
  2. 调用 `room_api.get("nonexistent")`
  3. 验证返回映射为 `SignatureError::RoomNotFound`
- **预期**：房间不存在映射

#### TC-L2-019：AutoSigner 端到端成功（mock）
- **关联 R-xxx**：R-003, R-004
- **步骤**：
  1. 启动 mock room_info + mock im_fetch server
  2. 构造 `AutoSigner`
  3. 调用 `signer.sign("test_web_rid")`
  4. 验证返回 `Ok(SignedWssMaterial)`
  5. 验证内部调用了 room_api + im_fetcher（次数 = 1）
- **预期**：完整签名链路通过

#### TC-L2-020：AutoSigner 任一步失败 → 传播错误
- **关联 R-xxx**：R-003, R-004
- **步骤**：
  1. 启动 mock room_info 返回 404
  2. 调用 `signer.sign("nonexistent")`
  3. 验证返回 `Err(SignatureError::RoomNotFound)`
  4. 验证 im_fetcher 未被调用
- **预期**：失败快速返回，不调用下游

#### TC-L2-021：gRPC ProvideSignedWss 接受 URL（新客户端）
- **关联 R-xxx**：R-008
- **步骤**：
  1. 启动 service（mock AutoSigner）
  2. gRPC 客户端调用 `ProvideSignedWss { url: Some("https://live.douyin.com/xxx") }`
  3. 验证返回 `Ok(material)` 或结构化错误
- **预期**：新客户端走 auto-sign 路径

#### TC-L2-022：gRPC ProvideSignedWss 拒绝无 URL（向后兼容）
- **关联 R-xxx**：R-008
- **步骤**：
  1. 启动 service
  2. gRPC 客户端调用 `ProvideSignedWss { url: None }`
  3. 验证返回 `Err(Status::invalid_argument(...))`
- **预期**：旧客户端被引导到 custom-barrage 路径

#### TC-L2-023：CLI ebg grab --url 成功路径
- **关联 R-xxx**：R-007
- **步骤**：
  1. mock AutoSigner 注入成功响应
  2. 执行 `ebg grab --url https://live.douyin.com/test`
  3. 验证进程启动并连接到 mock WSS
- **预期**：CLI 启动 + 全链路成功

#### TC-L2-024：CLI ebg grab --url 错误输出
- **关联 R-xxx**：R-007
- **步骤**：
  1. mock AutoSigner 返回 `SignatureError::CookieExpired`
  2. 执行 `ebg grab --url https://live.douyin.com/test`
  3. 验证 stderr 输出包含 `"code: COOKIE_EXPIRED"` 和 `"retryable: false"`
  4. 验证 exit code = 1
- **预期**：结构化错误 + 非零退出码

#### TC-L2-025：WSS 连接 + dispatcher 收到 mock 弹幕
- **关联 R-xxx**：R-005
- **步骤**：
  1. 启动本地 mock WSS server，推送 1 条 ChatMessage
  2. service 用 signed URL 连接
  3. 验证 `core/dispatcher` 收到该消息
  4. 验证 WS/gRPC 下游收到 `BarrageEvent`
- **预期**：WSS 链路打通

#### TC-L2-026：config.toml 解析 [auth] 段
- **关联 R-xxx**：R-002
- **步骤**：
  1. 准备 TOML：`[auth]\nttwid = "test_ttwid"\nsessionid = "test_sessionid"`
  2. 调用 `AppConfig::from_str(toml)`
  3. 验证 `cfg.auth.ttwid == "test_ttwid"`
  4. 验证 `cfg.auth.validate().is_ok()`
- **预期**：配置正确加载

#### TC-L2-027：config.example.toml 包含 [auth] 示例
- **关联 R-xxx**：R-002
- **步骤**：
  1. 读取 `config.example.toml`
  2. 验证包含 `[auth]` 段
  3. 验证包含 `ttwid` 字段说明
- **预期**：示例文件更新

#### TC-L2-028：proto 兼容性（双向）
- **关联 R-xxx**：R-008
- **步骤**：
  1. 旧 proto 客户端（无 `url` 字段）序列化 `ProvideSignedWssRequest`
  2. 新 proto 服务端反序列化
  3. 验证 `req.url == None`
  4. 验证不 panic
- **预期**：proto 层面向后兼容

### 2.3 L3 手工（真实环境验收）

#### TC-L3-001：真实抖音直播间 URL → 弹幕
- **关联 R-xxx**：R-001 ~ R-010（全链路）
- **步骤**：
  1. 准备有效的 `ttwid` cookie（从浏览器复制）
  2. 填入 `config.toml` 的 `[auth]` 段
  3. 找到正在直播的抖音房间 URL（如 `https://live.douyin.com/741891423654`）
  4. 执行 `ebg grab --url <url>`
  5. 观察 WS/gRPC 下游是否收到弹幕事件
  6. 验证弹幕内容与实际直播间一致
- **预期**：用户在浏览器看到的弹幕，服务端也能收到

#### TC-L3-002：错误 cookie → 明确错误提示
- **关联 R-xxx**：R-002, R-006
- **步骤**：
  1. 配置过期/无效的 `ttwid`
  2. 执行 `ebg grab --url <valid_url>`
  3. 观察错误输出
  4. 验证错误信息明确提示"请重新粘贴 ttwid"
- **预期**：用户能根据错误信息自助修复

#### TC-L3-003：房间已结束 → RoomNotFound
- **关联 R-xxx**：R-003
- **步骤**：
  1. 找一个已下播的抖音房间 URL
  2. 执行 `ebg grab --url <ended_url>`
  3. 观察错误输出
  4. 验证错误为 `RoomNotFound` 且不重试
- **预期**：清晰区分房间状态

#### TC-L3-004：旧 gRPC 客户端仍能工作（向后兼容）
- **关联 R-xxx**：R-008
- **步骤**：
  1. 启动 service
  2. 用旧 custom-barrage gRPC 客户端连接（不传 url 字段）
  3. 验证旧路径仍能工作
- **预期**：不破坏现有集成方

---

## 3. 覆盖率目标

| 模块 | L1 覆盖率 | L2 覆盖率 | 备注 |
|------|----------|----------|------|
| `collector::url` | 100% | 100% | 纯函数 |
| `collector::error` | 100% | 100% | 枚举分支 |
| `collector::signer` | ≥80% | ≥90% | 含 mock 测试 |
| `collector::im_fetch` | ≥80% | ≥90% | 含 mock HTTP |
| `service::grpc_server::provide_signed_wss` | ≥80% | ≥85% | 集成测试 |
| `cli::grab` | ≥70% | ≥80% | CLI 测试 |

---

*由 DevFlow 追踪。请勿手动编辑。*
