# dynamic-room-subscription — Design

## 业务流

参见 state.json 当前阶段输入的 Phase 3 blueprint 流程图（sequenceDiagram）。

## 范围

### 范围内

- `POST /v1/rooms`、`DELETE /v1/rooms/{id}`、`GET /v1/rooms`
- `DynamicRoomManager` 替换 `SingleRoomManager`
- `WsServer` 接受 `/rooms/<id>` 路径路由
- 内部走 `BrowserPool.sign()` 获取 `SignedMaterial`
- WSS push + HTTP fetch fallback
- 签名阶段错误通过 POST 响应返回

### 范围外

- 房间列表持久化
- 自动空闲清理
- 权限/认证
- UI
- 房间状态事件通知

## 技术决策

1. **RoomManager 单实例**：`Arc<DynamicRoomManager>` 注入 REST、gRPC、WsServer
2. **WsServer 路径解析**：`Uri::path()` 解析 `/rooms/<id>`；非匹配返回 400
3. **每房间独立 dispatcher**：`Arc<Mutex<HashMap<String, Vec<mpsc::Sender>>>>` + `subscribe(room_id)` push、`unsubscribe` remove
4. **collector 复用**：抽取 `spawn_collector(material, filter, dispatcher_tx) -> JoinHandle` 公共函数
5. **删除 `SingleRoomManager`**：直接删除 `room.rs`

## 新增/修改文件

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `crates/service/src/room.rs` | 删除 | 由 `dynamic_room.rs` 替换 |
| `crates/service/src/dynamic_room.rs` | 新增 | DynamicRoomManager + RoomHandle |
| `crates/service/src/api/mod.rs` | 修改 | 注册 `POST/DELETE/GET /v1/rooms` |
| `crates/service/src/api/rooms.rs` | 新增 | rooms REST handlers |
| `crates/service/src/ws_server.rs` | 修改 | 接受 `Arc<DynamicRoomManager>`；路径解析 |
| `crates/service/src/ws_path.rs` | 新增 | 路径解析工具：`/rooms/<id>` -> Some(id) |
| `crates/service/src/collector_spawn.rs` | 新增 | `spawn_collector(material, filter, dispatcher_tx)` |
| `crates/service/src/run.rs` | 修改 | 删除 SingleRoomManager 启动逻辑；注入 DynamicRoomManager |
| `crates/service/src/grpc_server.rs` | 修改 | 适配新的 room manager（保持 gRPC subscribe） |
| `crates/service/src/lib.rs` | 修改 | 更新导出 |
| `crates/cli/src/main.rs` | 修改 | 可选：CLI `grab` 复用 spawn_collector |

## 架构图

```
+---------------------------------------+
| DynamicRoomManager                    |
| HashMap<room_id, RoomHandle>          |
|   |                                   |
|   +-- spawn_collector ->              |
|       |                               |
|       v                               |
|   collector task (WSS or HTTP fetch)  |
|       | events                        |
|       v                               |
|   HashMap<room_id, dispatcher map>    |
+---------------------------------------+
            ^
            | subscribe(room_id)
            |
+---------------------------------------+
| WsServer                               |
|   accept() -> parse /rooms/<id>       |
|   -> subscribe to room dispatcher     |
+---------------------------------------+
```

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| 浏览器签名资源耗尽 | 沿用现有 BrowserPool 并发限制 |
| WS dispatcher map 锁竞争 | parking_lot::Mutex + 每房间独立 vec |
| 客户端断开时数量错乱 | Arc<AtomicUsize> client_count |
| 同一 WSS 被多房间复用 | 不允许：每房间独立 collector |
| proto schema 影响 | 检查 signed_proto 字段；如需调整则在 proto 模块 |
