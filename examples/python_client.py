#!/usr/bin/env python3
"""
eleven-barrage-grab WS 客户端示例

连接到本地 eleven-barrage-grab 服务，接收弹幕事件。

依赖：
    pip install websockets

运行：
    python examples/python_client.py ws://127.0.0.1:8888
"""
import asyncio
import json
import sys
from typing import Any, Dict

import websockets


async def handle_barrage_event(event: Dict[str, Any]) -> None:
    """处理单条弹幕事件。"""
    event_type = event.get("event_type", "Unknown")
    data = event.get("data", {})

    if event_type == "ChatMessage":
        user = data.get("user", {}).get("nickname", "匿名")
        content = data.get("content", "")
        print(f"[弹幕] {user}: {content}")
    elif event_type == "GiftMessage":
        user = data.get("user", {}).get("nickname", "匿名")
        gift = data.get("gift", {}).get("name", "未知礼物")
        count = data.get("repeatCount", 1)
        print(f"[礼物] {user} 送出 {gift} x{count}")
    elif event_type == "LikeMessage":
        user = data.get("user", {}).get("nickname", "匿名")
        count = data.get("count", 1)
        total = data.get("total", 0)
        print(f"[点赞] {user} 点赞 +{count} (累计 {total})")
    else:
        # 未知事件类型，原样打印
        print(f"[{event_type}] {json.dumps(data, ensure_ascii=False)[:200]}")


async def main(uri: str) -> None:
    print(f"connecting to {uri} ...")
    async with websockets.connect(uri) as ws:
        print(f"connected. waiting for barrage events (Ctrl+C to exit)...")
        async for message in ws:
            try:
                event = json.loads(message)
                await handle_barrage_event(event)
            except json.JSONDecodeError as e:
                print(f"[warn] invalid JSON: {e}", file=sys.stderr)
            except Exception as e:
                print(f"[warn] error handling event: {e}", file=sys.stderr)


if __name__ == "__main__":
    uri = sys.argv[1] if len(sys.argv) > 1 else "ws://127.0.0.1:8888"
    try:
        asyncio.run(main(uri))
    except KeyboardInterrupt:
        print("\nexiting...")