# Requirements: url-fetch-consumer

## 目标

让 `eleven-barrage-grab` 能够仅根据一个抖音直播间 URL，自动完成签名并持续拉取弹幕消息。重点补齐当前缺失的 **HTTP fetch fallback** 消费路径，使 `ebg grab --url <url>` 在抖音使用 HTTP 轮询而非 WSS push 时仍能正常工作。

最终部署在 **Linux 无 GUI 环境**，不依赖继承本地已登录的 Edge 浏览器；所有 auth cookie（ttwid/sessionid）通过配置/env 注入。

## 包含项

- CDP `Network.responseReceived` + `Network.getResponseBody` 拦截 `webcast/im/fetch` 响应体。
- 将 fetch 响应的 protobuf body 解码为内部 `Response` 消息并送入现有 `Dispatcher`。
- CLI `ebg grab --url <url>` 同时支持 WSS push 与 HTTP fetch 两种签名结果。
- 扩展 `SignedWssMaterial`（或引入新的 `SignedMaterial` 类型）以区分 WSS URL 与 HTTP fetch URL。
- 清理临时测试文件、加固 `.gitignore`、提供 `config.example.toml` 模板。
- Linux 部署文档：浏览器路径仅通过 `[browser] edge_path` 配置指定。

## 排除项

- 自动生成 `ttwid`。
- 验证码/滑块/反爬绕过。
- 持久化存储弹幕。
- 多直播间并发调度优化。
- 浏览器路径自动搜索 fallback。

## 成功标准

- `ebg sign --url <live_url>` 在 30 秒内返回带签名的端点（WSS 或 HTTP fetch）。
- `ebg grab --url <live_url>` 能持续输出 `WebcastChatMessage` / `WebcastGiftMessage` / `WebcastLikeMessage` 等事件。
- 在抖音降级到 HTTP fetch 的网络环境下，`ebg grab` 仍能稳定工作。
- Linux 上仅通过配置文件 + 注入 cookie 即可启动，无需登录桌面浏览器。
- workspace 测试保持全部通过，新增测试覆盖 HTTP fetch 路径。

## 需求清单

### R-001 CDP 拦截 HTTP fetch 响应体（P0）

在现有 `crates/collector/src/signer.rs` 的 CDP 会话基础上，扩展对 `Network.responseReceived` 的监听。

- 当响应 URL 匹配 `/webcast/im/fetch/` 且 content-type 为 protobuf 相关类型时，调用 `Network.getResponseBody` 读取 body。
- 将 body bytes 通过新通道（如 broadcast/async channel）交给消费端。
- 处理 body base64 编码（CDP 默认返回 base64）与可能的 gzip 压缩。
- 仅拦截 `RequestWillBeSent` 已匹配到的请求，避免无差别读取所有响应。

**验收标准：**
- [ ] 单元测试：模拟 CDP `Network.responseReceived` + `Network.getResponseBody` 事件，能正确提取 fetch body bytes。
- [ ] 在真实直播间中，服务日志出现 `captured fetch response body, length=N`。

**依赖：** 无。

---

### R-002 HTTP fetch 消费端（P0）

将 R-001 捕获到的 protobuf body 解码并分发。

- 复用/扩展 `crates/core/src/decoder.rs` 的解码逻辑：fetch body 是裸 `Response` protobuf（没有 WSS 外层 `WssResponse`）。
- 将解码后的 `Message` 列表送入现有 `Dispatcher`，与 WSS 路径复用同一下游事件通道。
- 处理 fetch 响应中 `cursor` / `internal_ext` 等字段的传递（若浏览器未自动管理，则通过 CDP JS 注入推进轮询）。
- 添加简单的去重/有序缓存，避免同一消息重复推送。

**验收标准：**
- [ ] 给定一个已录制的 fetch protobuf payload，单元测试能解码出至少一种弹幕事件。
- [ ] `ebg grab` 在 HTTP fetch 模式下运行 60 秒，能持续收到事件。
- [ ] 与 WSS 模式共享同一下游事件结构，不引入新的消息 schema。

**依赖：** R-001。

---

### R-003 保持浏览器轮询活跃（P1）

抖音页面在后台/无操作时可能降低轮询频率或暂停。通过 CDP 维持活跃。

