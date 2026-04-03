# dimos-viewer WebSocket Event Stream

When `dimos-viewer` is started with `--connect`, LCM multicast is not available
(LCM uses UDP multicast which is limited to the local machine or subnet). Instead,
the viewer starts a WebSocket server that broadcasts click and keyboard events as
JSON to any connected client.

## Starting the server

```sh
dimos-viewer --connect [<grpc-proxy-url>] [--ws-port <port>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--connect [url]` | — | Enable connect mode. Optionally specify the gRPC proxy URL (defaults to `rerun+http://127.0.0.1:9877/proxy`). |
| `--ws-port <port>` | `3030` | Port for the WebSocket event server. |

The WebSocket server listens on:

```
ws://0.0.0.0:<ws-port>/ws
```

Multiple clients can connect simultaneously. All connected clients receive every
message. The server does not accept any inbound messages from clients.

## Message format

All messages are UTF-8 JSON objects with a `"type"` string discriminant.

### `heartbeat`

Sent once per second regardless of viewer activity. Useful for connection health
checks and detecting viewer restarts.

```json
{
  "type": "heartbeat",
  "timestamp_ms": 1773869091428
}
```

| Field | Type | Description |
|-------|------|-------------|
| `timestamp_ms` | `u64` | Unix timestamp in milliseconds (from the viewer's system clock). |

### `click`

Sent when the user clicks a 3D point in the Rerun viewport. Corresponds to the
`/clicked_point` convention from RViz / `geometry_msgs/PointStamped`.

```json
{
  "type": "click",
  "x": 1.234,
  "y": -0.567,
  "z": 0.000,
  "entity_path": "/world/ground_plane",
  "timestamp_ms": 1773869091428
}
```

| Field | Type | Description |
|-------|------|-------------|
| `x` | `f64` | World-space X coordinate (metres). |
| `y` | `f64` | World-space Y coordinate (metres). |
| `z` | `f64` | World-space Z coordinate (metres). |
| `entity_path` | `string` | Rerun entity path of the clicked object. |
| `timestamp_ms` | `u64` | Unix timestamp in milliseconds at the moment of the click. |

Click events are debounced: a minimum of 100 ms must elapse between successive
published clicks. Rapid clicks within that window are silently dropped (with a
warning logged after 5 consecutive rapid clicks).

### `twist`

Sent every frame (~60 Hz) while movement keys are held on the keyboard teleop
overlay. Corresponds to `geometry_msgs/Twist` / `/cmd_vel` convention.

The keyboard overlay must first be **engaged** by clicking on it (green border =
active). Clicking anywhere outside the overlay disengages it and sends a `stop`.

```json
{
  "type": "twist",
  "linear_x": 0.5,
  "linear_y": 0.0,
  "linear_z": 0.0,
  "angular_x": 0.0,
  "angular_y": 0.0,
  "angular_z": 0.8
}
```

| Field | Type | Description |
|-------|------|-------------|
| `linear_x` | `f64` | Forward (+) / backward (−) velocity in m/s. |
| `linear_y` | `f64` | Strafe left (+) / right (−) velocity in m/s. |
| `linear_z` | `f64` | Vertical velocity in m/s (always 0 for ground robots). |
| `angular_x` | `f64` | Roll rate in rad/s (always 0). |
| `angular_y` | `f64` | Pitch rate in rad/s (always 0). |
| `angular_z` | `f64` | Yaw left (+) / right (−) rate in rad/s. |

**Key bindings:**

| Key | Effect |
|-----|--------|
| `W` / `↑` | Forward (`linear_x = +0.5`) |
| `S` / `↓` | Backward (`linear_x = −0.5`) |
| `A` / `←` | Turn left (`angular_z = +0.8`) |
| `D` / `→` | Turn right (`angular_z = −0.8`) |
| `Q` | Strafe left (`linear_y = +0.5`) |
| `E` | Strafe right (`linear_y = −0.5`) |
| `Shift` | Speed multiplier ×2 |

Multiple keys can be held simultaneously; their effects are summed.

### `stop`

Sent when all movement keys are released, the overlay is disengaged, or `Space`
is pressed (emergency stop). Signals the robot to halt immediately.

```json
{
  "type": "stop"
}
```

No additional fields. Semantically equivalent to a `twist` with all fields zero.

## Example client (Deno)

A reference test client is provided at `dimos/test_ws_client.ts`:

```sh
deno run --allow-net dimos/test_ws_client.ts
# or with a custom address:
deno run --allow-net dimos/test_ws_client.ts ws://192.168.1.10:3030/ws
```

## Local mode (no `--connect`)

Without `--connect`, the viewer uses LCM UDP multicast instead of WebSocket:

| Channel | Message type |
|---------|-------------|
| `/clicked_point#geometry_msgs.PointStamped` | Click events |
| `/cmd_vel#geometry_msgs.Twist` | Twist / stop commands |

The WebSocket server is **not** started in this mode.
