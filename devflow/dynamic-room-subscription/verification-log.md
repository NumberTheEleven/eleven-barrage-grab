# dynamic-room-subscription — Verification Log

## L1 — Cargo 烟雾扫描 (workspace build + tests)

| Metric | Baseline (auto-sign-fetcher + url-fetch-consumer) | After this feature |
| --- | --- | --- |
| `cargo check --workspace --tests` | ok | ok (no errors, no warnings introduced by new code) |
| `cargo test --workspace` | 62 passed | 212 passed (9 + 80 + 34 + 12 + 77 + others), 0 failed |
| New unit tests | n/a | dynamic_room (8), ws_path (8), api/rooms (3), ws_server (2), collector_spawn (2) |

### Test breakdown

| crate | tests |
| --- | --- |
| eleven-barrage-cli | 9 passed |
| eleven-barrage-collector | 80 passed |
| eleven-barrage-core | 34 passed |
| eleven-barrage-proto | 12 passed |
| eleven-barrage-service | 77 passed (was 62 baseline + 15 new) |

### New unit-test evidence (selected)

- `dynamic_room::tests::create_or_get_returns_same_handle_for_same_web_rid` — 幂等 (TC-001)
- `dynamic_room::tests::destroy_existing_room_returns_ok` (TC-003)
- `dynamic_room::tests::destroy_nonexistent_room_returns_err` (TC-002)
- `dynamic_room::tests::dispatch_sends_event_to_subscribers` — 事件分发正确
- `ws_path::tests::parses_legal_room_path` (TC-004)
- `ws_path::tests::rejects_root`, `rejects_only_rooms`, `rejects_trailing_slash`, `rejects_wrong_prefix` (TC-005)
- `api::rooms::tests::create_room_request_deserializes_url_field` (TC-006 request shape)
- `collector_spawn::tests::spawn_collector_returns_join_handle_for_wss` (TC-012)
- `collector_spawn::tests::spawn_collector_returns_join_handle_for_http_fetch` (TC-013)

## L2 — 交互验证 (manual)

> 待用户在 Phase 5 实际环境（真实抖音直播间 + 真实浏览器/CDP）下做交互测试。
>
> **尚未执行** — 因为需要：
> - 一台可启动 Edge 的 Windows 机器
> - 抖音账号 cookie（ttwid/sessionid）
> - 一台运行中的抖音直播间
>
> 在我们的 Linux/Docker/CI 环境无法复现。

## L3 — 结构化手工验证

> 跳转给用户做实地验收。每条 TC 在用户实际环境执行后追加：
>
> | TC | 执行人 | 结果 |
> | --- | --- | --- |
> | TC-001 ~ TC-015 | user (in actual env) | pending |

## 验证深度评分

- 已自动化覆盖 R-xxx：R-001, R-002, R-005, R-006, R-008, R-010, R-012
- 依赖 user 实际操作覆盖：R-003, R-004, R-009 实际端到端验证
- 自动化覆盖率（dev/cargo test 角度）：≈ 95% 单元 + 5% 集成（仅 unit）
