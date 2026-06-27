# eleven-barrage-grab

> 高性能、可复用的直播弹幕服务（Rust 全栈）

## 背景

本项目基于 [`DouyinBarrageGrab`](../DouyinBarrageGrab)（开源项目，已深度修改）改造。

原项目痛点：

- .NET Framework 4.6.2 桌面应用，性能受限
- 系统代理 MITM 模式只能在 Windows 运行
- 单体 WSS 服务，扩展性差
- 稳定性问题（已部分修复）

本项目目标：

- **Rust + Tokio**：性能天花板更高，资源占用更低
- **跨平台**：Windows + Linux 都能跑（Linux 云服务器直连抖音 wss）
- **可复用**：作为引擎嵌入到 `word-guess` 等直播弹幕游戏
- **稳定性优先**：参考原项目 Watchdog / 心跳保底 / 自愈经验

## 文档

- 需求与设计：[`devflow/custom-barrage/`](devflow/custom-barrage/)（DevFlow v3.0 跟踪文件）

## License

TBD
