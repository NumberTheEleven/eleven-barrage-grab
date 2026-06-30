# Test Cases: url-fetch-consumer

## 覆盖映射

| TC | 对应 R-xxx | 类型 | 自动化级别 |
|---|---|---|---|
| TC-001 | R-001 | 单元测试 | L1 |
| TC-002 | R-001 | 单元测试 | L1 |
| TC-003 | R-002 | 单元测试 | L1 |
| TC-004 | R-002 | 集成测试 | L2 |
| TC-005 | R-003 | 集成测试 | L2 |
| TC-006 | R-004 | 回归检查 | L1 |
| TC-007 | R-005 | 集成测试 | L2 |
| TC-008 | R-006 | 配置校验 | L1 |
| TC-009 | R-007 | 全量回归 | L1 |
| TC-010 | R-007 | 手工验收 | L3 |

---

## TC-001 CDP response body 正确提取

**目标：** 验证 `FetchConsumer` 能从模拟的 CDP 事件中读取 fetch 响应体。

**步骤：**
1. 构造一个 `Network.responseReceived` 事件，URL 为 `https://live.douyin.com/webcast/im/fetch/?room_id=123`。
2. 注册对应的 `Network.getResponseBody` 响应，返回 base64 编码的 protobuf body。
3. 调用 `FetchConsumer::on_response_received`。

**预期结果：**
- 成功触发一次 body 提取。
- 提取出的 bytes 与原始 protobuf 一致。

---

## TC-002 非 fetch URL 被忽略

**目标：** 验证只有 `/webcast/im/fetch/` URL 会触发 body 读取。

**步骤：**
1. 构造 `Network.responseReceived` 事件，URL 为 `https://live.douyin.com/webcast/gift/list/`。
2. 调用处理函数。

**预期结果：**
- 不调用 `Network.getResponseBody`。
- 不触发任何消费事件。

---

## TC-003 HTTP fetch body 解码为弹幕事件

**目标：** 验证裸 `Response` protobuf 能被解码并分发出 `BarrageEvent`。

**步骤：**
1. 准备一段录制的 fetch protobuf payload（或构造包含 `WebcastChatMessage` 的最小 payload）。
2. 调用 `Response::decode(&payload)`。
3. 用 `Dispatcher::dispatch` 转换消息。

**预期结果：**
- 至少产出一个 `BarrageEvent`。
- 事件 `method()` 为 `WebcastChatMessage`。

---

## TC-004 `FetchConsumer` 端到端事件流

**目标：** 验证 `FetchConsumer` 在 mock CDP 环境下能持续输出事件。

**步骤：**
1. 使用 `crates/collector/src/cdp/mock.rs` 中的 `CdpTransport` mock。
2. 模拟一次导航成功 + 一次 fetch response。
3. 启动 `FetchConsumer` 并订阅其事件 channel。

**预期结果：**
- channel 在 5 秒内收到至少一个 `BarrageEvent`。
- 消费者运行期间无 panic。

---

## TC-005 页面不活跃时仍保持轮询

**目标：** 验证 `FetchConsumer` 会周期注入 JS 保持页面活跃。

**步骤：**
1. mock CDP transport，记录所有 `Runtime.evaluate` 调用。
2. 启动 `FetchConsumer` 并运行 15 秒。

**预期结果：**
- 至少观察到 3 次 `Runtime.evaluate` 调用，且 evaluate 的 expression 包含 `visibilityState` 或 `focus`。

---

## TC-006 临时文件与敏感信息不进入 git

**目标：** 验证清理后工作区干净且无敏感配置。

**步骤：**
1. 执行 `git status --porcelain`。
2. 检查 `.gitignore` 是否包含 `config-test-*.toml`、`sign-result.json`、`data/`。
3. 检查 `config.toml` 是否不含真实 ttwid。
4. 检查 `config.example.toml` 是否存在且不含真实 secret。

**预期结果：**
- `git status` 不显示 `config-test-ttwid.toml`、`sign-result.json`、`service-test.log`。
- `.gitignore` 包含上述模式。
- `config.example.toml` 可语法校验通过。

---

## TC-007 CLI `ebg grab --url` 支持 HTTP fetch

**目标：** 验证 CLI 在拿到 HttpFetch 签名结果后走 fetch 消费路径。

**步骤：**
1. mock gRPC 服务端，返回 `kind = HTTP_FETCH` 的 `SignedMaterialProto`。
2. mock `FetchConsumer`（或注入 stub），使其输出一个固定的 `BarrageEvent`。
3. 运行 `ebg grab --url https://live.douyin.com/test`。

**预期结果：**
- CLI 不调用 WSS 连接代码。
- 控制台打印出 mock 的弹幕事件。
- 退出码为 0。

---

## TC-008 Linux 浏览器路径配置校验

**目标：** 验证未配置浏览器路径时给出清晰错误。

**步骤：**
1. 准备一份 `config.example.toml`，其中 `[browser] edge_path` 为空。
2. 运行 `ebg validate --config config.example.toml`。

**预期结果：**
- 校验失败并提示 `browser.edge_path is required`。

---

## TC-009 workspace 全量回归

**目标：** 确保本次改动不破坏现有 WSS 路径。

**步骤：**
1. 运行 `cargo test --workspace`。
2. 运行 `cargo clippy --workspace -- -D warnings`。

**预期结果：**
- 所有测试通过。
- clippy 无新增 warning。

---

## TC-010 真实直播间端到端验收

**目标：** 在真实环境中验证 `ebg grab --url` 能获取弹幕。

**步骤：**
1. 准备 `config.local.toml`，注入用户提供的 `ttwid`。
2. 启动 service：`ebg start --config config.local.toml`。
3. 执行：`ebg grab --url "https://live.douyin.com/950092374817"`。
4. 观察 60 秒。

**预期结果：**
- 60 秒内至少输出一条 `WebcastChatMessage` 或 `WebcastLikeMessage` 事件。
- 服务日志无 ERROR 级别崩溃。
- 结果记录到 `devflow/url-fetch-consumer/verification-log.md`。

---

## 失败处理规则

| 失败场景 | 处理 |
|---|---|
| L1 失败 | 修复对应实现代码，重跑测试 |
| L2 失败 | 修复 `FetchConsumer` 或 mock 数据，重跑测试 |
| L3 失败 | 检查网络/CDP/ttwid 有效性，必要时回退到 blueprint 阶段重新评估 |
