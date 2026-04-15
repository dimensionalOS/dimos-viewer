//! Box renderer for efficient rendering of large numbers of solid 3D boxes.
//!
//! How it works:
//! =================
//! Each box is rendered as a 12-triangle (36-vertex) cube whose geometry is synthesized
//! entirely in the vertex shader from `@builtin(vertex_index)`. No vertex buffer, no
//! index buffer — per-instance data (center, `half_size`, rotation, color, picking id)
//! lives in `DataTextureSource`-backed data textures, indexed as `vertex_idx / 36`.
//!
//! This mirrors the [`super::point_cloud`] renderer's approach and is meant to replace the
//! generic [`super::mesh_renderer`] path for the `Solid` `FillMode` of `Boxes3D`, which is
//! ~100× slower at high instance counts (see <https://github.com/rerun-io/rerun/issues/10276>).
//!
//! Wireframe boxes still go through `LineDrawableBuilder`.

use std::num::NonZeroU64;
use std::ops::Range;

use bitflags::bitflags;
use enumset::{EnumSet, enum_set};
use itertools::Itertools as _;
use smallvec::smallvec;

use super::{DrawData, DrawError, RenderContext, Renderer};
use crate::allocator::create_and_fill_uniform_buffer_batch;
use crate::draw_phases::{
    DrawPhase, OutlineMaskProcessor, PickingLayerObjectId, PickingLayerProcessor,
};
use crate::renderer::{DrawDataDrawable, DrawInstruction, DrawableCollectionViewInfo};
use crate::view_builder::ViewBuilder;
use crate::wgpu_resources::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, GpuBindGroup, GpuBindGroupLayoutHandle,
    GpuRenderPipelineHandle, GpuRenderPipelinePoolAccessor, PipelineLayoutDesc, RenderPipelineDesc,
};
use crate::{
    Box3DBuilder, DebugLabel, DepthOffset, DrawableCollector, OutlineMaskPreference,
    include_shader_module,
};

bitflags! {
    /// Property flags for a box batch.
    ///
    /// Needs to be kept in sync with `box3d.wgsl`.
    #[repr(C)]
    #[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct Box3DBatchFlags : u32 {
        /// If true, apply simple face-normal Lambert shading.
        const FLAG_ENABLE_SHADING = 0b0001;
    }
}

pub mod gpu_data {
    use crate::draw_phases::PickingLayerObjectId;
    use crate::{Size, wgpu_buffer_types};

    /// Per-instance center and radius (radius unused in v1 solid; reserved for wireframe v2).
    #[repr(C, packed)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct CenterRadius {
        pub center: glam::Vec3,
        pub radius: Size,
    }
    static_assertions::assert_eq_size!(CenterRadius, glam::Vec4);

    /// Per-instance half-size (xyz) with padding to `Vec4`.
    #[repr(C, packed)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct HalfSize {
        pub half_size: glam::Vec3,
        pub(crate) _pad: f32,
    }
    static_assertions::assert_eq_size!(HalfSize, glam::Vec4);

    /// Per-instance orientation quaternion (XYZW), normalized on CPU.
    #[repr(C, packed)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct Quaternion {
        pub q: glam::Vec4,
    }
    static_assertions::assert_eq_size!(Quaternion, glam::Vec4);

    /// Uniform buffer that changes once per draw data rendering.
    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct DrawDataUniformBuffer {
        pub radius_boost_in_ui_points: wgpu_buffer_types::F32RowPadded,
        pub end_padding: [wgpu_buffer_types::PaddingRow; 16 - 1],
    }

    /// Uniform buffer that changes for every batch of boxes.
    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct BatchUniformBuffer {
        pub world_from_obj: wgpu_buffer_types::Mat4,

        pub flags: u32, // Box3DBatchFlags
        pub depth_offset: f32,
        pub _row_padding: [f32; 2],

        pub outline_mask_ids: wgpu_buffer_types::UVec2,
        pub picking_object_id: PickingLayerObjectId,

        pub end_padding: [wgpu_buffer_types::PaddingRow; 16 - 6],
    }
}

/// Internal, ready-to-draw representation of [`Box3DBatchInfo`].
#[derive(Clone)]
struct Box3DBatch {
    bind_group: GpuBindGroup,
    vertex_range: Range<u32>,
    active_phases: EnumSet<DrawPhase>,
}

