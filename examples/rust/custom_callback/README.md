# dimos-viewer

Interactive Rerun viewer for DimOS with click-to-navigate support.

## Installation

```bash
pip install dimos-viewer
```

Or as part of DimOS:

```bash
pip install dimos[viewer]
```

## Usage

### Command Line

```bash
# Launch the viewer (listens on gRPC port 9877)
dimos-viewer

# Or via Python module
python -m dimos_viewer
```

### Programmatic

```python
from dimos_viewer import launch

# Launch in background
proc = launch(port=9877, background=True)

# ... your code using rerun SDK to send data ...

# Clean up
proc.terminate()
```

## How It Works

The DimOS viewer is a customized [Rerun](https://rerun.io/) viewer that:

1. **Accepts gRPC connections** on port 9877 from the Rerun SDK
2. **Publishes click events** over LCM when you click on 3D entities
3. **Enables click-to-navigate** — click a point in the viewer and your robot navigates there

Click events are published on the LCM channel `/clicked_point#geometry_msgs.PointStamped` using the ROS `geometry_msgs/PointStamped` format.

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux    | x86_64      | ✅      |
| Linux    | aarch64     | ✅      |
| macOS    | arm64 (M1+) | ✅      |
| Windows  | -           | ❌ Not supported |

## Versioning

dimos-viewer tracks Rerun's version. Version 0.30.0a1 is based on Rerun 0.30.0-alpha.1.

| dimos-viewer | Based on Rerun |
|-------------|----------------|
| 0.1.x       | 0.30.0-alpha.1 |

## Requirements

- Python >= 3.10
- GPU with Vulkan or OpenGL support (for the viewer UI)
- LCM library (for click event transport, included via UDP multicast)

## License

MIT OR Apache-2.0
