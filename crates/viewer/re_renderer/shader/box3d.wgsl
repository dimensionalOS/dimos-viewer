#import <./global_bindings.wgsl>
#import <./types.wgsl>
#import <./utils/camera.wgsl>
#import <./utils/flags.wgsl>
#import <./utils/depth_offset.wgsl>

// Per-instance data textures (indexed by `vertex_index / 36`).
@group(1) @binding(0) var center_radius_texture:       texture_2d<f32>;
@group(1) @binding(1) var half_size_texture:           texture_2d<f32>;
@group(1) @binding(2) var quaternion_texture:          texture_2d<f32>;
@group(1) @binding(3) var color_texture:               texture_2d<f32>; // sRGB → linear on load
@group(1) @binding(4) var picking_instance_id_texture: texture_2d<u32>;

struct DrawDataUniformBuffer {
    radius_boost_in_ui_points: f32,
    _padding: vec4f,
};
@group(1) @binding(5) var<uniform> draw_data: DrawDataUniformBuffer;

struct BatchUniformBuffer {
    world_from_obj: mat4x4f,
    flags: u32,
    depth_offset: f32,
    _padding: vec2u,
    outline_mask: vec2u,
    picking_layer_object_id: vec2u,
};
@group(2) @binding(0) var<uniform> batch: BatchUniformBuffer;

// Flags — keep in sync with box3d.rs#Box3DBatchFlags.
const FLAG_ENABLE_SHADING: u32 = 1u;

// 36 corner indices forming the 12 triangles of the unit cube.
// Corner index `i` encodes sign bits in XYZ:
//   x = ((i >> 0) & 1) * 2 - 1
//   y = ((i >> 1) & 1) * 2 - 1
//   z = ((i >> 2) & 1) * 2 - 1
// Triangles are CW in world space, which becomes CCW in NDC after re_renderer's
// reverse-Z right-handed projection — matching wgpu's default `FrontFace::Ccw`.
// Ordering: -X, +X, -Y, +Y, -Z, +Z (2 triangles per face, 6 verts per face).
const CUBE_TRI_CORNERS: array<u32, 36> = array<u32, 36>(
    // -X face
    0u, 3u, 1u,   0u, 2u, 3u,
    // +X face
    4u, 7u, 6u,   4u, 5u, 7u,
    // -Y face
    0u, 5u, 4u,   0u, 1u, 5u,
    // +Y face
    2u, 7u, 3u,   2u, 6u, 7u,
    // -Z face
    0u, 6u, 2u,   0u, 4u, 6u,
    // +Z face
    1u, 7u, 5u,   1u, 3u, 7u,
);

// Object-space face normal per vertex (one of ±X, ±Y, ±Z, repeated 6 times per face).
const CUBE_TRI_NORMALS: array<vec3f, 36> = array<vec3f, 36>(
    // -X face
    vec3f(-1.0, 0.0, 0.0), vec3f(-1.0, 0.0, 0.0), vec3f(-1.0, 0.0, 0.0),
    vec3f(-1.0, 0.0, 0.0), vec3f(-1.0, 0.0, 0.0), vec3f(-1.0, 0.0, 0.0),
    // +X face
    vec3f( 1.0, 0.0, 0.0), vec3f( 1.0, 0.0, 0.0), vec3f( 1.0, 0.0, 0.0),
    vec3f( 1.0, 0.0, 0.0), vec3f( 1.0, 0.0, 0.0), vec3f( 1.0, 0.0, 0.0),
    // -Y face
    vec3f(0.0, -1.0, 0.0), vec3f(0.0, -1.0, 0.0), vec3f(0.0, -1.0, 0.0),
    vec3f(0.0, -1.0, 0.0), vec3f(0.0, -1.0, 0.0), vec3f(0.0, -1.0, 0.0),
    // +Y face
    vec3f(0.0,  1.0, 0.0), vec3f(0.0,  1.0, 0.0), vec3f(0.0,  1.0, 0.0),
    vec3f(0.0,  1.0, 0.0), vec3f(0.0,  1.0, 0.0), vec3f(0.0,  1.0, 0.0),
    // -Z face
    vec3f(0.0, 0.0, -1.0), vec3f(0.0, 0.0, -1.0), vec3f(0.0, 0.0, -1.0),
    vec3f(0.0, 0.0, -1.0), vec3f(0.0, 0.0, -1.0), vec3f(0.0, 0.0, -1.0),
    // +Z face
    vec3f(0.0, 0.0,  1.0), vec3f(0.0, 0.0,  1.0), vec3f(0.0, 0.0,  1.0),
    vec3f(0.0, 0.0,  1.0), vec3f(0.0, 0.0,  1.0), vec3f(0.0, 0.0,  1.0),
);