/// A 3D-box drawing operation. Expected to be recreated every frame.
#[derive(Clone)]
pub struct Box3DDrawData {
    bind_group_all_boxes: Option<GpuBindGroup>,
    bind_group_all_boxes_outline_mask: Option<GpuBindGroup>,
    batches: Vec<Box3DBatch>,
}

impl DrawData for Box3DDrawData {
    type Renderer = Box3DRenderer;

    fn collect_drawables(
        &self,
        _view_info: &DrawableCollectionViewInfo,
        collector: &mut DrawableCollector<'_>,
    ) {
        // TODO(#1611): transparency. For now boxes are opaque only and drawn late.
        for (batch_idx, batch) in self.batches.iter().enumerate() {
            collector.add_drawable(
                batch.active_phases,
                DrawDataDrawable {
                    distance_sort_key: f32::MAX,
                    draw_data_payload: batch_idx as _,
                },
            );
        }
    }
}

/// Data that is valid for a batch of solid 3D boxes.
pub struct Box3DBatchInfo {
    pub label: DebugLabel,

    /// Transformation applied to box centers and orientations.
    ///
    /// TODO(andreas): We don't apply scaling to the box extents yet. Need to pass a scaling factor
    /// like `Mat3::from(world_from_obj).determinant().abs().cbrt()` if needed.
    pub world_from_obj: glam::Affine3A,

    /// Additional properties of this box batch.
    pub flags: Box3DBatchFlags,

    /// Number of boxes covered by this batch.
    pub box_count: u32,

    /// Optional outline mask setting for the entire batch.
    pub overall_outline_mask_ids: OutlineMaskPreference,

    /// Per-instance-range outline mask overrides.
    ///
    /// Ranges are relative to this batch, measured in **boxes** (not vertices).
    pub additional_outline_mask_ids_instance_ranges: Vec<(Range<u32>, OutlineMaskPreference)>,

    /// Picking object id that applies for the entire batch.
    pub picking_object_id: PickingLayerObjectId,

    /// Depth offset applied after projection.
    pub depth_offset: DepthOffset,
}

