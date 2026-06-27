# Requirements — custom-barrage

> Phase 2 产出。共 33 项需求（R-001 ~ R-033），按 P0/P1/P2 优先级组织。

## 1. 项目元信息

| 字段 | 值 |
|------|---|
| Feature | custom-barrage |
| 创建时间 | 2026-06-27 |
| DevFlow 版本 | 3.0 |
| 关联 commit | (initial) |

## 2. 范围摘要（来自 Phase 1）

- **目标**：自研 Rust 全栈的高性能直播弹幕服务，复用 `DouyinBarrageGrab` 的协议解码经验
- **MVP 事件**：Chat（弹幕）、Gift（礼物）、Like（点赞）
- **保留 schema（不推送）**：Member、Social、Control、RoomUserSeq、Fansclub
- **下游接口**：WebSocket（JSON）+ gRPC（Protobuf）双通道
- **平台**：Windows + Linux 双平台并行编译
- **部署**：单二进制 daemon
- **架构预留**：多房间（内部多房间抽象，MVP 单房间验证）

## 3. 需求清单

### 📌 P0：MVP 核心路径（15 项）

#### R-001：Rust workspace 结构
- **描述**：multi-crate workspace，至少包含 `core`、`collector`、`service`、`proto`、`cli` 五个 crate
- **验收标准**：
  - [ ] `cargo build --release` 在 Windows 和 Linux 各产出可运行二进制
  - [ ] 5 个 crate 各自独立可测试
  - [ ] workspace 根 `Cargo.toml` 配置正确（共享依赖版本、`cargo metadata` 可枚举所有 crate）

#### R-002：跨平台构建
- **描述**：Windows + Linux 单二进制 daemon
- **验收标准**：
  - [ ] `cargo build --release --target x86_64-unknown-linux-gnu` 成功
  - [ ] `cargo build --release --target x86_64-pc-windows-msvc` 成功
  - [ ] 两个二进制文件大小 < 30MB（无 GUI 依赖）

#### R-003：配置加载
- **描述**：TOML + 环境变量 + CLI flags 三层覆盖
- **验收标准**：
  - [ ] 默认配置文件 `config.toml` 加载
  - [ ] 环境变量 `ELEVEN_BARRAGE_*` 覆盖配置
  - [ ] CLI flags（如 `--room-id`）优先级最高
  - [ ] 配置错误时给出明确错误信息并退出非零码

#### R-006：复用原项目 protobuf schema
- **描述**：从 `DouyinBarrageGrab/BarrageGrab/Modles/ProtoEntity/` 迁移 `.proto` 定义
- **验收标准**：
  - [ ] `.proto` 文件提取并迁移
  - [ ] `prost-build` 在 build.rs 中生成 Rust 类型
  - [ ] 生成的类型与原项目字段命名 1:1 对应

#### R-007：gzip 解压 + 外层 `WssResponse` 解码
- **描述**：参考 `WssBarrageGrab.cs:104-176`
- **验收标准**：
  - [ ] 处理 gzip 压缩标志（`compress_type=gzip`）
  - [ ] wire_type 前置校验（0-5 合法范围）
  - [ ] 单次解压失败不抛异常，仅记日志

#### R-008：内层 `Response` 解码 + 消息分发
- **描述**：按 `msg.Method` 路由到具体消息类型
- **验收标准**：
  - [ ] `Response.Messages` 列表遍历
  - [ ] 按 `msg.Method` 字段路由到 8 种消息类型
  - [ ] 消息 ID 缓存（`msgId`）避免重复处理（参考原项目 320 条环形缓冲）

#### R-009：8 种消息类型解码器
- **描述**：Chat/Like/Gift/Member/Social/Control/RoomUserSeq/Fansclub
- **验收标准**：
  - [ ] 对应 8 个 protobuf 消息类型完整解码
  - [ ] 每种类型在单元测试中至少 1 个 fixture 用例
  - [ ] 解码失败时该条跳过、不影响其他消息

#### R-014：service 主入口与 daemon 模式
- **描述**：服务可作为 daemon 启动
- **验收标准**：
  - [ ] `eleven-barrage-grab service start` 启动 daemon
  - [ ] 优雅关闭（SIGTERM / Ctrl+C 触发资源清理）
  - [ ] systemd / Windows Service 集成说明文档

#### R-015：单房间 wss 长连接管理
- **描述**：Tokio async 维护单房间 wss 连接
- **验收标准**：
  - [ ] 使用 `tokio-tungstenite` 连接抖音 wss 端点
  - [ ] 自动 ping（30s 间隔）+ pong 处理
  - [ ] 连接断开后指数退避重连（1s → 2s → 4s → ... → 60s 上限）

