# Test Cases — custom-barrage

> Phase 3 产出。每个 R-xxx 至少 1 个 TC-xxx。按 L1/L2/L3 分层。

## 1. 层级说明

| 层级 | 自动化 | 工具 | 触发时机 |
|------|--------|------|---------|
| **L1 烟雾** | ✅ | `cargo test --release` / `cargo build` | 每个 commit + Phase 5 入口 |
| **L2 交互** | ✅ | `tokio::test` + mock fixture | 每个 commit + Phase 5 |
| **L3 手工** | ❌ | 真实浏览器 + 真实房间 | Phase 5 用户手工验收 |

## 2. TC 清单

### 2.1 L1 烟雾（编译与启动）

#### TC-L1-001：cargo build 通过（Linux）
- **关联 R-xxx**：R-001, R-002
- **步骤**：
  1. 在 Linux 环境执行 `cargo build --release --target x86_64-unknown-linux-gnu`
  2. 检查 exit code = 0
  3. 检查产出 `target/x86_64-unknown-linux-gnu/release/eleven-barrage-grab` 存在
  4. 检查文件大小 < 30MB
- **预期**：exit code 0，文件存在且符合大小限制

#### TC-L1-002：cargo build 通过（Windows）
- **关联 R-xxx**：R-001, R-002
- **步骤**：
  1. 在 Windows 环境执行 `cargo build --release --target x86_64-pc-windows-msvc`
  2. 检查 exit code = 0
  3. 检查产出二进制存在
- **预期**：exit code 0

#### TC-L1-003：cargo test 通过（单元测试）
- **关联 R-xxx**：R-001, R-007, R-008, R-009, R-016, R-017, R-024, R-028
- **步骤**：
  1. 执行 `cargo test --release`
  2. 检查所有测试通过
  3. 检查覆盖率报告（`cargo tarpaulin` 或 `cargo-llvm-cov`）core crate > 80%
- **预期**：所有测试通过，覆盖率达标

#### TC-L1-004：服务启动（无崩溃）
- **关联 R-xxx**：R-014, R-023
- **步骤**：
  1. 准备测试用 config.toml
  2. 执行 `eleven-barrage-grab service start`
  3. 等待 5 秒
  4. 检查进程仍存在（PID 未退出）
  5. 执行 `eleven-barrage-grab service stop`
- **预期**：进程稳定运行 5 秒，优雅退出

#### TC-L1-005：clippy / fmt 通过
- **关联 R-xxx**：R-001
- **步骤**：
  1. 执行 `cargo clippy --all-targets --all-features -- -D warnings`
  2. 执行 `cargo fmt --check`
- **预期**：无 warning，格式符合规范

### 2.2 L2 交互（单元/集成测试）

#### TC-L2-001：protobuf schema 编译
- **关联 R-xxx**：R-006
- **步骤**：
  1. 执行 `cargo build -p proto`
  2. 检查 `target/debug/build/proto-*/out/*.rs` 生成
- **预期**：生成的 Rust 类型与原项目 .proto 一致

#### TC-L2-002：gzip 解压正确
- **关联 R-xxx**：R-007
- **步骤**：
  1. 准备 fixture：`tests/fixtures/wss_compressed.bin`（真实抖音 wss gzip 帧）
  2. 调用 `Decoder::decode_wss_frame(bytes)`
  3. 验证返回 `WssResponse` 不为空
- **预期**：解压成功，payload 字段可读

#### TC-L2-003：无效 wire_type 前置校验
- **关联 R-xxx**：R-007
- **步骤**：
  1. 准备 fixture：wire_type = 6 的数据
  2. 调用 `Decoder::decode_wss_frame(bytes)`
  3. 验证返回 `Err(DecodeError::InvalidWireType(6))`
  4. 验证**未**增加 decoder 错误计数（避免误判）
- **预期**：返回错误，不影响 session 状态

#### TC-L2-004：消息分发按 Method 路由
- **关联 R-xxx**：R-008, R-009
- **步骤**：
  1. 准备 fixture：包含 8 种不同 msg.Method 的 Response
  2. 调用 `Dispatcher::dispatch(response)`
  3. 验证返回 `Vec<BarrageEvent>` 含 8 种 event_type
- **预期**：8 种消息类型都被正确识别和分发

#### TC-L2-005：MVP 事件过滤生效
- **关联 R-xxx**：R-016
- **步骤**：
  1. 配置 `push_event_types = ["ChatMessage", "GiftMessage", "LikeMessage"]`
  2. 输入 8 种不同事件
  3. 验证输出仅含 Chat/Gift/Like 三种
- **预期**：过滤逻辑正确，5 种被过滤

