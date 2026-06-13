#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
LOG_PATH="${WEB_PAGE_VIEW_SMOKE_LOG:-/tmp/rerun_web_page_view_smoke.log}"

cd "${REPO_ROOT}"

pkill -x dimos-viewer 2>/dev/null || true
pkill -x rerun 2>/dev/null || true

nohup env -u WAYLAND_DISPLAY \
  WINIT_UNIX_BACKEND=x11 \
  cargo run -p dimos-viewer \
    --features rerun/native_webview \
    -- \
    --new \
    "$@" \
  > "${LOG_PATH}" 2>&1 < /dev/null &

python3 - <<'PY'
import socket
import time

for _ in range(240):
    sock = socket.socket()
    try:
        sock.connect(("127.0.0.1", 9876))
        print("viewer ready: rerun+http://127.0.0.1:9876/proxy")
        break
    except OSError:
        time.sleep(0.5)
    finally:
        sock.close()
else:
    raise SystemExit("viewer did not open gRPC port 9876")
PY

echo "viewer log: ${LOG_PATH}"
