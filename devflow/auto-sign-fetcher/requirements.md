# Requirements — auto-sign-fetcher

> Phase 2 产出。共 10 项需求（R-001 ~ R-010），按 P0/P1 优先级组织。

## 1. 项目元信息

| 字段 | 值 |
|------|---|
| Feature | auto-sign-fetcher |
| 创建时间 | 2026-06-28 |
| DevFlow 版本 | 3.0 |
| 关联 commit | e59547c (custom-barrage MVP merge) |
| Worktree | `.claude/worktrees/devflow-auto-sign-fetcher` |
| 目标分支 | `main` |
| Feature 分支 | `auto-sign-fetcher` |

## 2. 范围摘要（来自 Phase 1）

### 目标
用户输入抖音直播间 URL（`https://live.douyin.com/<web_rid>`），服务自动完成签名链路并连接到弹幕流。**全程无需用户提供已签名 URL**。

### 包含项
1. URL 解析器（仅 `live.douyin.com` 长链 + 容错提取）
2. Cookie 配置（`config.toml` 中粘贴 `ttwid` / `sessionid`）
3. `webcast/room/web/enter/` 调用（`web_rid` → `room_id`）
4. `webcast/im/fetch/` 调用（`room_id` + cookie → 签名后 wss URL）
5. WSS 连接 + 弹幕处理（复用 `core/dispatcher` + `filter` + `dedup`）
6. 结构化错误响应（错误码 + `Retryable` 标志）
7. 双输入接口：CLI（`ebg grab --url <url>`）+ gRPC（`ProvideSignedWss` 扩展 `url` 字段）

### 排除项
- 签名算法逆向实现（由 `collector/` 实现细节，本次只做集成）
- 短链 `v.douyin.com` 重定向跟随（反检测风险）
- 自动扫码登录
- 重试逻辑（调用方根据 `Retryable` 自行决定）
- 纯 `room_id` 数字直传
- 降级到 `custom-barrage` 路径（独立 feature）

### 成功标准
1. `ebg grab --url https://live.douyin.com/664637748606` 即可拿到弹幕流
2. 无 scheme 的 URL 同样可用
3. Cookie 过期 → `CookieExpired` + `Retryable=false`
4. 算法失效 → `AlgorithmChanged` + `Retryable=false`
5. 网络抖动 → `NetworkTransient` + `Retryable=true`
6. 集成测试可验证 URL → 弹幕全链路（mock 签名 server）
7. 不破坏现有 `custom-barrage` 62/62 测试

## 3. 需求清单

### 📌 P0：MVP 核心路径（7 项）

#### R-001：URL 解析器
- **描述**：从用户输入 URL 提取 `web_rid`，仅支持 `live.douyin.com`，容错处理无 scheme / query / fragment
- **依赖**：无
- **验收标准**：
  - [ ] `https://live.douyin.com/664637748606` → `web_rid="664637748606"`
  - [ ] `live.douyin.com/xxx`（无 scheme）→ `web_rid="xxx"`
  - [ ] `live.douyin.com/xxx?foo=bar` → `web_rid="xxx"`
  - [ ] `live.douyin.com/xxx#section` → `web_rid="xxx"`
  - [ ] `v.douyin.com/abc` → `UrlFormatNotSupported` 错误
  - [ ] `""` → `EmptyUrl` 错误
  - [ ] 单元测试覆盖 ≥6 个 case

#### R-002：Cookie 配置加载
- **描述**：从 `config.toml` 的 `[auth]` 段读取 `ttwid` / `sessionid`
- **依赖**：无
- **验收标准**：
  - [ ] `config.toml` 新增 `[auth]` 段（`ttwid`、`sessionid`）
  - [ ] 启动时校验 cookie 字段至少一个非空
  - [ ] 配置缺失 → `ConfigMissing` 错误
  - [ ] `config.example.toml` 更新示例
  - [ ] 单元测试：空 / 缺字段 / 完整三种情况

#### R-006：结构化错误响应
- **描述**：定义 `SignatureError` 统一错误类型，含错误码 + `retryable` 标志
- **依赖**：无（被 R-003 / R-004 使用）
- **验收标准**：
  - [ ] 定义 `SignatureError` 枚举
  - [ ] 变体：`CookieExpired` / `AlgorithmChanged` / `RoomNotFound` / `UrlFormatNotSupported` / `NetworkTransient` / `EmptyUrl` / `ConfigMissing`
  - [ ] 每个变体带 `retryable: bool`
  - [ ] 实现 `std::error::Error` + `Display`
  - [ ] 单元测试：每个变体的 `retryable` 值正确

