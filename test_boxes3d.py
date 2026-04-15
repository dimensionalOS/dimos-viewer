"""Smoke test for the new shader-synthesized Boxes3D fast path.

Logs three scenes to test_boxes3d.rrd:
  /single        — one orange box at the origin (sanity check: visible? lit?)
  /grid          — 1000-box voxel-style grid (perf + correctness at scale)
  /rotated       — 5 boxes with non-identity quaternions (rotation correctness)
"""

import math

import numpy as np
import rerun as rr

rr.init("boxes3d_smoke", spawn=False)
rr.save("test_boxes3d.rrd")

# 1. Single box at origin
rr.log(
    "single",
    rr.Boxes3D(
        centers=[[0.0, 0.0, 0.0]],
        half_sizes=[[0.5, 0.5, 0.5]],
        colors=[[255, 128, 0]],
        fill_mode="solid",
    ),
)

# 2. Voxel grid: 10x10x10 = 1000 small cubes
n = 10
spacing = 1.5
centers = []
colors = []
for x in range(n):
    for y in range(n):
        for z in range(n):
            centers.append([x * spacing, y * spacing + 5.0, z * spacing])
            colors.append([x * 25, y * 25, z * 25])
rr.log(
    "grid",
    rr.Boxes3D(
        centers=centers,
        half_sizes=[[0.4, 0.4, 0.4]] * len(centers),
        colors=colors,
        fill_mode="solid",
    ),
)

# 3. Rotated boxes — verify quaternion path
rotated_centers = []
rotated_quats = []
rotated_half_sizes = []
rotated_colors = []
for i in range(5):
    angle = i * math.pi / 5
    rotated_centers.append([i * 2.0 - 4.0, -3.0, 0.0])
    # Quaternion around Z axis: (0, 0, sin(a/2), cos(a/2))
    rotated_quats.append([0.0, 0.0, math.sin(angle / 2), math.cos(angle / 2)])
    rotated_half_sizes.append([0.8, 0.4, 0.2])
    rotated_colors.append([255, 64 + i * 30, 200])
rr.log(
    "rotated",
    rr.Boxes3D(
        centers=rotated_centers,
        half_sizes=rotated_half_sizes,
        quaternions=rotated_quats,
        colors=rotated_colors,
        fill_mode="solid",
    ),
)

print("Wrote test_boxes3d.rrd")
print("Open it with: cargo run -p rerun-cli -- test_boxes3d.rrd")
