# dynamic-room-subscription — Requirements

基于 Phase 1 需求澄清产出。

## 目标

让 `eleven-barrage-grab` 服务支持「动态房间订阅」：用户给出一个抖音直播间 URL，服务内部完成签名和采集，对外暴露一个 WebSocket 地址供客户端订阅该房间弹幕。

## 包含项 / 排除项 / 成功标准 / 约束

### 包含

- `POST /v1/rooms { url }` 同步等待签名成功，返回 `{ room_id, ws_url, status }`
- 同 URL 幂等返回相同 room_id，多个 WS 客户端共享同一采集
- `DELETE /v1/rooms/{room_id}` 手动销毁
- `GET /v1/rooms` 列出活跃房间
- `WebSocket /rooms/{room_id}` 路径路由
- 替换固定 room_id 启动模式
- 复用 BrowserPool / WSS / HTTP fetch collector
- 统一错误响应格式

### 排除

- 房间列表持久化
- 房间自动空闲清理
- 房间权限/认证
- UI
- WS 房间状态事件通知

### 成功标准

- `cargo test` 全部通过（保留 62/62 基线 + 新增测试）
- L1 烟雾 + L2 交互验证全部通过
- 完整 DevFlow 六阶段用户验收通过

### 约束

- 不破坏现有 REST `/v1/sign`、gRPC 接口
- 沿用 BrowserPool 并发限制
- Rust 2021 edition、rust-version 1.74

## R-xxx 清单

### R-001 POST /v1/rooms 创建/复用房间  (P0)
- [ ] 接受 JSON `{ "url": "https://live.douyin.com/xxx" }`
- [ ] 同步等待签名成功（内部走 BrowserPool）
- [ ] 成功返回 201 + `{ room_id, ws_url, status: "connected" }`
- [ ] 失败返回结构化错误码（INVALID_URL、ROOM_NOT_FOUND、COOKIE_EXPIRED、NETWORK_TRANSIENT）

### R-002 同 URL 幂等返回相同 room_id  (P0)
- [ ] RoomManager 用 `web_rid` 作为 key 去重
- [ ] 已存在则跳过签名步骤，直接返回现有 room_id
- [ ] 多个 WS 客户端共享同一份采集

### R-003 DELETE /v1/rooms/{room_id} 销毁房间  (P0)
- [ ] 停止采集（WSS 连接 / FetchConsumer 任务）
- [ ] 断开该房间所有 WS 客户端
- [ ] 成功 204，房间不存在 404

### R-004 GET /v1/rooms 列出活跃房间  (P1)
- [ ] 返回 `{ rooms: [{ room_id, url, status, client_count, created_at_unix }] }`

### R-005 WsServer 路径路由 /rooms/{id}  (P0)
- [ ] accept 时解析 URI 路径
- [ ] 匹配 `/rooms/<id>` 注册到该房间 dispatcher
- [ ] 不匹配关闭连接

### R-006 DynamicRoomManager 替换 SingleRoomManager  (P0)
- [ ] 内部 `Arc<Mutex<HashMap<String, RoomHandle>>>`
- [ ] RoomHandle 包含 web_rid、url、采集 task 句柄、client_count (Arc<AtomicUsize>)、shutdown_tx、created_at

### R-007 服务启动后不启动默认房间  (P0)
- [ ] 删除 run.rs 中 SingleRoomManager 启动逻辑
- [ ] config.toml 中 service.room_id 保留但仅作文档
- [ ] 无 POST 请求时 WS server 空载

### R-008 复用 BrowserPool + WSS + FetchConsumer  (P0)
- [ ] 复用 `pool::BrowserPool.sign(web_rid)` 拿 SignedMaterial
- [ ] 根据 kind 走 WSS push 或 HTTP fetch fallback
- [ ] 抽取 `spawn_collector(material, filter, dispatcher_tx) -> JoinHandle` 作为公共函数

### R-009 采集任务生命周期与资源清理  (P0)
- [ ] DELETE 时关闭采集任务
- [ ] WS 客户端断开不影响采集
- [ ] 最后一个客户端断开不自动停止采集

### R-010 错误响应统一格式  (P0)
- [ ] 复用 ApiError 风格的 `{ error: { code, message, retryable } }`
- [ ] HTTP status 跟随错误类型

### R-011 向后兼容保留的接口  (P1)
- [ ] `POST /v1/sign` 仍可用
- [ ] gRPC `SignedBarrageService` 仍可用
- [ ] gRPC `BarrageService.subscribe` 行为调整：保留并接收 event source

### R-012 单元/集成测试覆盖  (P0)
- [ ] RoomManager create/dedup/destroy 测试
- [ ] WsServer 路径路由测试
- [ ] REST handler POST/DELETE/GET 测试

## 依赖关系

- R-006 是核心，被 R-001/002/003/004/008 依赖
- R-007 依赖 R-006
- R-005 依赖 R-006
- R-008 依赖现有 pool / WssConnectionManager / fetch_consumer

## 优先级

- P0: R-001、R-002、R-003、R-005、R-006、R-007、R-008、R-009、R-010、R-012
- P1: R-004、R-011
