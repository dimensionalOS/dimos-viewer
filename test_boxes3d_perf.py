"""Perf test: log a large voxel grid of solid boxes.

Usage:
    python test_boxes3d_perf.py [N]   # N = grid side length (default 100 → 1M boxes)
"""

import sys

import numpy as np
import rerun as rr

n = int(sys.argv[1]) if len(sys.argv) > 1 else 100
total = n * n * n
print(f"Grid: {n}x{n}x{n} = {total:,} boxes")

# Build positions and colors with numpy — much faster than Python loops at 1M scale.
xs, ys, zs = np.meshgrid(np.arange(n), np.arange(n), np.arange(n), indexing="ij")
centers = np.stack([xs.ravel(), ys.ravel(), zs.ravel()], axis=1).astype(np.float32) * 1.2

# Color = position-based gradient.
colors = np.zeros((total, 3), dtype=np.uint8)
colors[:, 0] = (xs.ravel() * (255 // max(1, n - 1))).astype(np.uint8)
colors[:, 1] = (ys.ravel() * (255 // max(1, n - 1))).astype(np.uint8)
colors[:, 2] = (zs.ravel() * (255 // max(1, n - 1))).astype(np.uint8)

half_sizes = np.full((total, 3), 0.4, dtype=np.float32)

rr.init("boxes3d_perf", spawn=False)
rr.save(f"perf_{n}.rrd")
rr.log(
    "voxels",
    rr.Boxes3D(
        centers=centers,
        half_sizes=half_sizes,
        colors=colors,
        fill_mode="solid",
    ),
)
print(f"Wrote perf_{n}.rrd ({total:,} boxes)")
print(f"Open with: ./target/release/dimos-viewer perf_{n}.rrd")