#### R-016：MVP 事件过滤
- **描述**：仅推送 Chat/Gift/Like
- **验收标准**：
  - [ ] 配置文件可控制 `push_event_types`（默认 Chat/Gift/Like）
  - [ ] Member/Social/Control/RoomUserSeq/Fansclub 5 种 schema 保留但不推送
  - [ ] 单元测试验证过滤逻辑

#### R-020：WebSocket 服务端（JSON）
- **描述**：下游消费者通过 WS 接入
- **验收标准**：
  - [ ] 默认监听 `0.0.0.0:8888`，可通过配置修改
  - [ ] 客户端连接时发送 `BarrageEvent` JSON 流
  - [ ] 消息格式 `{"event_type": "ChatMessage", "data": {...}}`
  - [ ] 客户端断线自动清理资源

#### R-023：Watchdog 后台线程
- **描述**：监控主进程健康
- **验收标准**：
  - [ ] 启动后台 tokio task，每 30s 检查主任务状态
  - [ ] 主任务无响应 > 60s 时触发告警日志（不强制重启进程）
  - [ ] 优雅退出时自动清理 watchdog

#### R-025：5s 心跳保底
- **描述**：参考 `DouyinBarrageGrab` commit `85d9514`
- **验收标准**：
  - [ ] 即使主消息流空闲，每 5s 发送 ping
  - [ ] 心跳超时自动断开重连
  - [ ] metrics 暴露心跳成功率

#### R-026：metrics 基础设施
- **描述**：QPS / 延迟 / 错误率
- **验收标准**：
  - [ ] 暴露 Prometheus 格式 `/metrics` 端点（端口可配置）
  - [ ] 关键指标：`barrage_events_total`, `barrage_processing_duration_seconds`, `wss_connection_state`, `decode_errors_total`
  - [ ] 默认端口 9090

### 📌 P1：重要扩展（13 项）

#### R-004：错误处理基础设施
- **描述**：`thiserror` + `anyhow`
- **验收标准**：
  - [ ] 业务错误用 `thiserror` 强类型枚举
  - [ ] 顶层错误用 `anyhow::Result` 链式传播
  - [ ] 关键错误路径有 `tracing` span 关联

#### R-005：可恢复性基础设施
- **描述**：`ResiliencePipeline` trait
- **验收标准**：
  - [ ] `ResiliencePipeline` trait 定义（重试 / 熔断 / 退避）
  - [ ] 默认实现：指数退避 + 抖动
  - [ ] 单元测试覆盖各种故障场景

#### R-010：房间元数据 API 客户端
- **描述**：复刻 `DyApiHelper.GetRoomInfoForApi`
- **验收标准**：
  - [ ] HTTP GET `https://live.douyin.com/webcast/room/web/enter/`
  - [ ] query 参数与原项目一致（`aid=6383`, `device_platform=web`, `web_rid={room_id}`）
  - [ ] Headers：UA + Referer + 可选 Cookie

#### R-013：MITM 兜底模式
- **描述**：复刻原项目系统代理抓包思路
- **验收标准**：
  - [ ] 当 wss 主动连接失败时，可降级到 MITM 模式
  - [ ] 复用原项目 Titanium.Web.Proxy 思路（Rust 实现可用 `hyper` + 自定义 CONNECT 处理）
  - [ ] 模式切换通过配置或 CLI flag 控制

#### R-017：消息去重
- **描述**：`msgId` 缓存（参考原项目）
- **验收标准**：
  - [ ] 每种消息类型独立环形缓冲（容量 300）
  - [ ] msgId 已存在则跳过（避免重复推送）
  - [ ] 内存上限可控（不会 OOM）

#### R-019：多房间架构抽象
- **描述**：架构预留，MVP 单房间验证
- **验收标准**：
  - [ ] `RoomManager` trait 定义多房间接口
  - [ ] MVP 实现 `SingleRoomManager`
  - [ ] 代码中明确标注"预留扩展点"，注释说明如何实现 `MultiRoomManager`

#### R-021：gRPC 服务端
- **描述**：tonic 框架，Protobuf 二进制
- **验收标准**：
  - [ ] `.proto` 定义 `BarrageEvent` 服务（双向流）
  - [ ] tonic 框架实现
  - [ ] 与 WebSocket 共享同一份业务事件 schema
  - [ ] 端口可配置（默认 50051）

#### R-022：客户端接入示例
- **描述**：Python SDK 风格
- **验收标准**：
  - [ ] `examples/python_client.py`：wss 连接 + JSON 解析
  - [ ] `examples/grpc_client.py`：gRPC 调用
  - [ ] 示例代码 ≤ 50 行

#### R-024：decoder 故障检测 + session 重连
- **描述**：参考原项目 commit `7a83d7b`
- **验收标准**：
  - [ ] 连续 5 次 protobuf 解析失败 → 触发 session 重连（不杀进程）
  - [ ] 重连后错误计数清零
  - [ ] 日志清晰标识 "session fault" 与 "recovered"