#### R-003：room_info 调用
- **描述**：HTTP 调用 `webcast/room/web/enter/` 将 `web_rid` 转为真实 `room_id`
- **依赖**：R-001, R-002, R-006
- **验收标准**：
  - [ ] HTTP 调用携带 cookie
  - [ ] 解析响应 → 提取 `room_id`
  - [ ] 网络错误 → `NetworkTransient` + `Retryable=true`
  - [ ] 房间不存在 → `RoomNotFound` + `Retryable=false`
  - [ ] HTTP 401/403 → `CookieExpired` + `Retryable=false` + 提示重新粘贴
  - [ ] 集成测试覆盖成功 + 4 种失败

#### R-004：im_fetch 调用
- **描述**：HTTP 调用 `webcast/im/fetch/` 拿到签名后的 wss URL + X-MS-STUB + signature
- **依赖**：R-003, R-006
- **验收标准**：
  - [ ] HTTP 调用携带 `room_id` + cookie
  - [ ] 解析响应 → 提取 wss URL + 动态参数
  - [ ] 网络错误 → `NetworkTransient` + `Retryable=true`
  - [ ] 响应格式异常 → `AlgorithmChanged` + `Retryable=false`
  - [ ] 集成测试覆盖成功 + 3 种失败

#### R-005：WSS 连接 + 弹幕处理
- **描述**：用签名后 wss URL 建立连接，复用现有 `core/dispatcher` + `core/filter` + `core/dedup`
- **依赖**：R-004
- **验收标准**：
  - [ ] WSS 客户端建立连接（tokio-tungstenite）
  - [ ] 心跳 / pong 处理（沿用 custom-barrage）
  - [ ] 消息解码（沿用 `core/decoder`）
  - [ ] 消息分发（沿用 `core/dispatcher`）
  - [ ] 集成测试：mock WSS server 验证消息流

#### R-009：向后兼容
- **描述**：现有 custom-barrage 62/62 测试全部通过
- **依赖**：无（贯穿全程）
- **验收标准**：
  - [ ] `cargo test` → 62/62 pass（custom-barrage 既有测试）
  - [ ] 不修改现有 API 签名（除非新增可选字段）
  - [ ] 不修改现有 `core/dispatcher` / `filter` / `dedup` 实现
  - [ ] 只新增模块，不修改现有模块

### 📌 P1：用户接口 + 端到端（3 项）

#### R-007：CLI 接口
- **描述**：`ebg grab --url <url>` 启动弹幕流
- **依赖**：R-001 ~ R-006
- **验收标准**：
  - [ ] `ebg grab --url <url>` 启动弹幕流
  - [ ] `ebg grab --url <url> --cookie-file <path>` 覆盖 config
  - [ ] 错误时输出结构化错误 + 非零退出码
  - [ ] 单元测试：CLI 参数解析

#### R-008：gRPC 接口扩展
- **描述**：`ProvideSignedWss` 增加 `url` 可选字段
- **依赖**：R-001 ~ R-006
- **验收标准**：
  - [ ] `proto/barrage.v1/wss.proto` 增加 `ProvideSignedWssRequest.url` 字段
  - [ ] 向后兼容：旧客户端（不传 `url`）继续走 custom-barrage 路径
  - [ ] 新客户端（传 `url`）走 auto-sign 路径
  - [ ] prost-build 重新生成 stubs
  - [ ] 集成测试：旧客户端 + 新客户端两种场景

#### R-010：集成测试
- **描述**：mock 签名 server 验证 URL → 弹幕全链路
- **依赖**：R-007, R-008
- **验收标准**：
  - [ ] 启动本地 mock HTTP server（响应 room_info + im_fetch）
  - [ ] 启动本地 mock WSS server（推送测试弹幕）
  - [ ] `ebg grab --url` 调用全链路
  - [ ] 验证 dispatcher 收到测试弹幕
  - [ ] 端到端测试覆盖成功路径

## 4. 依赖图

```
R-001 (URL)──────┐
                 ├─→ R-003 (room_info) ─→ R-004 (im_fetch) ─→ R-005 (WSS) ─→ R-007 (CLI) ─┐
R-002 (Cookie)───┤                                                                         ├─→ R-010 (E2E)
                 └─→ R-006 (错误类型) ──────────────────────────────────────→ R-008 (gRPC)┘
                                                                                ↑
                                                                            R-009 (兼容)
                                                                            （贯穿全程）
```

## 5. 自动加载 memory

本次 DevFlow 启动时自动加载的 memory（已确认范围决策）：

- `[[project-future-url-direct-mode]]` — 下次 feature 计划 + 用户选 A 收尾的决策
- `[[project-eleven-barrage-grab]]` — 项目工作区结构 + MVP 接入点
- `[[feedback-toolchain-on-d-drive]]` — Rust 工具链 D 盘配置
- `[[feedback-protoc-build-rs-pattern]]` — prost-build serde derive 注入

---

*由 DevFlow 追踪。请勿手动编辑。*
