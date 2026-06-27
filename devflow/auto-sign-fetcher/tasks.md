# Tasks — auto-sign-fetcher

> Phase 4 产出。任务拆解为 T-xxx，标注依赖、复杂度、涉及文件。

## 1. 实施范围

### 📌 本期实施（11 个 T-xxx）

| ID | 任务 | 关联 R-xxx | 依赖 | 复杂度 | 涉及文件 |
|----|------|----------|------|--------|---------|
| **T-001** | SignatureError 统一错误类型 | R-006 | - | 低 | `crates/collector/src/error.rs` |
| **T-002** | UrlParser URL 解析器 | R-001 | T-001 | 低 | `crates/collector/src/url.rs` |
| **T-003** | AuthConfig cookie 配置 | R-002 | T-001 | 低 | `crates/service/src/config.rs`, `config.example.toml` |
| **T-004** | ImFetcher im_fetch HTTP 调用 | R-004 | T-001 | 中 | `crates/collector/src/im_fetch.rs` |
| **T-005** | RoomInfoApi 错误映射 | R-003 | T-001 | 低 | `crates/service/src/api.rs` |
| **T-006** | signed.proto + ProvideSignedWss RPC 定义 | R-008 | - | 中 | `crates/proto/proto/signed.proto`, `crates/proto/build.rs` |
| **T-007** | AutoSigner 组合 RoomApi + ImFetcher | R-003, R-004 | T-002, T-004, T-005 | 中 | `crates/collector/src/signer.rs` |
| **T-008** | gRPC 服务实现 ProvideSignedWss | R-008 | T-006, T-007 | 中 | `crates/service/src/grpc_server.rs` |
| **T-009** | ebg grab CLI 子命令 | R-007 | T-008 | 中 | `crates/cli/src/main.rs` |
| **T-010** | WSS 集成（AutoSigner → WssConnectionManager） | R-005 | T-007 | 中 | `crates/service/src/run.rs`, `crates/service/src/wss.rs` |
| **T-011** | 集成测试 + E2E mock 链路 | R-010 | T-001~T-010 | 高 | `crates/collector/tests/*.rs`, `crates/service/tests/*.rs` |

## 2. 关键依赖图

```
T-001 (SignatureError) ─────────────────┐
                                       ├── T-002 (UrlParser)
                                       │       │
T-003 (AuthConfig) ────────────────────┤       │
                                       │       └── T-007 (AutoSigner)
T-004 (ImFetcher) ─────────────────────┤              │
                                       │              ├── T-008 (gRPC service)
T-005 (RoomInfoApi 错误映射) ──────────┘              │       │
                                                       │       └── T-009 (CLI)
T-006 (signed.proto) ─────────────────────────────────┘              │
                                                                       │
                                                                       ├── T-010 (WSS 集成)
                                                                       │       │
                                                                       │       └── T-011 (E2E 测试)
                                                                       │
                                                    R-009 (向后兼容) ───┘
                                                    （贯穿全程，每次 commit 验证）
```

## 3. TDD 循环规范

每个 T-xxx 内部：
1. **Red**：写失败的测试（先 mock fixture 或接口）
2. **Green**：实现最小代码使测试通过
3. **Refactor**：清理代码
4. **Commit**：`feat(collector):` / `feat(service):` / `feat(cli):` / `test:` 等前缀
5. **Review**：自查代码质量

## 4. 验收标准

Phase 4 完成时必须满足：
- ✅ `cargo build --release` 在 Windows 通过
- ✅ `cargo test --release` 全部通过（既有 62/62 + 新增 ≥15 个测试）
- ✅ `cargo clippy --all-targets --all-features -- -D warnings` 通过
- ✅ `cargo fmt --check` 通过
- ✅ R-009 向后兼容：现有 custom-barrage 62 个测试 0 regression

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

scope 取值：`proto`, `core`, `collector`, `service`, `cli`, `tests`, `devflow`

## 6. 进度追踪

| ID | 状态 | 关联 R-xxx | commit |
|----|------|----------|--------|
| T-001 | pending | R-006 | - |
| T-002 | pending | R-001 | - |
| T-003 | pending | R-002 | - |
| T-004 | pending | R-004 | - |
| T-005 | pending | R-003 | - |
| T-006 | pending | R-008 | - |
| T-007 | pending | R-003, R-004 | - |
| T-008 | pending | R-008 | - |
| T-009 | pending | R-007 | - |
| T-010 | pending | R-005 | - |
| T-011 | pending | R-010 | - |

## 7. Phase 4 完成 checkpoint

所有 T-001 ~ T-011 完成后，由用户确认是否进入 Phase 5。
