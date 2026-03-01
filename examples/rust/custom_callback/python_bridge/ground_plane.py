"""Invisible ground plane for click-anywhere support.

Rerun's picking system only reports coordinates when clicking on an entity.
This module logs a large, nearly-invisible ground plane mesh so that clicks
on "empty" floor space still produce world-space coordinates.

Usage:
    from ground_plane import log_ground_plane
    
    # After rr.init() and rr.connect_grpc():
    log_ground_plane()  # logs at z=0, 100m x 100m
    
    # Custom size/height:
    log_ground_plane(size=200.0, z=0.5, entity_path="world/floor")
"""

import rerun as rr
import numpy as np


def log_ground_plane(
    size: float = 100.0,
    z: float = 0.0,
    entity_path: str = "world/ground_plane",
    opacity: int = 1,
    color: tuple = (40, 40, 40),
    subdivisions: int = 10,
) -> None:
    """Log an invisible ground plane mesh for click-anywhere support.
    
    Args:
        size: Half-extent of the plane in meters (default 100 = 200m x 200m total)
        z: Height of the ground plane (default 0.0)
        entity_path: Rerun entity path for the plane
        opacity: Alpha value 0-255 (default 1 = nearly invisible)
        color: RGB color tuple (default dark gray)
        subdivisions: Grid subdivisions (more = better picking accuracy)
    """
    # Generate a subdivided grid mesh for better picking resolution.
    # A single quad has poor pick accuracy at large scales because the
    # GPU interpolation produces imprecise barycentric coords.
    steps = subdivisions + 1
    xs = np.linspace(-size, size, steps)
    ys = np.linspace(-size, size, steps)
    xx, yy = np.meshgrid(xs, ys)
    zz = np.full_like(xx, z)
    
    # Vertices: (steps * steps) points
    vertices = np.stack([xx.flatten(), yy.flatten(), zz.flatten()], axis=-1).astype(np.float32)
    
    # Triangles: 2 per grid cell
    triangles = []
    for row in range(subdivisions):
        for col in range(subdivisions):
            i = row * steps + col
            triangles.append([i, i + 1, i + steps])
            triangles.append([i + 1, i + steps + 1, i + steps])
    
    triangle_indices = np.array(triangles, dtype=np.uint32)
    
    # RGBA color with near-zero alpha
    rgba = [color[0], color[1], color[2], opacity]
    vertex_colors = np.tile(rgba, (len(vertices), 1)).astype(np.uint8)
    
    rr.log(
        entity_path,
        rr.Mesh3D(
            vertex_positions=vertices,
            triangle_indices=triangle_indices,
            vertex_colors=vertex_colors,
        ),
    )