- 在页面加载后通过 `Runtime.evaluate` 注入轻量 JS：保持 `Page.visibilityState = 'visible'`、定时触发 `mousemove`/`focus` 事件，或覆盖页面轮询停止逻辑。
- 初期目标：让浏览器自身发起的 `webcast/im/fetch` 请求间隔保持在可接受范围（≤5s）。
- 不强制切换为纯 JS 轮询；优先复用浏览器原生 fetch。

**验收标准：**
- [ ] 服务启动后 5 分钟内，`webcast/im/fetch` 请求未出现长时间（>30s）中断。
- [ ] 若观察到轮询停止，服务记录 warn 日志并尝试通过 CDP 重新激活页面。

**依赖：** 无（可与 R-002 并行）。

---

### R-004 清理临时文件与敏感信息（P1）

将当前工作区中的临时测试文件和敏感配置移出 git 跟踪范围。

- 删除 `config-test-ttwid.toml`、`sign-result.json`、`service-test.log`。
- 在 `.gitignore` 中增加：
  ```
  config-test-*.toml
  sign-result.json
  data/
  ```
- 提供 `config.example.toml`，包含所有必要配置段但不含真实 ttwid。
- 在 `config.toml` 中移除测试用的真实 ttwid（已 revert，需保持 clean）。

**验收标准：**
- [ ] `git status --porcelain` 不再出现上述临时文件。
- [ ] `.gitignore` 提交到 feature 分支。
- [ ] `config.example.toml` 存在且通过语法检查（`cargo run --bin ebg -- --config config.example.toml --help` 不报错）。

**依赖：** 无。

---

### R-005 CLI `ebg grab --url` 支持 HTTP fetch（P0）

让 `ebg grab` 能根据 URL 自动签名并消费 HTTP fetch 结果。

- 解析签名结果：若 `wss_url` 是 `wss://.../webcast/im/push`，走现有 WSS consumer；若是 `https://.../webcast/im/fetch/`，走 R-002 的 HTTP fetch consumer。
- 引入 `SignedMaterial` 枚举或给 `SignedWssMaterial` 增加 `kind` 字段，避免下游依赖 URL 字符串判断。
- 更新 CLI help 与错误提示：`NO_WSS_CAPTURED` 改为更中性的 `NO_SIGNED_ENDPOINT_CAPTURED`。

**验收标准：**
- [ ] `ebg grab --url "https://live.douyin.com/950092374817"` 在 HTTP fetch 模式下能输出事件。
- [ ] 单元测试覆盖 `SignedMaterial` 枚举的 URL 分类逻辑。
- [ ] 原 WSS 路径的 CLI 行为不变。

**依赖：** R-002。

---

### R-006 Linux 部署文档与浏览器路径配置（P2）

明确 Linux 无 GUI 环境的运行方式。

- 在 `config.example.toml` 中给出 Linux 下 `[browser] edge_path` 示例（如 `/usr/bin/microsoft-edge`、`/usr/bin/chromium`）。
- README 或 `docs/linux-deployment.md` 说明：
  - 安装 headless Chromium/Edge 的方式。
  - 如何通过 `config.local.toml` 或环境变量注入 `ttwid`。
  - 不继承浏览器 profile 的设计。

**验收标准：**
- [ ] 文档中列出最小启动命令。
- [ ] `[browser] edge_path` 未配置时启动给出清晰错误提示。

**依赖：** R-004、R-005。

---

### R-007 测试与回归（P1）

确保 HTTP fetch 路径有测试覆盖，且不影响现有功能。

- 为 R-001 增加 CDP mock 测试。
- 为 R-002 增加 fetch protobuf 解码测试（可用录制数据或构造最小 payload）。
- 运行 `cargo test --workspace` 并保证全部通过。
- 手动验收：用用户提供的 URL + ttwid 跑一次端到端 `ebg grab`。

**验收标准：**
- [ ] workspace 测试全绿。
- [ ] 新增测试行覆盖 R-001、R-002、R-005 的核心分支。
- [ ] 端到端验收报告写入 `devflow/url-fetch-consumer/verification-log.md`。

**依赖：** R-001、R-002、R-005。

## 约束

- 消费端必须依赖浏览器的网络路径，host 进程不直接请求 Douyin。
- 所有 secret 不提交到 git。
- Linux 生产环境无 GUI，浏览器路径需配置指定。
- 不修改现有 protobuf schema；复用现有 `WssResponse`/`Response` 定义。