fn corner_offset(i: u32) -> vec3f {
    return vec3f(
        f32((i >> 0u) & 1u),
        f32((i >> 1u) & 1u),
        f32((i >> 2u) & 1u),
    ) * 2.0 - vec3f(1.0);
}

// Rotate vector `v` by unit quaternion `q` (XYZW).
fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Fetch an RGBA float texel by flat instance index (column-major layout).
fn load_f(tex: texture_2d<f32>, idx: u32) -> vec4f {
    let dim = textureDimensions(tex);
    return textureLoad(tex, vec2u(idx % dim.x, idx / dim.x), 0);
}

fn load_u(tex: texture_2d<u32>, idx: u32) -> vec4u {
    let dim = textureDimensions(tex);
    return textureLoad(tex, vec2u(idx % dim.x, idx / dim.x), 0);
}

struct VertexOut {
    @builtin(position) position: vec4f,

    @location(0) @interpolate(perspective) world_position: vec3f,
    @location(1) @interpolate(flat) normal_world: vec3f,
    @location(2) @interpolate(flat) color: vec4f,
    @location(3) @interpolate(flat) picking_instance_id: vec2u,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32) -> VertexOut {
    let instance_idx = vertex_idx / 36u;
    let slot = vertex_idx % 36u;
    let corner_idx = CUBE_TRI_CORNERS[slot];
    let normal_obj = CUBE_TRI_NORMALS[slot];

    let center_radius = load_f(center_radius_texture, instance_idx);
    let half_size     = load_f(half_size_texture,     instance_idx).xyz;
    let q             = load_f(quaternion_texture,    instance_idx);
    let color         = load_f(color_texture,         instance_idx);
    let pid           = load_u(picking_instance_id_texture, instance_idx).xy;

    // Object-space corner position, then apply instance rotation around center.
    let local = corner_offset(corner_idx) * half_size; // ±half_size per axis
    let rotated = quat_rotate(q, local);
    let obj_pos = vec4f(center_radius.xyz + rotated, 1.0);

    // Degenerate instance guard: if half_size is all-zero, the vertex collapses onto
    // the center, producing zero-area triangles that the rasterizer discards for free.

    let world_pos4 = batch.world_from_obj * obj_pos;
    let world_pos = world_pos4.xyz / world_pos4.w;

    // Normal: rotate by instance quaternion and by the rotational part of world_from_obj.
    let n_obj = quat_rotate(q, normal_obj);
    let n_world = normalize((batch.world_from_obj * vec4f(n_obj, 0.0)).xyz);

    var out: VertexOut;
    out.position = apply_depth_offset(
        frame.projection_from_world * vec4f(world_pos, 1.0),
        batch.depth_offset);
    out.world_position = world_pos;
    out.normal_world = n_world;
    out.color = color;
    out.picking_instance_id = pid;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    var shading = 1.0;
    if has_any_flag(batch.flags, FLAG_ENABLE_SHADING) {
        // Simple half-Lambert with a headlight from the camera.
        let view_dir = normalize(frame.camera_position - in.world_position);
        let ndotl = max(0.0, dot(normalize(in.normal_world), view_dir));
        shading = 0.35 + 0.65 * ndotl;
    }
    return vec4f(in.color.rgb * shading, in.color.a);
}

@fragment
fn fs_main_picking_layer(in: VertexOut) -> @location(0) vec4u {
    return vec4u(batch.picking_layer_object_id, in.picking_instance_id);
}

@fragment
fn fs_main_outline_mask(in: VertexOut) -> @location(0) vec2u {
    return batch.outline_mask;
}