impl Default for Box3DBatchInfo {
    #[inline]
    fn default() -> Self {
        Self {
            label: DebugLabel::default(),
            world_from_obj: glam::Affine3A::IDENTITY,
            flags: Box3DBatchFlags::FLAG_ENABLE_SHADING,
            box_count: 0,
            overall_outline_mask_ids: OutlineMaskPreference::NONE,
            additional_outline_mask_ids_instance_ranges: Vec::new(),
            picking_object_id: Default::default(),
            depth_offset: 0,
        }
    }
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum Box3DDrawDataError {
    #[error("Failed to transfer data to the GPU: {0}")]
    FailedTransferringDataToGpu(#[from] crate::allocator::CpuWriteGpuReadError),
}

/// Number of vertices emitted per box instance (12 triangles × 3 vertices).
const VERTS_PER_BOX: u32 = 36;

impl Box3DDrawData {
    pub fn new(builder: Box3DBuilder<'_>) -> Result<Self, Box3DDrawDataError> {
        re_tracing::profile_function!();

        let Box3DBuilder {
            ctx,
            center_radius_buffer,
            half_size_buffer,
            quaternion_buffer,
            color_buffer,
            picking_instance_ids_buffer,
            batches,
            radius_boost_in_ui_points_for_outlines,
        } = builder;

        let box_renderer = ctx.renderer::<Box3DRenderer>();
        let batches = batches.as_slice();

        if center_radius_buffer.is_empty() {
            return Ok(Self {
                bind_group_all_boxes: None,
                bind_group_all_boxes_outline_mask: None,
                batches: Vec::new(),
            });
        }

        let num_instances = center_radius_buffer.len();

        let fallback_batches = [Box3DBatchInfo {
            label: "fallback_batches".into(),
            world_from_obj: glam::Affine3A::IDENTITY,
            flags: Box3DBatchFlags::empty(),
            box_count: num_instances as _,
            overall_outline_mask_ids: OutlineMaskPreference::NONE,
            additional_outline_mask_ids_instance_ranges: Vec::new(),
            picking_object_id: Default::default(),
            depth_offset: 0,
        }];
        let batches = if batches.is_empty() {
            &fallback_batches
        } else {
            batches
        };

        let center_radius_texture = center_radius_buffer.finish(
            wgpu::TextureFormat::Rgba32Float,
            "Box3DDrawData::center_radius_texture",
        )?;
        let half_size_texture = half_size_buffer.finish(
            wgpu::TextureFormat::Rgba32Float,
            "Box3DDrawData::half_size_texture",
        )?;
        let quaternion_texture = quaternion_buffer.finish(
            wgpu::TextureFormat::Rgba32Float,
            "Box3DDrawData::quaternion_texture",
        )?;
        let color_texture = color_buffer.finish(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "Box3DDrawData::color_texture",
        )?;
        let picking_instance_id_texture = picking_instance_ids_buffer.finish(
            wgpu::TextureFormat::Rg32Uint,
            "Box3DDrawData::picking_instance_id_texture",
        )?;

        let draw_data_uniform_buffer_bindings = create_and_fill_uniform_buffer_batch(
            ctx,
            "Box3DDrawData::DrawDataUniformBuffer".into(),
            [
                gpu_data::DrawDataUniformBuffer {
                    radius_boost_in_ui_points: 0.0.into(),
                    end_padding: Default::default(),
                },
                gpu_data::DrawDataUniformBuffer {
                    radius_boost_in_ui_points: radius_boost_in_ui_points_for_outlines.into(),
                    end_padding: Default::default(),
                },
            ]
            .into_iter(),
        );
        let (draw_data_uniform_buffer_bindings_normal, draw_data_uniform_buffer_bindings_outline) =
            draw_data_uniform_buffer_bindings
                .into_iter()
                .collect_tuple()
                .unwrap();

        let mk_bind_group = |label, draw_data_uniform_buffer_binding| {
            ctx.gpu_resources.bind_groups.alloc(
                &ctx.device,
                &ctx.gpu_resources,
                &BindGroupDesc {
                    label,
                    entries: smallvec![
                        BindGroupEntry::DefaultTextureView(center_radius_texture.handle),
                        BindGroupEntry::DefaultTextureView(half_size_texture.handle),
                        BindGroupEntry::DefaultTextureView(quaternion_texture.handle),
                        BindGroupEntry::DefaultTextureView(color_texture.handle),
                        BindGroupEntry::DefaultTextureView(picking_instance_id_texture.handle),
                        draw_data_uniform_buffer_binding,
                    ],
                    layout: box_renderer.bind_group_layout_all_boxes,
                },
            )
        };

        let bind_group_all_boxes = mk_bind_group(
            "Box3DDrawData::bind_group_all_boxes".into(),
            draw_data_uniform_buffer_bindings_normal,
        );
        let bind_group_all_boxes_outline_mask = mk_bind_group(
            "Box3DDrawData::bind_group_all_boxes_outline_mask".into(),
            draw_data_uniform_buffer_bindings_outline,
        );

        // Process batches.
        let mut batches_internal = Vec::with_capacity(batches.len());
        {
            let uniform_buffer_bindings = create_and_fill_uniform_buffer_batch(
                ctx,
                "box3d batch uniform buffers".into(),
                batches
                    .iter()
                    .map(|batch_info| gpu_data::BatchUniformBuffer {
                        world_from_obj: batch_info.world_from_obj.into(),
                        flags: batch_info.flags.bits(),
                        outline_mask_ids: batch_info
                            .overall_outline_mask_ids
                            .0
                            .unwrap_or_default()
                            .into(),
                        picking_object_id: batch_info.picking_object_id,
                        depth_offset: batch_info.depth_offset as f32,

                        _row_padding: [0.0, 0.0],
                        end_padding: Default::default(),
                    }),
            );

            // Additional micro-batches for per-range outline mask overrides.
            let mut uniform_buffer_bindings_mask_only_batches =
                create_and_fill_uniform_buffer_batch(
                    ctx,
                    "box3d batch uniform buffers - mask only".into(),
                    batches
                        .iter()
                        .flat_map(|batch_info| {
                            batch_info
                                .additional_outline_mask_ids_instance_ranges
                                .iter()
                                .map(|(_, mask)| gpu_data::BatchUniformBuffer {
                                    world_from_obj: batch_info.world_from_obj.into(),
                                    flags: batch_info.flags.bits(),
                                    outline_mask_ids: mask.0.unwrap_or_default().into(),
                                    picking_object_id: batch_info.picking_object_id,
                                    depth_offset: batch_info.depth_offset as f32,

                                    _row_padding: [0.0, 0.0],
                                    end_padding: Default::default(),
                                })
                        })
                        .collect::<Vec<_>>()
                        .into_iter(),
                )
                .into_iter();

            let mut start_instance_for_next_batch = 0u32;
            for (batch_info, uniform_buffer_binding) in
                batches.iter().zip(uniform_buffer_bindings.into_iter())
            {
                let instance_range_end = start_instance_for_next_batch + batch_info.box_count;
                let mut active_phases = enum_set![DrawPhase::Opaque | DrawPhase::PickingLayer];
                if batch_info.overall_outline_mask_ids.is_some() {
                    active_phases.insert(DrawPhase::OutlineMask);
                }

                batches_internal.push(box_renderer.create_box_batch(
                    ctx,
                    batch_info.label.clone(),
                    uniform_buffer_binding,
                    start_instance_for_next_batch..instance_range_end,
                    active_phases,
                ));

                for (range, _) in &batch_info.additional_outline_mask_ids_instance_ranges {
                    let range = (range.start + start_instance_for_next_batch)
                        ..(range.end + start_instance_for_next_batch);
                    batches_internal.push(box_renderer.create_box_batch(
                        ctx,
                        format!("{:?} strip-only {:?}", batch_info.label, range).into(),
                        uniform_buffer_bindings_mask_only_batches.next().unwrap(),
                        range.clone(),
                        enum_set![DrawPhase::OutlineMask],
                    ));
                }

                start_instance_for_next_batch = instance_range_end;

                if start_instance_for_next_batch >= num_instances as u32 {
                    break;
                }
            }
        }

        Ok(Self {
            bind_group_all_boxes: Some(bind_group_all_boxes),
            bind_group_all_boxes_outline_mask: Some(bind_group_all_boxes_outline_mask),
            batches: batches_internal,
        })
    }
}

pub struct Box3DRenderer {
    render_pipeline_color: GpuRenderPipelineHandle,
    render_pipeline_picking_layer: GpuRenderPipelineHandle,
    render_pipeline_outline_mask: GpuRenderPipelineHandle,
    bind_group_layout_all_boxes: GpuBindGroupLayoutHandle,
    bind_group_layout_batch: GpuBindGroupLayoutHandle,
}

impl Box3DRenderer {
    fn create_box_batch(
        &self,
        ctx: &RenderContext,
        label: DebugLabel,
        uniform_buffer_binding: BindGroupEntry,
        instance_range: Range<u32>,
        active_phases: EnumSet<DrawPhase>,
    ) -> Box3DBatch {
        let bind_group = ctx.gpu_resources.bind_groups.alloc(
            &ctx.device,
            &ctx.gpu_resources,
            &BindGroupDesc {
                label,
                entries: smallvec![uniform_buffer_binding],
                layout: self.bind_group_layout_batch,
            },
        );

        Box3DBatch {
            bind_group,
            vertex_range: (instance_range.start * VERTS_PER_BOX)
                ..(instance_range.end * VERTS_PER_BOX),
            active_phases,
        }
    }
}

impl Renderer for Box3DRenderer {
    type RendererDrawData = Box3DDrawData;

    fn create_renderer(ctx: &RenderContext) -> Self {
        re_tracing::profile_function!();

        let render_pipelines = &ctx.gpu_resources.render_pipelines;

        let float_tex = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uint_tex = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let bind_group_layout_all_boxes = ctx.gpu_resources.bind_group_layouts.get_or_create(
            &ctx.device,
            &BindGroupLayoutDesc {
                label: "Box3DRenderer::bind_group_layout_all_boxes".into(),
                entries: vec![
                    float_tex(0), // center_radius
                    float_tex(1), // half_size
                    float_tex(2), // quaternion
                    float_tex(3), // color (sRGB)
                    uint_tex(4),  // picking_instance_id
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(std::mem::size_of::<
                                gpu_data::DrawDataUniformBuffer,
                            >() as _),
                        },
                        count: None,
                    },
                ],
            },
        );

        let bind_group_layout_batch = ctx.gpu_resources.bind_group_layouts.get_or_create(
            &ctx.device,
            &BindGroupLayoutDesc {
                label: "Box3DRenderer::bind_group_layout_batch".into(),
                entries: vec![wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<
                            gpu_data::BatchUniformBuffer,
                        >() as _),
                    },
                    count: None,
                }],
            },
        );

        let pipeline_layout = ctx.gpu_resources.pipeline_layouts.get_or_create(
            ctx,
            &PipelineLayoutDesc {
                label: "Box3DRenderer::pipeline_layout".into(),
                entries: vec![
                    ctx.global_bindings.layout,
                    bind_group_layout_all_boxes,
                    bind_group_layout_batch,
                ],
            },
        );

        let shader_module_desc = include_shader_module!("../../shader/box3d.wgsl");
        let shader_module = ctx
            .gpu_resources
            .shader_modules
            .get_or_create(ctx, &shader_module_desc);

        let render_pipeline_desc_color = RenderPipelineDesc {
            label: "Box3DRenderer::render_pipeline_color".into(),
            pipeline_layout,
            vertex_entrypoint: "vs_main".into(),
            vertex_handle: shader_module,
            fragment_entrypoint: "fs_main".into(),
            fragment_handle: shader_module,
            vertex_buffers: smallvec![],
            render_targets: smallvec![Some(ViewBuilder::MAIN_TARGET_COLOR_FORMAT.into())],
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(ViewBuilder::MAIN_TARGET_DEFAULT_DEPTH_STATE),
            // Solid boxes fully cover their fragments — no alpha-to-coverage needed.
            multisample: ViewBuilder::main_target_default_msaa_state(ctx.render_config(), false),
        };
        let render_pipeline_color =
            render_pipelines.get_or_create(ctx, &render_pipeline_desc_color);
        let render_pipeline_picking_layer = render_pipelines.get_or_create(
            ctx,
            &RenderPipelineDesc {
                label: "Box3DRenderer::render_pipeline_picking_layer".into(),
                fragment_entrypoint: "fs_main_picking_layer".into(),
                render_targets: smallvec![Some(PickingLayerProcessor::PICKING_LAYER_FORMAT.into())],
                depth_stencil: PickingLayerProcessor::PICKING_LAYER_DEPTH_STATE,
                multisample: PickingLayerProcessor::PICKING_LAYER_MSAA_STATE,
                ..render_pipeline_desc_color.clone()
            },
        );
        let render_pipeline_outline_mask = render_pipelines.get_or_create(
            ctx,
            &RenderPipelineDesc {
                label: "Box3DRenderer::render_pipeline_outline_mask".into(),
                fragment_entrypoint: "fs_main_outline_mask".into(),
                render_targets: smallvec![Some(OutlineMaskProcessor::MASK_FORMAT.into())],
                depth_stencil: OutlineMaskProcessor::MASK_DEPTH_STATE,
                multisample: OutlineMaskProcessor::mask_default_msaa_state(ctx.device_caps().tier),
                ..render_pipeline_desc_color
            },
        );

        Self {
            render_pipeline_color,
            render_pipeline_picking_layer,
            render_pipeline_outline_mask,
            bind_group_layout_all_boxes,
            bind_group_layout_batch,
        }
    }

    fn draw(
        &self,
        render_pipelines: &GpuRenderPipelinePoolAccessor<'_>,
        phase: DrawPhase,
        pass: &mut wgpu::RenderPass<'_>,
        draw_instructions: &[DrawInstruction<'_, Self::RendererDrawData>],
    ) -> Result<(), DrawError> {
        let pipeline_handle = match phase {
            DrawPhase::OutlineMask => self.render_pipeline_outline_mask,
            DrawPhase::Opaque => self.render_pipeline_color,
            DrawPhase::PickingLayer => self.render_pipeline_picking_layer,
            _ => unreachable!("We were called on a phase we weren't subscribed to: {phase:?}"),
        };
        let pipeline = render_pipelines.get(pipeline_handle)?;

        pass.set_pipeline(pipeline);

        for DrawInstruction {
            draw_data,
            drawables,
        } in draw_instructions
        {
            let bind_group_all_boxes = match phase {
                DrawPhase::OutlineMask => &draw_data.bind_group_all_boxes_outline_mask,
                DrawPhase::Opaque | DrawPhase::PickingLayer => &draw_data.bind_group_all_boxes,
                _ => unreachable!("We were called on a phase we weren't subscribed to: {phase:?}"),
            };
            let Some(bind_group_all_boxes) = bind_group_all_boxes else {
                re_log::debug_panic!(
                    "Box3D data bind group for draw phase {phase:?} was not set despite being submitted for drawing."
                );
                continue;
            };
            pass.set_bind_group(1, bind_group_all_boxes, &[]);

            for drawable in *drawables {
                let batch = &draw_data.batches[drawable.draw_data_payload as usize];
                pass.set_bind_group(2, &batch.bind_group, &[]);
                pass.draw(batch.vertex_range.clone(), 0..1);
            }
        }

        Ok(())
    }
}
