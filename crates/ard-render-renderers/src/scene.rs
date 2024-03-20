use ard_ecs::prelude::*;
use ard_formats::mesh::IndexData;
use ard_math::Vec3A;
use ard_pal::prelude::{
    Buffer, BufferCreateInfo, BufferUsage, CommandBuffer, Context, MemoryUsage, QueueType,
    QueueTypes, RenderPass, SharingMode,
};
use ard_render_base::{ecs::Frame, resource::ResourceAllocator};
use ard_render_camera::ubo::CameraUbo;
use ard_render_material::{factory::MaterialFactory, material::MaterialResource};
use ard_render_meshes::{factory::MeshFactory, mesh::MeshResource};
use ard_render_objects::{
    objects::RenderObjects,
    set::{RenderableSet, RenderableSetUpdate},
};
use ard_render_si::{bindings::Layouts, types::GpuObjectId};
use ard_render_textures::factory::TextureFactory;
use std::ops::DerefMut;

use crate::{
    bins::{DrawBins, RenderArgs},
    calls::OutputDrawCalls,
    draw_gen::{DrawGenPipeline, DrawGenSets},
    highz::HzbImage,
    passes::{
        color::ColorPassSets, depth_only::DepthOnlyPassSets, depth_prepass::DepthPrepassSets, COLOR_ALPHA_CUTOFF_PASS_ID, COLOR_OPAQUE_PASS_ID, DEPTH_ALPHA_CUTOFF_PREPASS_PASS_ID, DEPTH_OPAQUE_PREPASS_PASS_ID, HIGH_Z_PASS_ID, TRANSPARENT_PASS_ID
    },
};

pub const DEFAULT_INPUT_ID_CAP: usize = 1;
pub const DEFAULT_OUTPUT_ID_CAP: usize = 1;

/// Primary GPU driven scene renderer.
#[derive(Resource)]
pub struct SceneRenderer {
    /// Object IDs which are filtered using the GPU driven frustum and occlusion culling compute
    /// shaders.
    input_ids: Buffer,
    /// IDs output from the culling computer shader to be bound when actual rendering is performed.
    output_ids: Buffer,
    /// Draw bins.
    bins: DrawBins,
    /// Draw calls.
    calls: OutputDrawCalls,
    /// Object information.
    set: RenderableSet,
    /// Set bindings for draw generation.
    draw_gen: DrawGenSets,
    /// Sets for rendering the HZB image.
    hzb_pass_sets: DepthOnlyPassSets,
    /// Sets for depth prepass rendering.
    depth_prepass_sets: DepthPrepassSets,
    /// Sets for color rendering.
    color_sets: ColorPassSets,
}

pub struct SceneRenderArgs<'a, 'b, const FIF: usize> {
    pub pass: &'b mut RenderPass<'a>,
    pub static_dirty: bool,
    pub camera: &'a CameraUbo,
    pub mesh_factory: &'a MeshFactory,
    pub material_factory: &'a MaterialFactory<FIF>,
    pub texture_factory: &'a TextureFactory,
    pub meshes: &'a ResourceAllocator<MeshResource, FIF>,
    pub materials: &'a ResourceAllocator<MaterialResource, FIF>,
}

impl SceneRenderer {
    pub fn new(
        ctx: &Context,
        layouts: &Layouts,
        draw_gen: &DrawGenPipeline,
        frames_in_flight: usize,
    ) -> Self {
        Self {
            input_ids: Buffer::new(
                ctx.clone(),
                BufferCreateInfo {
                    size: (DEFAULT_INPUT_ID_CAP * std::mem::size_of::<GpuObjectId>()) as u64,
                    array_elements: frames_in_flight,
                    buffer_usage: BufferUsage::STORAGE_BUFFER,
                    memory_usage: MemoryUsage::CpuToGpu,
                    queue_types: QueueTypes::COMPUTE,
                    sharing_mode: SharingMode::Exclusive,
                    debug_name: Some("input_ids".to_owned()),
                },
            )
            .unwrap(),
            output_ids: Buffer::new(
                ctx.clone(),
                BufferCreateInfo {
                    size: (DEFAULT_OUTPUT_ID_CAP * std::mem::size_of::<u32>()) as u64,
                    array_elements: 1,
                    buffer_usage: BufferUsage::STORAGE_BUFFER,
                    memory_usage: MemoryUsage::GpuOnly,
                    queue_types: QueueTypes::MAIN | QueueTypes::COMPUTE,
                    sharing_mode: SharingMode::Exclusive,
                    debug_name: Some("output_ids".to_owned()),
                },
            )
            .unwrap(),
            bins: DrawBins::new(ctx, frames_in_flight),
            calls: OutputDrawCalls::new(ctx, frames_in_flight),
            set: RenderableSet::default(),
            hzb_pass_sets: DepthOnlyPassSets::new(ctx, layouts, frames_in_flight),
            depth_prepass_sets: DepthPrepassSets::new(ctx, layouts, frames_in_flight),
            color_sets: ColorPassSets::new(ctx, layouts, frames_in_flight),
            draw_gen: DrawGenSets::new(draw_gen, true, frames_in_flight),
        }
    }