#### TC-L2-006：msgId 去重
- **关联 R-xxx**：R-017
- **步骤**：
  1. 准备 100 个相同 msgId 的消息
  2. 调用 `Dedup::process(events)`
  3. 验证输出仅含 1 条
- **预期**：去重生效

#### TC-L2-007：环形缓冲容量限制
- **关联 R-xxx**：R-017
- **步骤**：
  1. 输入 1000 个不同 msgId
  2. 验证 Dedup 内部缓冲不超过 300（每类型）
- **预期**：内存上限可控

#### TC-L2-008：session 故障检测
- **关联 R-xxx**：R-024
- **步骤**：
  1. mock decoder 连续 5 次失败
  2. 验证触发 `SessionFaultEvent`
  3. 验证错误计数清零
- **预期**：故障被检测并触发重连信号

#### TC-L2-009：房间元数据 API 调用
- **关联 R-xxx**：R-010
- **步骤**：
  1. 调用 `RoomInfoAPI::get(web_room_id, None)`
  2. mock HTTP 服务器返回标准 room info JSON
  3. 验证 query 参数符合 `aid=6383, device_platform=web, web_rid=...`
  4. 验证 Headers 包含 UA 和 Referer
- **预期**：HTTP 调用参数正确

#### TC-L2-010：WS 客户端连接 + 接收 JSON
- **关联 R-xxx**：R-020
- **步骤**：
  1. 启动服务（mock wss 上游）
  2. 使用 `tokio-tungstenite` 客户端连接 `ws://127.0.0.1:8888`
  3. 触发 mock 上游发送 BarrageEvent
  4. 验证客户端收到 JSON 格式消息
- **预期**：JSON 格式正确，字段完整

#### TC-L2-011：gRPC 客户端连接 + 接收 Protobuf
- **关联 R-xxx**：R-021
- **步骤**：
  1. 启动服务
  2. 使用 `tonic` 客户端调用 `Subscribe`
  3. 触发 mock 上游发送事件
  4. 验证客户端收到 Protobuf 编码的 BarrageEvent
- **预期**：Protobuf 编码正确

#### TC-L2-012：指数退避重连
- **关联 R-xxx**：R-015
- **步骤**：
  1. mock wss 上游主动断开
  2. 验证重连间隔 1s → 2s → 4s → 8s → 16s → 32s → 60s（封顶）
- **预期**：退避逻辑正确

#### TC-L2-013：5s 心跳保底
- **关联 R-xxx**：R-025
- **步骤**：
  1. mock 上游空闲（不发消息）
  2. 等待 5 秒
  3. 验证 wss 客户端发送了 ping 帧
- **预期**：心跳按 5s 间隔发送

#### TC-L2-014：metrics 端点暴露
- **关联 R-xxx**：R-026
- **步骤**：
  1. 启动服务
  2. `curl http://127.0.0.1:9090/metrics`
  3. 验证返回 Prometheus 格式
  4. 验证含 `barrage_events_total`, `wss_connection_state`, `decode_errors_total`
- **预期**：metrics 正确暴露

#### TC-L2-015：优雅关闭
- **关联 R-xxx**：R-014, R-023
- **步骤**：
  1. 启动服务
  2. 发送 SIGTERM
  3. 验证进程在 5 秒内退出，exit code 0
  4. 验证 watchdog task 被清理
- **预期**：无 panic，无资源泄漏

#### TC-L2-016：配置加载三层覆盖
- **关联 R-xxx**：R-003
- **步骤**：
  1. 准备 config.toml: `room_id = "default"`
  2. 设置环境变量 `ELEVEN_BARRAGE_ROOM_ID=env_value`
  3. CLI flag `--room-id cli_value`
  4. 验证最终生效值为 `cli_value` > `env_value` > `default`
- **预期**：三层覆盖优先级正确

#### TC-L2-017：错误处理 + tracing span
- **关联 R-xxx**：R-004
- **步骤**：
  1. 触发业务错误（mock invalid input）
  2. 验证错误被 `thiserror` 包装
  3. 验证 tracing log 含 `error` 级别记录 + 相关 span 字段
- **预期**：错误可追溯

#### TC-L2-018：结构化日志输出
- **关联 R-xxx**：R-027
- **步骤**：
  1. 设置 `RUST_LOG=info`
  2. 启动服务并触发一些日志
  3. 验证日志输出为 JSON 格式
  4. 验证字段：timestamp、level、target、message
- **预期**：JSON 日志符合规范

### 2.3 L3 手工（真实房间验收）

#### TC-L3-001：真实房间拉取弹幕
- **关联 R-xxx**：R-014, R-015, R-020, R-021
- **步骤**：
  1. 打开抖音网页，挑选一个活跃直播间（web_room_id 已知）
  2. 启动服务：`eleven-barrage-grab service start --room-id {id}`
  3. 在网页中发一条弹幕"测试 custom-barrage"
  4. 使用 Python 客户端连接 `ws://127.0.0.1:8888`
  5. 验证客户端在 2 秒内收到包含"测试 custom-barrage"的 BarrageEvent
