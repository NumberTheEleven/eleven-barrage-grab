#!/usr/bin/env python3
"""
eleven-barrage-grab gRPC 客户端示例 (Python)

连接到 eleven-barrage-grab gRPC 服务，订阅弹幕事件。

依赖：
    pip install grpcio grpcio-tools
    python -m grpc_tools.protoc -I crates/service/proto --python_out=. --grpc_python_out=. crates/service/proto/barrage.proto

注意：当前 MVP 阶段 gRPC server 仅为 stub（仅占用端口），本示例待 T-008 完成后可用。

运行：
    python examples/grpc_client.py
"""
import asyncio
import sys

# 注：完整 gRPC 客户端需要先通过 protoc 生成 Python stub
# 当前 stub 实现不处理 BarrageService，因此示例仅展示接口契约
USAGE = """
gRPC 客户端示例（待 T-008 完成后启用）

完整流程：
1. 从 crates/service/proto/barrage.proto 生成 Python stub：
   python -m grpc_tools.protoc -I crates/service/proto \\
       --python_out=examples/grpc_gen \\
       --grpc_python_out=examples/grpc_gen \\
       crates/service/proto/barrage.proto
2. 运行：
   python examples/grpc_client.py
"""


async def main() -> None:
    print(USAGE)


if __name__ == "__main__":
    asyncio.run(main())