#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${REPO_ROOT}"

pixi run uvpy -c '
import time
import rerun as rr
import rerun.blueprint as rrb

rr.init("rerun_example_web_page_youtube_smoke")
rr.connect_grpc("rerun+http://127.0.0.1:9876/proxy")
rr.send_blueprint(
    rrb.Blueprint(
        rrb.WebPageView(
            name="YouTube",
            config=rrb.WebPageViewConfig(
                url="https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                show_navigation_controls=True,
            ),
        ),
        collapse_panels=True,
    )
)
time.sleep(2)
print("sent YouTube smoke page")
'
