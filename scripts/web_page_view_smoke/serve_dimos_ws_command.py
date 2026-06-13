#!/usr/bin/env python3
"""One-shot websocket command server for the DimOS Web Page View smoke test."""

from __future__ import annotations

import base64
import hashlib
import json
import socket
import struct
import time


HOST = "127.0.0.1"
PORT = 3032
WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


COMMANDS = [
    {
        "type": "open_web_page_view",
        "panel_id": "webpage-rerun",
        "title": "Rerun — DimOS WS",
        "url": "https://rerun.io",
        "show_navigation_controls": True,
    },
    {
        "type": "open_web_page_view",
        "panel_id": "webpage-example",
        "title": "Example — DimOS WS no controls",
        "url": "https://example.com",
        "show_navigation_controls": False,
    },
]


def websocket_accept_key(headers: str) -> str:
    for line in headers.splitlines():
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
            digest = hashlib.sha1((key + WEBSOCKET_GUID).encode("ascii")).digest()
            return base64.b64encode(digest).decode("ascii")
    raise RuntimeError("missing Sec-WebSocket-Key")


def send_text_frame(conn: socket.socket, text: str) -> None:
    payload = text.encode("utf-8")
    header = bytearray([0x81])
    if len(payload) < 126:
        header.append(len(payload))
    elif len(payload) < 65536:
        header.extend([126])
        header.extend(struct.pack("!H", len(payload)))
    else:
        header.extend([127])
        header.extend(struct.pack("!Q", len(payload)))
    conn.sendall(bytes(header) + payload)


def main() -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind((HOST, PORT))
        server.listen(1)
        print(f"listening on ws://{HOST}:{PORT}/ws")

        conn, addr = server.accept()
        with conn:
            request = conn.recv(4096).decode("utf-8", errors="replace")
            accept_key = websocket_accept_key(request)
            response = (
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {accept_key}\r\n"
                "\r\n"
            )
            conn.sendall(response.encode("ascii"))
            print(f"client connected from {addr[0]}:{addr[1]}")

            for command in COMMANDS:
                send_text_frame(conn, json.dumps(command))
                print(f"sent command: {command['panel_id']}")
                time.sleep(0.25)

            time.sleep(2.0)


if __name__ == "__main__":
    main()
