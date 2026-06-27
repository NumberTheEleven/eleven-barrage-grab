# 构建与验证指南

> Phase 4 完成后的 L1/L2 验证步骤

## 前置要求

- **Rust 1.74+**（推荐 `rustup` 安装）
- **protoc**（可选，`prost-build` 会自动下载 prebuilt binary）
- **Git**

## 安装 Rust

如果系统未安装 Rust：

```bash
# Windows (PowerShell)
winget install Rustlang.Rustup
# 或下载：https://rustup.rs/

# Linux / macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## L1 烟雾验证（编译 + clippy + fmt）

在 `eleven-barrage-grab` 仓库根目录执行：

```bash
# 1. cargo build（编译所有 crate）
cargo build --release

# 2. cargo test（运行单元测试）
cargo test --release --all

# 3. cargo clippy（静态分析）
cargo clippy --all-targets --all-features -- -D warnings

# 4. cargo fmt（格式检查）
cargo fmt --check
```

**预期结果：**
- `cargo build` 编译成功
- `cargo test` 所有测试通过
- `cargo clippy` 无 warning
- `cargo fmt --check` 无格式问题

## 跨平台构建

```bash
# Linux 二进制（在 Windows 上交叉编译需要 cross 工具）
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# Windows 二进制
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
```

## 启动验证

```bash
# 1. 复制配置模板
cp config.example.toml config.toml
# 编辑 config.toml，设置 service.room_id

# 2. 显示当前配置（不启动）
./target/release/eleven-barrage-grab show-config

# 3. 验证配置
./target/release/eleven-barrage-grab validate

# 4. 启动服务
./target/release/eleven-barrage-grab start --config config.toml
```

## Python 客户端验证

```bash
pip install websockets
python examples/python_client.py ws://127.0.0.1:8888
```

## 已知编译问题与解决

### 问题 1：prost-build 无法找到 protoc

如果 `cargo build` 报错找不到 `protoc`：

```bash
# Windows (Chocolatey)
choco install protoc

# 或下载 prebuilt：https://github.com/protocolbuffers/protobuf/releases
```

`prost-build` 默认会自动下载 protoc，无需手动安装。如果下载失败，设置环境变量：

```bash
PROTOC=/path/to/protoc.exe cargo build
```

### 问题 2：tokio-tungstenite native-tls 依赖

如果启用 `native-tls-vendored` feature 出错，可改为：

```toml
tokio-tungstenite = { version = "0.23", default-features = false, features = ["connect"] }
```

### 问题 3：metrics-exporter-prometheus 版本

如果 `0.14` 版本 API 不兼容，尝试：

```toml
metrics-exporter-prometheus = "0.13"
```

## Phase 4 验证 checklist

完成 L1/L2 验证后：

- [ ] `cargo build --release` 通过
- [ ] `cargo test --release` 全部通过（约 30+ 单元测试）
- [ ] `cargo clippy` 无 warning
- [ ] `cargo fmt --check` 通过
- [ ] Python 客户端可连接到 mock 或真实服务
- [ ] 二进制文件大小 < 30MB

完成后进入 Phase 5：测试验证（含真实房间 L3 验收）。