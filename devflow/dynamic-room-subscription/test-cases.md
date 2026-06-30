# dynamic-room-subscription — Test Cases

| ID | 类型 | 用例 | 步骤 | 预期结果 |
|----|------|------|------|----------|
| TC-001 | 单元 | RoomManager create_or_get 幂等 | 第一次 create_or_get("A") 返回新；第二次返回相同的 | 两次返回相同 room_id |
| TC-002 | 单元 | RoomManager destroy 不存在 | destroy("nonexistent") | Err(RoomNotFound) |
| TC-003 | 单元 | RoomManager destroy 后清空 | destroy 之后 len | len==0 |
| TC-004 | 单元 | WsServer 路径解析合法路径 | URI `/rooms/abc` | Some("abc") |
| TC-005 | 单元 | WsServer 路径解析非法路径 | `/`、`/rooms`、`/rooms/` | None |
| TC-006 | 集成 | POST /v1/rooms 成功 | mock sign | 201 + body |
| TC-007 | 集成 | POST /v1/rooms INVALID_URL | url 非抖音域 | 400 + INVALID_URL |
| TC-008 | 集成 | POST /v1/rooms 幂等 | 同一 url 两次 | 相同 room_id |
| TC-009 | 集成 | DELETE 成功 | 存在房间 | 204，list 减少 |
| TC-010 | 集成 | DELETE 不存在 | 不存在 id | 404 |
| TC-011 | 集成 | GET /v1/rooms 列出 | 创建 2 个后 GET | 返回 2 条 |
| TC-012 | 单元 | spawn_collector 走 WSS 路径 | mock SignedMaterial::Wss | 启动 ws task + 推送事件 |
| TC-013 | 单元 | spawn_collector 走 HTTP fetch 路径 | mock SignedMaterial::HttpFetch | 启动 fetch consumer |
| TC-014 | 集成 | 删除 collector 资源 | destroy 后 task 退出 | JoinHandle 完成 |
| TC-015 | 单元 | error 响应格式 | 任意 ApiError | 字段齐 |
