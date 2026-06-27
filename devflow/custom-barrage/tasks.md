# Tasks — custom-barrage

> Phase 4 产出。任务拆解为 T-xxx，标注依赖、复杂度、涉及文件。

## 1. 实施范围（MVP + 关键 P1）

### 📌 本期实施（10 个 T-xxx）

| ID | 任务 | 关联 R-xxx | 依赖 | 复杂度 | 涉及文件 |
|----|------|----------|------|--------|---------|
| **T-001** | Workspace + 各 crate skeleton | R-001 | - | 中 | `Cargo.toml` (workspace root), `crates/*/Cargo.toml` |
| **T-002** | protobuf schema 迁移 + prost-build | R-006 | T-001 | 中 | `crates/proto/proto/*.proto`, `crates/proto/build.rs`, `crates/proto/src/lib.rs` |
| **T-003** | core crate：Decoder + Dispatcher + Filter + Dedup + ResiliencePipeline | R-007, R-008, R-009, R-016, R-017, R-004, R-005 | T-001, T-002 | 高 | `crates/core/src/*.rs`, `crates/core/tests/*.rs` |
| **T-004** | service crate：主入口 + WssConnectionManager + RoomInfoAPI | R-014, R-015, R-010, R-024 | T-003 | 高 | `crates/service/src/main.rs`, `crates/service/src/wss.rs`, `crates/service/src/api.rs`, `crates/service/src/session.rs` |
| **T-005** | WS 服务端实现（JSON）+ 客户端示例 | R-020, R-022 | T-004 | 中 | `crates/service/src/ws_server.rs`, `examples/python_client.py` |
| **T-006** | metrics + tracing + 配置加载 | R-026, R-027, R-003 | T-004 | 中 | `crates/service/src/config.rs`, `crates/service/src/metrics.rs`, `crates/service/src/logging.rs` |
| **T-007** | Watchdog + 心跳保底 | R-023, R-025 | T-004 | 中 | `crates/service/src/watchdog.rs`, T-004 中 wss.rs 集成心跳 |
| **T-008** | gRPC 服务端（stub 形式，完整接口 + 简单实现） | R-021 | T-004 | 中 | `crates/service/src/grpc_server.rs`, `crates/service/proto/barrage.proto` |
| **T-009** | README + 配置模板 + .gitignore 完善 | R-031 | T-001~T-008 | 低 | `README.md`, `config.example.toml`, `.gitignore` |
| **T-010** | L1 + L2 自动化测试（cargo test 通过） | R-028（部分） | T-001~T-008 | 中 | 各 crate `tests/*.rs`, `tests/fixtures/*.bin`（mock） |

### ⏸ 后续 Phase 5/6 再补（架构预留 / Phase 5 后置）

| ID | 任务 | 关联 R-xxx | 备注 |
|----|------|----------|------|
| T-011 | collector 子命令实现（CDP 集成） | R-011, R-012, OQ-1 | 需技术 spike，Phase 5 后 |
| T-012 | MITM 兜底模式 | R-013 | 复杂度高，Phase 5 后 |
| T-013 | 性能基准测试（criterion + mock 1000 msg/s） | R-029 | Phase 5 验证后 |
| T-014 | word-guess 接入演示 | R-033 | 需在 word-guess 仓库操作 |
| T-015 | 多房间实现（MultiRoomManager） | R-019 | 架构预留接口在 T-003 已留 |
| T-016 | API 完整文档（WS + gRPC schema 详解） | R-032 | Phase 5 后补 |

## 2. 关键依赖图

```
T-001 (workspace skeleton)
  │
  ├── T-002 (protobuf schema)
  │      │
  │      └── T-003 (core: decoder + dispatcher + filter + dedup + resilience)
  │             │
  │             └── T-004 (service: main + wss + api + session)
  │                    ├── T-005 (WS server + Python client)
  │                    ├── T-006 (metrics + tracing + config)
  │                    ├── T-007 (watchdog + heartbeat)
  │                    └── T-008 (gRPC server stub)
  │
  └── T-009 (README + config template)
  
T-010 (L1 + L2 tests) ── depends on T-001~T-008
```

## 3. TDD 循环规范

每个 T-xxx 内部：
1. **Red**：写失败的测试（先 mock fixture 或接口）
2. **Green**：实现最小代码使测试通过
3. **Refactor**：清理代码
4. **Commit**：`feat(core):` / `feat(service):` / `test:` 等前缀
5. **Review**：自查代码质量

## 4. 验收标准

Phase 4 完成时必须满足：
- ✅ `cargo build --release` 在当前平台通过
- ✅ `cargo test --release` 全部通过
- ✅ `cargo clippy --all-targets -- -D warnings` 通过
- ✅ `cargo fmt --check` 通过
- ✅ Python 客户端示例可连接到本地服务（mock 模式）

## 5. commit 规范

```bash
# 允许的 commit 类型
feat(scope):    新功能
fix(scope):     bug 修复
refactor(scope): 重构（无功能变化）
test(scope):    测试代码
docs(scope):    文档
chore(scope):   构建/工具链/CI
```

scope 取值：`proto`, `core`, `service`, `cli`, `examples`, `docs`, `devflow`

## 6. Phase 4 完成 checkpoint

所有 T-001 ~ T-010 完成后，由用户确认是否进入 Phase 5。