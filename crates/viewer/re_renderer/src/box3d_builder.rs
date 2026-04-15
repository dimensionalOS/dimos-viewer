//! Builder for [`crate::renderer::Box3DDrawData`].
//!
//! Accumulates per-instance data for solid 3D boxes across one or more batches and turns
//! them into `DataTextureSource`-backed GPU textures that the shader reads from.

use itertools::izip;
use re_log::{ResultExt as _, debug_assert_eq};

use crate::allocator::DataTextureSource;
use crate::draw_phases::PickingLayerObjectId;
use crate::renderer::gpu_data::{CenterRadius, HalfSize, Quaternion};
use crate::renderer::{Box3DBatchFlags, Box3DBatchInfo, Box3DDrawData, Box3DDrawDataError};
use crate::{
    Color32, CpuWriteGpuReadError, DebugLabel, DepthOffset, OutlineMaskPreference,
    PickingLayerInstanceId, RenderContext, Size,
};

/// Builder for solid 3D boxes.
pub struct Box3DBuilder<'ctx> {
    pub(crate) ctx: &'ctx RenderContext,

    pub(crate) center_radius_buffer: DataTextureSource<'ctx, CenterRadius>,
    pub(crate) half_size_buffer: DataTextureSource<'ctx, HalfSize>,
    pub(crate) quaternion_buffer: DataTextureSource<'ctx, Quaternion>,

    pub(crate) color_buffer: DataTextureSource<'ctx, Color32>,
    pub(crate) picking_instance_ids_buffer: DataTextureSource<'ctx, PickingLayerInstanceId>,

    pub(crate) batches: Vec<Box3DBatchInfo>,

    pub(crate) radius_boost_in_ui_points_for_outlines: f32,
}

impl<'ctx> Box3DBuilder<'ctx> {
    pub fn new(ctx: &'ctx RenderContext) -> Self {
        Self {
            ctx,
            center_radius_buffer: DataTextureSource::new(ctx),
            half_size_buffer: DataTextureSource::new(ctx),
            quaternion_buffer: DataTextureSource::new(ctx),
            color_buffer: DataTextureSource::new(ctx),
            picking_instance_ids_buffer: DataTextureSource::new(ctx),
            batches: Vec::with_capacity(16),
            radius_boost_in_ui_points_for_outlines: 0.0,
        }
    }

    /// Reserve capacity for an additional number of boxes.
    ///
    /// Returns the number that can actually be added without hitting the data-texture limit.
    pub fn reserve(
        &mut self,
        expected_number_of_additional_boxes: usize,
    ) -> Result<usize, CpuWriteGpuReadError> {
        self.center_radius_buffer
            .reserve(expected_number_of_additional_boxes)?;
        self.half_size_buffer
            .reserve(expected_number_of_additional_boxes)?;
        self.quaternion_buffer
            .reserve(expected_number_of_additional_boxes)?;
        self.color_buffer
            .reserve(expected_number_of_additional_boxes)?;
        self.picking_instance_ids_buffer
            .reserve(expected_number_of_additional_boxes)
    }

    /// Outline radius boost (UI points) for the outline-mask pass.
    pub fn radius_boost_in_ui_points_for_outlines(
        &mut self,
        radius_boost_in_ui_points_for_outlines: f32,
    ) {
        self.radius_boost_in_ui_points_for_outlines = radius_boost_in_ui_points_for_outlines;
    }

    /// Start a new batch.
    #[inline]
    pub fn batch(&mut self, label: impl Into<DebugLabel>) -> Box3DBatchBuilder<'_, 'ctx> {
        self.batches.push(Box3DBatchInfo {
            label: label.into(),
            ..Box3DBatchInfo::default()
        });
        Box3DBatchBuilder(self)
    }

    #[inline]
    pub fn batch_with_info(&mut self, info: Box3DBatchInfo) -> Box3DBatchBuilder<'_, 'ctx> {
        self.batches.push(info);
        Box3DBatchBuilder(self)
    }

    /// Finalize into draw data.
    pub fn into_draw_data(self) -> Result<Box3DDrawData, Box3DDrawDataError> {
        Box3DDrawData::new(self)
    }
}

pub struct Box3DBatchBuilder<'a, 'ctx>(&'a mut Box3DBuilder<'ctx>);

impl Drop for Box3DBatchBuilder<'_, '_> {
    fn drop(&mut self) {
        // Remove unused empty batch.
        if self.0.batches.last().unwrap().box_count == 0 {
            self.0.batches.pop();
        }
    }
}

impl Box3DBatchBuilder<'_, '_> {
    #[inline]
    fn batch_mut(&mut self) -> &mut Box3DBatchInfo {
        self.0
            .batches
            .last_mut()
            .expect("batch should have been added on Box3DBatchBuilder creation")
    }

    #[inline]
    pub fn world_from_obj(mut self, world_from_obj: glam::Affine3A) -> Self {
        self.batch_mut().world_from_obj = world_from_obj;
        self
    }

    #[inline]
    pub fn outline_mask_ids(mut self, outline_mask_ids: OutlineMaskPreference) -> Self {
        self.batch_mut().overall_outline_mask_ids = outline_mask_ids;
        self
    }

    #[inline]
    pub fn depth_offset(mut self, depth_offset: DepthOffset) -> Self {
        self.batch_mut().depth_offset = depth_offset;
        self
    }

