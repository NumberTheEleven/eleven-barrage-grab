# eleven-barrage-grab

> 高性能、可复用的抖音直播弹幕服务（Rust 全栈）

## 背景

本项目基于 [`DouyinBarrageGrab`](../DouyinBarrageGrab)（开源项目，已深度修改）的痛点自研：

| 原项目痛点 | 新项目方案 |
|-----------|-----------|
| .NET Framework 4.6.2 性能受限 | Rust 1.74+ async 全栈，tokio runtime |
| 仅 Windows（系统代理抓包） | Windows + Linux 跨平台，主动连接 wss |
| 单体 WSS 服务，难扩展 | 5 个 crate workspace，模块化 |
| 稳定性靠手动修复 | Watchdog + 心跳保底 + decoder 自愈 + 指数退避 |

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│  eleven-barrage-grab daemon (单二进制)                       │
├─────────────────────────────────────────────────────────────┤
│  cli          service        core         proto    collector │
│  (入口)     (主进程)        (解码)        (类型)    (占位)     │
├─────────────────────────────────────────────────────────────┤
│  RoomInfoAPI → WssConnectionManager → BarrageEvent →         │
│                  ↓                                          │
│             WS Server (8888/JSON)                            │
│             gRPC Server (50051/Protobuf)                     │
│             Prometheus (9090)                               │
└─────────────────────────────────────────────────────────────┘
```

### Crate 结构

- `crates/proto` — Protobuf schema（从原项目 `BarrageGrab/proto/*.proto` 迁移）
- `crates/core` — 解码 / 分发 / 过滤 / 去重 / 故障检测 / 重试策略
- `crates/service` — 主服务（wss 连接 + WS/gRPC 输出 + Watchdog + Metrics）
- `crates/collector` — 采集器子命令（占位，OQ-1 spike 后实现）
- `crates/cli` — `ebg` 短命令入口

## 快速开始

### 前置要求

- Rust 1.74+
- （Windows）WebView2 Runtime（可选，仅 UI 模式需要）

### 构建

```bash
# 克隆仓库
git clone <repo-url>
cd eleven-barrage-grab

# Release 构建
cargo build --release

# 跨平台构建（Linux 二进制）
cargo build --release --target x86_64-unknown-linux-gnu

# 跨平台构建（Windows 二进制）
cargo build --release --target x86_64-pc-windows-msvc
```

### 运行

```bash
# 1. 准备配置文件
cp config.example.toml config.toml
# 编辑 config.toml，设置 service.room_id

# 2. 启动服务
./target/release/eleven-barrage-grab start --config config.toml

# 或使用短命令
./target/release/ebg start --config config.toml

# 或环境变量方式
ELEVEN_BARRAGE_ROOM_ID=741891423654 ./target/release/eleven-barrage-grab start
```

### CLI 参数

```text
eleven-barrage-grab [OPTIONS] [COMMAND]

Commands:
  start        启动服务（默认）
  show-config  显示当前配置
  validate     验证配置合法性

Options:
  -c, --config <PATH>     配置文件路径
      --room-id <ID>      抖音直播间 web_room_id
      --wss-url <URL>     WSS URL（覆盖配置）
      --cookie <COOKIE>   抖音登录态 Cookie（覆盖配置）
  -h, --help              打印帮助
  -V, --version           打印版本
```

### 接入示例（Python）

```bash
pip install websockets
python examples/python_client.py ws://127.0.0.1:8888
```

输出示例：
```
connecting to ws://127.0.0.1:8888 ...
connected. waiting for barrage events (Ctrl+C to exit)...
[弹幕] 用户A: 主播好棒
[礼物] 用户B 送出 玫瑰 x10
[点赞] 用户C 点赞 +1 (累计 1234)
```

## 监控

Prometheus metrics 暴露在 `http://0.0.0.0:9090/metrics`：

- `barrage_events_total{event_type}` — 事件总数（按类型）
- `barrage_processing_duration_seconds` — 处理延迟直方图
- `wss_connection_state{room_id}` — WSS 连接状态
- `decode_errors_total{error_type}` — 解码错误总数
- `heartbeat_success_total` — 心跳成功数
- `reconnect_total{reason}` — 重连次数

## 设计参考

### 与原项目对照

| 原项目（C# / .NET Framework） | 新项目（Rust） |
|---------------------------|----------------|
| Titanium.Web.Proxy | tokio-tungstenite (主动连接) |
| protobuf-net | prost |
| Fleck WebSocket | tokio-tungstenite |
| Newtonsoft.Json | serde_json |
| NLog | tracing |
| System.Timers | tokio::time::interval |

### 借鉴的稳定性改进

| Commit | 借鉴内容 | 新项目对应 |
|--------|---------|-----------|
| `2af80cf` decoder 异常不再触发进程重启 | R-024 decoder 自愈 + session 重连 | `crates/core/src/session.rs` |
| `85d9514` 5s 心跳保底 | R-025 5s 心跳 | `crates/service/src/wss.rs:spawn_heartbeat` |
| `c4c1eb2` 关闭 Titanium 连接池 | 不适用（无 MITM） | — |
| `1ca6107` Watchdog 后台线程 | R-023 Watchdog | `crates/service/src/watchdog.rs` |
| `7a83d7b` Decoder 故障检测 + 自动恢复 | R-024 | `crates/core/src/session.rs` |

## 文档

- 需求：[`devflow/custom-barrage/requirements.md`](devflow/custom-barrage/requirements.md)
- 设计：[`devflow/custom-barrage/design.md`](devflow/custom-barrage/design.md)
- 测试用例：[`devflow/custom-barrage/test-cases.md`](devflow/custom-barrage/test-cases.md)
- 任务清单：[`devflow/custom-barrage/tasks.md`](devflow/custom-barrage/tasks.md)
- API 文档（待补充）：`docs/api-ws.md` / `docs/api-grpc.md`

## 路线图

### MVP（已完成）

- [x] 单二进制 daemon
- [x] Chat / Gift / Like 三种事件
- [x] WS 服务端（JSON）
- [x] gRPC 服务端（stub）
- [x] Watchdog + 5s 心跳 + 指数退避
- [x] Decoder 自愈 + session 重连
- [x] Prometheus metrics + tracing

### 后续迭代

- [ ] collector 子命令（CDP 集成，OQ-1）
- [ ] MITM 兜底模式（R-013）
- [ ] 完整 gRPC service（tonic 双向流）
- [ ] 多房间并行（`MultiRoomManager`，R-019）
- [ ] 性能基准（criterion，1000 msg/s 负载）
- [ ] word-guess 接入演示（R-033）

## 贡献

欢迎 PR 和 issue。当前阶段以核心路径稳定性为主。

## License

MIT OR Apache-2.0