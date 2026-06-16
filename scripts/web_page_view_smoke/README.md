# Web page view smoke scripts

These scripts manually smoke-test the experimental native Web Page View.

## Launch the native viewer

```bash
scripts/web_page_view_smoke/launch_viewer.sh
```

The script forces the native X11 path on Linux because the current `wry` backend uses child-window embedding.

## Send two side-by-side pages through the Python blueprint API

```bash
scripts/web_page_view_smoke/show_two_pages.sh
```

Expected result:

- left panel: `https://rerun.io` with Web Page View controls visible
- right panel: `https://example.com` with Web Page View controls hidden

## Send a YouTube page through the Python blueprint API

```bash
scripts/web_page_view_smoke/show_youtube.sh
```

Linux video playback depends on WebKitGTK/GStreamer codecs. If YouTube says the browser cannot play the video, install the system GStreamer codec plugins, then relaunch the viewer.

## Test the DimOS websocket command path

Start the one-shot command server in one terminal:

```bash
python3 scripts/web_page_view_smoke/serve_dimos_ws_command.py
```

Launch the viewer with the websocket URL in another terminal:

```bash
scripts/web_page_view_smoke/launch_viewer.sh --ws-url ws://127.0.0.1:3032/ws
```

Expected result: the DimOS websocket command creates two Web Page View panels using the same blueprint-backed Web Page View state as the Python API path.