    #[inline]
    pub fn flags(mut self, flags: Box3DBatchFlags) -> Self {
        self.batch_mut().flags |= flags;
        self
    }

    #[inline]
    pub fn picking_object_id(mut self, picking_object_id: PickingLayerObjectId) -> Self {
        self.batch_mut().picking_object_id = picking_object_id;
        self
    }

    /// Per-range outline mask override. Range is in boxes (instances), batch-local.
    #[inline]
    pub fn push_additional_outline_mask_ids_for_range(
        mut self,
        range: std::ops::Range<u32>,
        ids: OutlineMaskPreference,
    ) -> Self {
        self.batch_mut()
            .additional_outline_mask_ids_instance_ranges
            .push((range, ids));
        self
    }

    /// Add a batch of solid 3D boxes.
    ///
    /// All slices except `centers`, `half_sizes`, and `quats` may be shorter than the number
    /// of boxes; the last value is repeated (or a sensible default is used).
    #[inline]
    pub fn add_boxes(
        mut self,
        centers: &[glam::Vec3],
        half_sizes: &[glam::Vec3],
        quats: &[glam::Vec4],
        colors: &[Color32],
        picking_ids: &[PickingLayerInstanceId],
        radii: &[Size],
    ) -> Self {
        re_tracing::profile_function!();

        debug_assert_eq!(
            self.0.center_radius_buffer.len(),
            self.0.half_size_buffer.len()
        );
        debug_assert_eq!(
            self.0.center_radius_buffer.len(),
            self.0.quaternion_buffer.len()
        );
        debug_assert_eq!(self.0.center_radius_buffer.len(), self.0.color_buffer.len());
        debug_assert_eq!(
            self.0.center_radius_buffer.len(),
            self.0.picking_instance_ids_buffer.len()
        );

        let Some(num_available) = self
            .0
            .center_radius_buffer
            .reserve(centers.len())
            .ok_or_log_error()
        else {
            return self;
        };

        let num_boxes = if centers.len() > num_available {
            re_log::error_once!(
                "Reached maximum number of boxes for box batch ({}). Ignoring excess.",
                self.0.center_radius_buffer.len() + num_available
            );
            num_available
        } else {
            centers.len()
        };

        if num_boxes == 0 {
            return self;
        }

        // Truncate to num_boxes — slices may be longer or shorter than `centers`.
        let centers = &centers[0..num_boxes];
        let half_sizes_len = half_sizes.len().min(num_boxes);
        let quats_len = quats.len().min(num_boxes);
        let half_sizes = &half_sizes[0..half_sizes_len];
        let quats = &quats[0..quats_len];
        let colors = &colors[0..colors.len().min(num_boxes)];
        let picking_ids = &picking_ids[0..picking_ids.len().min(num_boxes)];
        let radii = &radii[0..radii.len().min(num_boxes)];

        self.batch_mut().box_count += num_boxes as u32;

        // Center + radius.
        {
            re_tracing::profile_scope!("centers & radii");
            let default_radius = radii.last().copied().unwrap_or(Size::ONE_UI_POINT);
            let rows: Vec<CenterRadius> = izip!(
                centers.iter().copied(),
                radii
                    .iter()
                    .copied()
                    .chain(std::iter::repeat(default_radius)),
            )
            .map(|(center, radius)| CenterRadius { center, radius })
            .collect();
            self.0
                .center_radius_buffer
                .extend_from_slice(&rows)
                .ok_or_log_error();
        }

        // Half sizes.
        {
            re_tracing::profile_scope!("half_sizes");
            let default_half = half_sizes.last().copied().unwrap_or(glam::Vec3::splat(0.5));
            let rows: Vec<HalfSize> = half_sizes
                .iter()
                .copied()
                .chain(std::iter::repeat(default_half))
                .take(num_boxes)
                .map(|half_size| HalfSize {
                    half_size,
                    _pad: 0.0,
                })
                .collect();
            self.0
                .half_size_buffer
                .extend_from_slice(&rows)
                .ok_or_log_error();
        }

        // Quaternions — default to identity (0,0,0,1).
        {
            re_tracing::profile_scope!("quaternions");
            let default_quat = glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
            let rows: Vec<Quaternion> = quats
                .iter()
                .copied()
                .chain(std::iter::repeat(default_quat))
                .take(num_boxes)
                .map(|q| Quaternion { q })
                .collect();
            self.0
                .quaternion_buffer
                .extend_from_slice(&rows)
                .ok_or_log_error();
        }

        // Colors — default to white.
        {
            re_tracing::profile_scope!("colors");
            self.0
                .color_buffer
                .extend_from_slice(colors)
                .ok_or_log_error();
            self.0
                .color_buffer
                .add_n(Color32::WHITE, num_boxes.saturating_sub(colors.len()))
                .ok_or_log_error();
        }

        // Picking ids — default to 0.
        {
            re_tracing::profile_scope!("picking_ids");
            self.0
                .picking_instance_ids_buffer
                .extend_from_slice(picking_ids)
                .ok_or_log_error();
            self.0
                .picking_instance_ids_buffer
                .add_n(
                    PickingLayerInstanceId::default(),
                    num_boxes.saturating_sub(picking_ids.len()),
                )
                .ok_or_log_error();
        }

        self
    }
}