    #[inline(always)]
    pub fn draw_gen_sets(&self) -> &DrawGenSets {
        &self.draw_gen
    }

    #[inline(always)]
    pub fn color_pass_sets_mut(&mut self) -> &mut ColorPassSets {
        &mut self.color_sets
    }

    pub fn transfer_ownership<'a>(
        &'a self,
        frame: Frame,
        commands: &mut CommandBuffer<'a>,
        new_queue: QueueType,
    ) {
        // Don't transfer ownership unless we have valid draw calls to render, because if we don't
        // then the buffers are never actually acquired, and we'll end up with duplicate releases.
        if !self.bins.has_valid_draws(frame) {
            return;
        }

        self.calls
            .transfer_ownership(commands, frame, self.bins.use_alternate(frame), new_queue);
        commands.transfer_buffer_ownership(&self.output_ids, 0, new_queue, None);
    }

    pub fn upload<const FIF: usize>(
        &mut self,
        frame: Frame,
        objects: &RenderObjects,
        meshes: &ResourceAllocator<MeshResource, FIF>,
        materials: &ResourceAllocator<MaterialResource, FIF>,
        view_location: Vec3A,
    ) {
        puffin::profile_function!();

        // Update the set with all objects to render
        RenderableSetUpdate::new(&mut self.set)
            .with_opaque()
            .with_alpha_cutout()
            .with_transparent()
            .update(view_location, objects, |_| true, |_| true, |_| true);

        // Expand ID buffers if needed
        let input_id_buffer_size = std::mem::size_of_val(self.set.ids()) as u64;
        let input_id_buffer_expanded =
            match Buffer::expand(&self.input_ids, input_id_buffer_size, false) {
                Some(mut new_buffer) => {
                    std::mem::swap(&mut self.input_ids, &mut new_buffer);
                    true
                }
                None => false,
            };

        let output_id_buffer_size = (self.set.ids().len() * std::mem::size_of::<u32>()) as u64;
        if let Some(mut new_buffer) = Buffer::expand(&self.output_ids, output_id_buffer_size, false)
        {
            std::mem::swap(&mut self.output_ids, &mut new_buffer);
        }

        // Write in object IDs
        let mut id_view = self.input_ids.write(usize::from(frame)).unwrap();
        let id_slice = bytemuck::cast_slice_mut::<_, GpuObjectId>(id_view.deref_mut());

        // Write in static ids if they were modified
        if input_id_buffer_expanded || objects.static_dirty() {
            id_slice[self.set.static_object_ranges().opaque.clone()]
                .copy_from_slice(&self.set.ids()[self.set.static_object_ranges().opaque.clone()]);
            id_slice[self.set.static_object_ranges().alpha_cutout.clone()].copy_from_slice(
                &self.set.ids()[self.set.static_object_ranges().alpha_cutout.clone()],
            );
        }

        // Write in dynamic object IDs
        id_slice[self.set.dynamic_object_ranges().opaque.clone()]
            .copy_from_slice(&self.set.ids()[self.set.dynamic_object_ranges().opaque.clone()]);
        id_slice[self.set.dynamic_object_ranges().alpha_cutout.clone()].copy_from_slice(
            &self.set.ids()[self.set.dynamic_object_ranges().alpha_cutout.clone()],
        );

        // Write in transparent object IDs
        id_slice[self.set.transparent_object_range().clone()]
            .copy_from_slice(&self.set.ids()[self.set.transparent_object_range().clone()]);

        // Generate bins
        let non_transparent_count = self.set.static_group_ranges().opaque.len()
            + self.set.static_group_ranges().alpha_cutout.len()
            + self.set.dynamic_group_ranges().opaque.len()
            + self.set.dynamic_group_ranges().alpha_cutout.len();

        self.bins
            .preallocate_draw_group_buffers(self.set.groups().len());

        self.bins.gen_bins(
            frame,
            self.set.groups()[self.set.static_group_ranges().opaque.clone()].iter(),
            self.set.groups()[self.set.static_group_ranges().alpha_cutout.clone()].iter(),
            self.set.groups()[self.set.dynamic_group_ranges().opaque.clone()].iter(),
            self.set.groups()[self.set.dynamic_group_ranges().alpha_cutout.clone()].iter(),
            self.set.groups().iter().skip(non_transparent_count),
            meshes,
            materials,
        );

        self.calls.preallocate(self.set.groups().len());
        self.calls
            .upload_counts(self.bins.bins(frame), frame, self.bins.use_alternate(frame));
    }

    pub fn update_bindings<const FIF: usize>(
        &mut self,
        frame: Frame,
        objects: &RenderObjects,
        hzb_image: &HzbImage<FIF>,
        meshes: &MeshFactory,
    ) {
        self.hzb_pass_sets.update_object_data_bindings(
            frame,
            objects.object_data(),
            &self.output_ids,
        );

        self.depth_prepass_sets.update_object_data_bindings(
            frame,
            objects.object_data(),
            &self.output_ids,
        );

        self.color_sets
            .update_object_data_bindings(frame, objects.object_data(), &self.output_ids);

        self.draw_gen.update_bindings(
            frame,
            self.set.ids().len(),
            self.set.non_transparent_object_count(),
            self.set.groups().len(),
            self.set.non_transparent_draw_count(),
            self.bins.draw_groups_buffer(frame),
            self.calls
                .instance_count_buffer(frame, self.bins.use_alternate(frame)),
            self.calls
                .draw_call_buffer(frame, self.bins.use_alternate(frame)),
            self.calls
                .draw_counts_buffer(frame, self.bins.use_alternate(frame)),
            objects,
            Some(hzb_image),
            &self.input_ids,
            &self.output_ids,
            meshes.mesh_info_buffer(),
        );
    }

    pub fn render_hzb<'a, const FIF: usize>(
        &'a self,
        frame: Frame,
        args: SceneRenderArgs<'a, '_, FIF>,
    ) {
        args.pass
            .bind_index_buffer(args.mesh_factory.index_buffer(), 0, 0, IndexData::TYPE);

        // Render static opaque geometry
        self.bins.render_static_opaque_bins(true, RenderArgs {
            pass_id: HIGH_Z_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.hzb_pass_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });
    }

    pub fn render_depth_prepass<'a, const FIF: usize>(
        &'a self,
        frame: Frame,
        args: SceneRenderArgs<'a, '_, FIF>,
    ) {
        args.pass
            .bind_index_buffer(args.mesh_factory.index_buffer(), 0, 0, IndexData::TYPE);

        // Render opaque and alpha cut objects
        self.bins.render_static_opaque_bins(false, RenderArgs {
            pass_id: DEPTH_OPAQUE_PREPASS_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.depth_prepass_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_dynamic_opaque_bins(RenderArgs {
            pass_id: DEPTH_OPAQUE_PREPASS_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.depth_prepass_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_static_alpha_cutoff_bins(RenderArgs {
            pass_id: DEPTH_ALPHA_CUTOFF_PREPASS_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.depth_prepass_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_dynamic_alpha_cutoff_bins(RenderArgs {
            pass_id: DEPTH_ALPHA_CUTOFF_PREPASS_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.depth_prepass_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });
    }

    pub fn render_opaque<'a, const FIF: usize>(
        &'a self,
        frame: Frame,
        args: SceneRenderArgs<'a, '_, FIF>,
    ) {
        args.pass
            .bind_index_buffer(args.mesh_factory.index_buffer(), 0, 0, IndexData::TYPE);

        self.bins.render_static_opaque_bins(false, RenderArgs {
            pass_id: COLOR_OPAQUE_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.color_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_dynamic_opaque_bins(RenderArgs {
            pass_id: COLOR_OPAQUE_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.color_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_static_alpha_cutoff_bins(RenderArgs {
            pass_id: COLOR_ALPHA_CUTOFF_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.color_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });

        self.bins.render_dynamic_alpha_cutoff_bins(RenderArgs {
            pass_id: COLOR_ALPHA_CUTOFF_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.color_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });
    }

    pub fn render_transparent<'a, const FIF: usize>(
        &'a self,
        frame: Frame,
        args: SceneRenderArgs<'a, '_, FIF>,
    ) {
        args.pass
            .bind_index_buffer(args.mesh_factory.index_buffer(), 0, 0, IndexData::TYPE);

        self.bins.render_transparent_bins(RenderArgs {
            pass_id: TRANSPARENT_PASS_ID,
            frame,
            camera: args.camera,
            global_set: self.color_sets.get_set(frame),
            pass: args.pass,
            calls: &self.calls,
            mesh_factory: args.mesh_factory,
            material_factory: args.material_factory,
            texture_factory: args.texture_factory,
            meshes: args.meshes,
            materials: args.materials,
        });
    }
}