- **预期**：真实弹幕事件被正确拉取并推送
- **验收证据**：Python 客户端输出截图 + 服务端日志

#### TC-L3-002：真实房间拉取礼物
- **关联 R-xxx**：R-014, R-015, R-016, R-020
- **步骤**：
  1. 启动服务并连接到活跃直播间
  2. 在网页中送一个礼物
  3. 验证 WS 客户端收到 GiftMessage 事件
  4. 验证字段完整：用户、礼物 ID、礼物名称、数量
- **预期**：礼物事件正确推送
- **验收证据**：客户端日志截图

#### TC-L3-003：真实房间拉取点赞
- **关联 R-xxx**：R-014, R-015, R-016, R-020
- **步骤**：
  1. 启动服务并连接到活跃直播间
  2. 在网页中点赞
  3. 验证 WS 客户端收到 LikeMessage 事件
- **预期**：点赞事件正确推送
- **验收证据**：客户端日志截图

#### TC-L3-004：gRPC 通道推送真实事件
- **关联 R-xxx**：R-021
- **步骤**：
  1. 启动服务并连接到活跃直播间
  2. 使用 Python gRPC 客户端订阅
  3. 在网页中发弹幕
  4. 验证 gRPC 客户端收到 Protobuf 编码的 BarrageEvent
- **预期**：gRPC 通道工作正常

#### TC-L3-005：长跑稳定性（24h+）
- **关联 R-xxx**：R-014, R-015, R-023, R-025, R-030
- **步骤**：
  1. 启动服务并连接到活跃直播间
  2. 持续运行 24 小时
  3. 监控：
     - 进程内存（应 < 100MB）
     - 文件句柄（不增长）
     - TCP 连接（保持稳定）
     - 弹幕丢失率（应 < 0.1%）
  4. 24h 后查看日志，是否有非预期重启
- **预期**：24h 长跑无崩溃，资源不泄漏
- **验收证据**：`/metrics` 24h 截图 + 服务日志 + 客户端事件计数

#### TC-L3-006：连接断开自动重连
- **关联 R-xxx**：R-015, R-024
- **步骤**：
  1. 启动服务并连接到活跃直播间
  2. 验证 WS 客户端持续接收事件
  3. 手动关闭浏览器 / 触发网络中断（拔网线 10s 后恢复）
  4. 验证服务自动重连（< 60s 内）
  5. 验证 WS 客户端继续接收新事件
- **预期**：自动重连生效，客户端无感知
- **验收证据**：服务日志（reconnect）+ 客户端事件流连续性

#### TC-L3-007：跨平台编译验证
- **关联 R-xxx**：R-002
- **步骤**：
  1. 在 Linux 机器上 `cargo build --release --target x86_64-unknown-linux-gnu`
  2. 复制二进制到另一台 Linux 机器（无 Rust 工具链）
  3. 运行 `./eleven-barrage-grab service start`
  4. 验证启动成功并能连接到抖音 wss
- **预期**：单二进制可直接运行
- **验收证据**：启动截图 + 一条弹幕推送截图

#### TC-L3-008：word-guess 接入演示
- **关联 R-xxx**：R-022, R-033
- **步骤**：
  1. 在 `D:/codingProjects/word-guess` 仓库中：
     - 移除 `DouyinBarrageGrab/WssBarrageServer.exe` 依赖
     - 添加 `eleven-barrage-grab` Python SDK 调用
  2. 启动 eleven-barrage-grab 服务
  3. 启动 word-guess
  4. 在直播间发弹幕"hello"
  5. 验证 word-guess 主界面正确显示该弹幕
- **预期**：word-guess 无缝切换到新服务
- **验收证据**：word-guess 界面截图

## 3. TC 路由统计

| 层级 | 数量 | 说明 |
|------|------|------|
| L1 烟雾 | 5 | 必须全部通过才能进入 Phase 5 |
| L2 交互 | 18 | 单元/集成测试，CI 自动运行 |
| L3 手工 | 8 | 用户在真实环境操作 |
| **总计** | **31** | - |

## 4. 优先级映射

| 阶段 | 必须通过的 TC |
|------|--------------|
| 进入 Phase 4 | L1-001 ~ L1-005（编译 + clippy + fmt 通过） |
| Phase 4 收尾 | L1-001 ~ L1-004 + L2-001 ~ L2-018 |
| Phase 5 入口 | L1 全部 + L2 全部 + L3 准备就绪（真实房间待命） |
| Phase 5 验收 | L3-001 ~ L3-008 全部由用户手工通过 |