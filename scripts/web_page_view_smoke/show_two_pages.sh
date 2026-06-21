#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${REPO_ROOT}"

pixi run uvpy -c '
import time
import rerun as rr
import rerun.blueprint as rrb

rr.init("rerun_example_web_page_manual_smoke")
rr.connect_grpc("rerun+http://127.0.0.1:9876/proxy")
rr.send_blueprint(
    rrb.Blueprint(
        rrb.Horizontal(
            rrb.WebPageView(
                name="Rerun — with controls",
                config=rrb.WebPageViewConfig(
                    url="https://rerun.io",
                    show_navigation_controls=True,
                ),
            ),
            rrb.WebPageView(
                name="Example — no controls",
                config=rrb.WebPageViewConfig(
                    url="https://example.com",
                    show_navigation_controls=False,
                ),
            ),
        ),
        collapse_panels=True,
    )
)
time.sleep(2)
print("sent two side-by-side Web Page Views")
'