#### R-027：结构化日志
- **描述**：`tracing` + JSON 格式
- **验收标准**：
  - [ ] 日志字段：timestamp、level、target、message、trace_id
  - [ ] 可通过 `RUST_LOG` 环境变量控制级别
  - [ ] 日志同时输出到 stderr 和文件（可选）

#### R-032：API 文档
- **描述**：WS + gRPC 接口文档
- **验收标准**：
  - [ ] WS 消息 schema 文档（含 8 种事件类型示例）
  - [ ] gRPC service 定义（`.proto` 自动生成）
  - [ ] 下游消费者接入指南

#### R-033：word-guess 接入示例
- **描述**：实战演示接入
- **验收标准**：
  - [ ] 在 `word-guess` 项目中演示如何接入（commit 或 PR）
  - [ ] 替换原 `WssBarrageServer.exe` 依赖
  - [ ] 验证 word-guess 弹幕功能正常

#### R-024：decoder 故障检测 + session 重连（重复占位，跳过）

> 注意：R-024 已列在上方，此处为冗余占位占位行 — 实际 Phase 2 已合并。

### 📌 P2：可选增强（5 项）

#### R-011：collector 子命令骨架
- **描述**：采集/消费分离架构
- **验收标准**：
  - [ ] `eleven-barrage-grab collector` 子命令
  - [ ] 接受 service 推送的 wss 连接材料
  - [ ] 单元测试覆盖接口契约

#### R-012：wss URL + headers 提取
- **描述**：采集模式具体实现
- **验收标准**：
  - [ ] 通过 CDP（Chrome DevTools Protocol）或浏览器扩展注入方式获取签名后 wss URL
  - [ ] 跨平台实现（Windows + Linux + macOS）
  - [ ] 与 service 的 IPC 协议明确

#### R-028：单元测试覆盖
- **描述**：protobuf 解码 + 路由 + 事件过滤
- **验收标准**：
  - [ ] 覆盖率 > 80% on `core` crate
  - [ ] 每个测试 fixture 来自真实抖音协议 dump
  - [ ] CI 自动运行 `cargo test`

#### R-029：性能基准测试
- **描述**：mock 高频弹幕
- **验收标准**：
  - [ ] `criterion` 基准测试覆盖解码路径
  - [ ] mock 1000 msg/s 负载下 p99 延迟 < 50ms
  - [ ] 报告写入 `devflow/custom-barrage/perf-report.md`

#### R-030：长跑稳定性测试
- **描述**：24h+ 真实房间验证
- **验收标准**：
  - [ ] Phase 5 L3 真实房间验收
  - [ ] 监控内存增长（应 < 100MB 单房间长跑）
  - [ ] 监控文件句柄 / TCP 连接（不泄漏）

## 4. 依赖关系（关键路径）

```
R-001 → R-002, R-003, R-014
R-006 → R-007 → R-008 → R-009 → R-016 → R-020 → R-021 → R-022 → R-033
R-014 → R-023, R-025, R-026
R-007, R-008 → R-024
R-008 → R-017
R-015, R-019（架构预留）
R-013（MITM 兜底，独立分支）
R-028, R-029, R-030（验证类，依赖核心功能）
```

## 5. 优先级分配

- **P0（15 项）**：MVP 必须完成，否则服务无法上线
- **P1（13 项）**：MVP 完成后立即补齐（稳定性、扩展性、实战验证）
- **P2（5 项）**：可选增强（MVP 后视情况）

## 6. 已解决的 Open Questions（来自 Phase 1）

- **OQ-1**：抖音 Web wss 签名机制 — 采集/消费分离架构
- **OQ-2**：登录态 Cookie — 可选配置
- **OQ-3**：WS 与 gRPC 双通道 — 共享 BarrageEvent schema，编码不同

详见 `state.json` 的 `open_questions` 字段。

## 7. 参考资料

- 原项目源码：`../DouyinBarrageGrab/`（重点：`BarrageGrab/Modles/ProtoEntity/`、`BarrageGrab/Utility/DyApiHelper.cs`、`BarrageGrab/Server/WssBarrageGrab.cs`、`BarrageGrab/Proxy/TitaniumProxy.cs`）
- 修复 commit 历史：
  - `2af80cf` fix(proxy): WebSocket decoder 异常不再触发进程重启
  - `85d9514` fix(proxy): 消除可避免的进程崩溃源 + 5s 心跳保底
  - `c4c1eb2` fix(proxy): 关闭 Titanium 连接池以消除 NetworkStream 竞态
  - `1ca6107` refactor(program): Watchdog后台线程 + 异常防御体系
  - `7a83d7b` feat(proxy): Decoder故障检测与自动恢复