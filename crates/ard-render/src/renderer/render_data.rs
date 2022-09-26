use std::ops::DerefMut;

use ard_core::prelude::Disabled;
use ard_ecs::prelude::*;
use ard_math::{Mat4, Vec2};
use ard_pal::prelude::*;
use bytemuck::{Pod, Zeroable};
use ordered_float::NotNan;

use crate::{
    camera::CameraUbo,
    factory::{
        allocator::{ResourceAllocator, ResourceId},
        materials::MaterialBuffers,
        meshes::MeshBuffers,
        textures::TextureSets,
        Factory,
    },
    material::{MaterialInner, MaterialInstance, PipelineType},
    mesh::{Mesh, MeshInner, ObjectBounds, VertexLayout},
    shader_constants::FRAMES_IN_FLIGHT,
    static_geometry::StaticGeometryInner,
};

use super::{occlusion::HzbImage, Model, RenderLayer, RenderQuery, Renderable};

const DEFAULT_OBJECT_DATA_CAP: u64 = 1;
const DEFAULT_INPUT_ID_CAP: u64 = 1;
const DEFAULT_OUTPUT_ID_CAP: u64 = 1;
const DEFAULT_DRAW_CALL_CAP: u64 = 1;

const GLOBAL_OBJECT_DATA_BINDING: u32 = 0;
const GLOBAL_OBJECT_ID_BINDING: u32 = 1;

const DRAW_GEN_DRAW_CALLS_BINDING: u32 = 0;
const DRAW_GEN_OBJECT_DATA_BINDING: u32 = 1;
const DRAW_GEN_INPUT_ID_BINDING: u32 = 2;
const DRAW_GEN_OUTPUT_ID_BINDING: u32 = 3;
const DRAW_GEN_CAMERA_BINDING: u32 = 4;
const DRAW_GEN_HZB_BINDING: u32 = 5;

const CAMERA_UBO_BINDING: u32 = 0;

const DRAW_GEN_WORKGROUP_SIZE: u32 = 256;

pub(crate) struct GlobalRenderData {
    /// Layout for global rendering data.
    pub global_layout: DescriptorSetLayout,
    /// Layout for draw call generation.
    pub draw_gen_layout: DescriptorSetLayout,
    /// Layout for camera view information.
    pub camera_layout: DescriptorSetLayout,
    /// Pipeline to perform draw call generation.
    pub draw_gen_pipeline: ComputePipeline,
    /// Contains all object data.
    pub object_data: Buffer,
}

pub(crate) struct RenderData {
    /// Global rendering data descriptor sets. A pair of them is stored to swap every frame for use
    /// in occlusion culling.
    pub global_sets: Vec<[DescriptorSet; 2]>,
    /// Sets for the camera UBO.
    pub camera_sets: Vec<DescriptorSet>,
    /// Draw generation descriptor sets.
    pub draw_gen_sets: Vec<DescriptorSet>,
    /// Draw keys used during render sorting. Holds the key and number of objects that use the key.
    pub keys: [Vec<(DrawKey, usize)>; FRAMES_IN_FLIGHT],
    /// UBO for camera data.
    pub camera_ubo: Buffer,
    /// Contains input IDs that the GPU will parse in draw generation.
    pub input_ids: Buffer,
    /// Generated by the GPU. Contains the indices into the primary object info array for all
    /// objects to render.
    pub output_ids: Buffer,
    /// Generated by the GPU. Contains indirect draw calls to perform.
    pub draw_calls: Buffer,
    /// Scratch space to hold dynamic input ids for sorting.
    dynamic_input_ids: Vec<InputObjectId>,
    /// The number of static objects detected for rendering.
    pub static_objects: usize,
    /// The number of dynamic objects detected for rendering.
    pub dynamic_objects: usize,
    /// The number of static draw calls from last frame.
    pub last_static_draws: usize,
    /// The total number of static draw calls.
    pub static_draws: usize,
    /// The total number of dynamic draw calls.
    pub dynamic_draws: usize,
}

pub(crate) struct RenderArgs<'a, 'b> {
    pub pass: &'b mut RenderPass<'a>,
    pub texture_sets: &'a TextureSets,
    pub material_buffers: &'a MaterialBuffers,
    pub mesh_buffers: &'a MeshBuffers,
    pub materials: &'a ResourceAllocator<MaterialInner>,
    pub meshes: &'a ResourceAllocator<MeshInner>,
    pub pipeline_ty: PipelineType,
    pub draw_offset: usize,
    pub draw_count: usize,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct DrawGenPushConstants {
    render_area: Vec2,
    object_count: u32,
}

/// Information to draw an object.
#[repr(C, align(16))]
#[derive(Copy, Clone)]
struct ObjectData {
    /// Model matrix of the object.
    pub model: Mat4,
    /// Index into the material buffer for this objects material. `NO_MATERIAL` if none.
    pub material: u32,
    /// Index into the textures buffer for this objects material. `NO_TEXTURES` if none.
    pub textures: u32,
    /// ID of the entity of this object.
    pub entity_id: u32,
    /// Version of the entity of this object.
    pub entity_ver: u32,
}

/// Draw calls are unique to each material/mesh combo.
#[repr(C)]
#[derive(Copy, Clone)]
struct DrawCall {
    /// Draw call for this object type.
    pub indirect: DrawIndexedIndirect,
    /// Object bounds for the mesh.
    pub bounds: ObjectBounds,
}

/// An object to be processed during draw call generations.
#[repr(C, align(16))]
#[derive(Copy, Clone)]
struct InputObjectId {
    /// Index into the `draw_calls` buffer of `RenderData` for what draw call this object belongs
    /// to.
    ///
    /// ## Note
    /// You might be wondering why this is an array. Well, in order to generate dynamic draw calls
    /// we need to sort all the objects by their draw key and then compact duplicates into single
    /// draws. In order to do this, all the objects must know what their batch index is "before we
    /// actually generate them" (this is mostly for performance reasons). With static objects it
    /// isn't an issue because they are already sorted. For dynamic objects we must sort them
    /// ourselves. To do this, we use this field to hold the draw key. Since the draw key is a
    /// 64-bit number, we need two u32 fields to hold it.
    pub draw_idx: [u32; 2],
    /// Index into the `object_data` buffer of `GlobalRenderData` for this object.
    pub data_idx: u32,
}

/// Index into the `object_data` buffer in `GlobalRenderData`.
pub type OutputObjectId = u32;

/// Used to sort draw calls.
pub type DrawKey = u64;

unsafe impl Zeroable for ObjectData {}
unsafe impl Pod for ObjectData {}

unsafe impl Zeroable for DrawCall {}
unsafe impl Pod for DrawCall {}

unsafe impl Zeroable for InputObjectId {}
unsafe impl Pod for InputObjectId {}

unsafe impl Zeroable for DrawGenPushConstants {}
unsafe impl Pod for DrawGenPushConstants {}

impl GlobalRenderData {
    pub fn new(ctx: &Context) -> Self {
        let object_data = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: std::mem::size_of::<ObjectData>() as u64 * DEFAULT_OBJECT_DATA_CAP,
                array_elements: FRAMES_IN_FLIGHT,
                buffer_usage: BufferUsage::STORAGE_BUFFER,
                memory_usage: MemoryUsage::CpuToGpu,
                debug_name: Some(String::from("object_data")),
            },
        )
        .unwrap();

        let camera_layout = DescriptorSetLayout::new(
            ctx.clone(),
            DescriptorSetLayoutCreateInfo {
                bindings: vec![
                    // UBO
                    DescriptorBinding {
                        binding: CAMERA_UBO_BINDING,
                        ty: DescriptorType::UniformBuffer,
                        count: 1,
                        stage: ShaderStage::AllGraphics,
                    },
                ],
            },
        )
        .unwrap();

        let global_layout = DescriptorSetLayout::new(
            ctx.clone(),
            DescriptorSetLayoutCreateInfo {
                bindings: vec![
                    // Object data
                    DescriptorBinding {
                        binding: GLOBAL_OBJECT_DATA_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::Read),
                        count: 1,
                        stage: ShaderStage::AllGraphics,
                    },
                    // Object IDs
                    DescriptorBinding {
                        binding: GLOBAL_OBJECT_ID_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::Read),
                        count: 1,
                        stage: ShaderStage::AllGraphics,
                    },
                ],
            },
        )
        .unwrap();

        let draw_gen_layout = DescriptorSetLayout::new(
            ctx.clone(),
            DescriptorSetLayoutCreateInfo {
                bindings: vec![
                    // Draw calls
                    DescriptorBinding {
                        binding: DRAW_GEN_DRAW_CALLS_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::ReadWrite),
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                    // Object data
                    DescriptorBinding {
                        binding: DRAW_GEN_OBJECT_DATA_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::Read),
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                    // Input IDs
                    DescriptorBinding {
                        binding: DRAW_GEN_INPUT_ID_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::Read),
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                    // Output IDs
                    DescriptorBinding {
                        binding: DRAW_GEN_OUTPUT_ID_BINDING,
                        ty: DescriptorType::StorageBuffer(AccessType::ReadWrite),
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                    // Camera
                    DescriptorBinding {
                        binding: DRAW_GEN_CAMERA_BINDING,
                        ty: DescriptorType::UniformBuffer,
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                    // HZB image
                    DescriptorBinding {
                        binding: DRAW_GEN_HZB_BINDING,
                        ty: DescriptorType::Texture,
                        count: 1,
                        stage: ShaderStage::Compute,
                    },
                ],
            },
        )
        .unwrap();

        let draw_gen_shader = Shader::new(
            ctx.clone(),
            ShaderCreateInfo {
                code: include_bytes!("../shaders/draw_gen.comp.spv"),
                debug_name: Some(String::from("draw_gen_shader")),
            },
        )
        .unwrap();

        let draw_gen_pipeline = ComputePipeline::new(
            ctx.clone(),
            ComputePipelineCreateInfo {
                layouts: vec![draw_gen_layout.clone()],
                module: draw_gen_shader,
                work_group_size: (DRAW_GEN_WORKGROUP_SIZE, 1, 1),
                push_constants_size: Some(std::mem::size_of::<DrawGenPushConstants>() as u32),
                debug_name: Some(String::from("draw_gen_pipeline")),
            },
        )
        .unwrap();

        Self {
            global_layout,
            draw_gen_layout,
            camera_layout,
            draw_gen_pipeline,
            object_data,
        }
    }

    /// Writes all possibly rendered objects into the global buffer. If the buffer was reszied,
    /// `true` is returned.
    pub fn prepare_object_data(
        &mut self,
        frame: usize,
        factory: &Factory,
        queries: &Queries<RenderQuery>,
        static_geometry: &StaticGeometryInner,
    ) -> bool {
        let query = queries.make::<(Entity, (Read<Renderable>, Read<Model>))>();
        let materials = factory.0.material_instances.lock().unwrap();

        // Expand object data buffer if required
        let obj_count = query.len() + static_geometry.len;
        let expanded = match Buffer::expand(
            &mut self.object_data,
            (obj_count * std::mem::size_of::<ObjectData>()) as u64,
            false,
        ) {
            Some(buffer) => {
                self.object_data = buffer;
                true
            }
            None => true,
        };

        // Write in every object
        let mut view = self.object_data.write(frame).unwrap();
        let slice = bytemuck::cast_slice_mut::<_, ObjectData>(&mut view);

        // Write in static geometry if it's dirty
        if static_geometry.dirty[frame] {
            let mut cur_offset = 0;
            for key in &static_geometry.sorted_keys {
                let batch = static_geometry.batches.get(key).unwrap();
                let material = materials.get(batch.renderable.material.id).unwrap();
                for i in 0..batch.ids.len() {
                    let entity = batch.entities[i];
                    slice[cur_offset] = ObjectData {
                        model: batch.models[i],
                        material: material
                            .material_block
                            .map(|block| block.into())
                            .unwrap_or(0),
                        textures: material
                            .texture_block
                            .map(|block| block.into())
                            .unwrap_or(0),
                        entity_id: entity.id(),
                        entity_ver: entity.ver(),
                    };
                    cur_offset += 1;
                }
            }
        }

        // Write in dynamic geometry
        for (i, (entity, (renderable, model))) in query.into_iter().enumerate() {
            let material = materials.get(renderable.material.id).unwrap();
            slice[i + static_geometry.len] = ObjectData {
                model: model.0,
                material: material
                    .material_block
                    .map(|block| block.into())
                    .unwrap_or(0),
                textures: material
                    .texture_block
                    .map(|block| block.into())
                    .unwrap_or(0),
                entity_id: entity.id(),
                entity_ver: entity.ver(),
            };
        }

        expanded
    }
}

impl RenderData {
    pub fn new(
        ctx: &Context,
        name: &str,
        global_layout: &DescriptorSetLayout,
        draw_gen_layout: &DescriptorSetLayout,
        camera_layout: &DescriptorSetLayout,
    ) -> Self {
        // Create buffers
        let camera_ubo = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: std::mem::size_of::<CameraUbo>() as u64,
                array_elements: FRAMES_IN_FLIGHT,
                buffer_usage: BufferUsage::UNIFORM_BUFFER,
                memory_usage: MemoryUsage::CpuToGpu,
                debug_name: Some(String::from("camera_ubo")),
            },
        )
        .unwrap();

        let input_ids = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: std::mem::size_of::<InputObjectId>() as u64 * DEFAULT_INPUT_ID_CAP,
                array_elements: FRAMES_IN_FLIGHT,
                buffer_usage: BufferUsage::STORAGE_BUFFER,
                memory_usage: MemoryUsage::CpuToGpu,
                debug_name: Some(String::from("input_ids")),
            },
        )
        .unwrap();

        let output_ids = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: std::mem::size_of::<OutputObjectId>() as u64 * DEFAULT_OUTPUT_ID_CAP,
                array_elements: FRAMES_IN_FLIGHT,
                buffer_usage: BufferUsage::STORAGE_BUFFER,
                memory_usage: MemoryUsage::GpuOnly,
                debug_name: Some(String::from("output_ids")),
            },
        )
        .unwrap();

        let draw_calls = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: std::mem::size_of::<DrawCall>() as u64 * DEFAULT_DRAW_CALL_CAP,
                // We need two draw call buffers per frame in flight because we alternate between
                // them for use in occlusion culling.
                array_elements: FRAMES_IN_FLIGHT * 2,
                buffer_usage: BufferUsage::STORAGE_BUFFER | BufferUsage::INDIRECT_BUFFER,
                memory_usage: MemoryUsage::CpuToGpu,
                debug_name: Some(String::from("draw_calls")),
            },
        )
        .unwrap();

        // Create global sets
        let mut global_sets = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for frame in 0..FRAMES_IN_FLIGHT {
            let set_a = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: global_layout.clone(),
                    debug_name: Some(format!("{name}_global_set_a_{frame}")),
                },
            )
            .unwrap();
            let set_b = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: global_layout.clone(),
                    debug_name: Some(format!("{name}_global_set_b_{frame}")),
                },
            )
            .unwrap();
            global_sets.push([set_a, set_b]);
        }

        // Create draw generation sets
        let mut draw_gen_sets = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for frame in 0..FRAMES_IN_FLIGHT {
            let set = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: draw_gen_layout.clone(),
                    debug_name: Some(format!("{name}_draw_gen_set_{frame}")),
                },
            )
            .unwrap();
            draw_gen_sets.push(set);
        }

        // Create camera sets
        let mut camera_sets = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for frame in 0..FRAMES_IN_FLIGHT {
            let mut set = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: camera_layout.clone(),
                    debug_name: Some(format!("{name}_camera_set_{frame}")),
                },
            )
            .unwrap();

            set.update(&[DescriptorSetUpdate {
                binding: CAMERA_UBO_BINDING,
                array_element: 0,
                value: DescriptorValue::UniformBuffer {
                    buffer: &camera_ubo,
                    array_element: frame,
                },
            }]);

            camera_sets.push(set);
        }

        let out = Self {
            global_sets,
            draw_gen_sets,
            camera_sets,
            keys: Default::default(),
            camera_ubo,
            input_ids,
            output_ids,
            draw_calls,
            dynamic_input_ids: Vec::default(),
            static_objects: 0,
            dynamic_objects: 0,
            last_static_draws: 0,
            static_draws: 0,
            dynamic_draws: 0,
        };

        out
    }

    /// Rebinds all draw generation descriptor set values.
    pub fn update_draw_gen_set(
        &mut self,
        global: &GlobalRenderData,
        hzb: &HzbImage,
        frame: usize,
        use_alternate: bool,
    ) {
        let alternate_frame = (frame * 2) + use_alternate as usize;
        let set = &mut self.draw_gen_sets[frame];
        let hzb_tex = hzb.texture();
        set.update(&[
            DescriptorSetUpdate {
                binding: DRAW_GEN_DRAW_CALLS_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &self.draw_calls,
                    array_element: alternate_frame,
                },
            },
            DescriptorSetUpdate {
                binding: DRAW_GEN_OBJECT_DATA_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &global.object_data,
                    array_element: frame,
                },
            },
            DescriptorSetUpdate {
                binding: DRAW_GEN_INPUT_ID_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &self.input_ids,
                    array_element: frame,
                },
            },
            DescriptorSetUpdate {
                binding: DRAW_GEN_OUTPUT_ID_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &self.output_ids,
                    array_element: frame,
                },
            },
            DescriptorSetUpdate {
                binding: DRAW_GEN_CAMERA_BINDING,
                array_element: 0,
                value: DescriptorValue::UniformBuffer {
                    buffer: &self.camera_ubo,
                    array_element: frame,
                },
            },
            DescriptorSetUpdate {
                binding: DRAW_GEN_HZB_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: hzb_tex,
                    array_element: frame,
                    sampler: Sampler {
                        min_filter: Filter::Nearest,
                        mag_filter: Filter::Nearest,
                        mipmap_filter: Filter::Nearest,
                        address_u: SamplerAddressMode::ClampToEdge,
                        address_v: SamplerAddressMode::ClampToEdge,
                        address_w: SamplerAddressMode::ClampToEdge,
                        anisotropy: None,
                        compare: None,
                        min_lod: NotNan::new(0.0).unwrap(),
                        max_lod: None,
                        unnormalize_coords: false,
                    },
                    base_mip: 0,
                    mip_count: hzb_tex.mip_count(),
                },
            },
        ]);
    }

    /// Rebinds all global descriptor set values.
    pub fn update_global_set(&mut self, global: &GlobalRenderData, frame: usize) {
        for i in 0..2 {
            let set = &mut self.global_sets[frame][i];
            set.update(&[
                DescriptorSetUpdate {
                    binding: GLOBAL_OBJECT_DATA_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &global.object_data,
                        array_element: frame,
                    },
                },
                DescriptorSetUpdate {
                    binding: GLOBAL_OBJECT_ID_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &self.output_ids,
                        array_element: frame,
                    },
                },
            ]);
        }
    }

    /// Updates the camera UBO.
    #[inline]
    pub fn update_camera_ubo(&mut self, frame: usize, data: CameraUbo) {
        let mut view = self.camera_ubo.write(frame).unwrap();
        bytemuck::cast_slice_mut::<_, CameraUbo>(view.deref_mut())[0] = data;
    }

    /// Prepares the input ID objects based on the provided query. Also generates draw keys to use
    /// when generating draw calls.
    ///
    /// Returns `true` if the ID buffer was expanded.
    pub fn prepare_input_ids(
        &mut self,
        frame: usize,
        layers: RenderLayer,
        queries: &Queries<RenderQuery>,
        static_geometry: &StaticGeometryInner,
    ) -> bool {
        // Record last static draw count
        self.last_static_draws = self.static_draws;

        // Reset keys for the frame
        let keys = &mut self.keys[frame];

        // State tracking:

        // If the input ID buffer was expanded
        let mut expanded = false;

        // The combined total number of objects
        let mut object_count = 0;

        // Offset within the global object data buffer
        let mut data_offset = 0;

        // The capacity of the input ID buffer measured in `InputObjectId`s.
        let mut cap = self.input_ids.size() as usize / std::mem::size_of::<InputObjectId>();

        // NOTE: We need to "erase" the lifetime associated with the view because expanding the
        // buffer while also needing to swap the view is currently impossible (afaik) in safe Rust.
        //
        // TODO: I think this only works on the Vulkan implementation. Other implementations might
        // not work because of flushing/invalidation of buffers. Look into that.
        let mut id_view = unsafe {
            let view = self.input_ids.write(frame).unwrap();
            let (ptr, len) = view.into_raw();
            let slice = std::slice::from_raw_parts_mut(ptr.as_ptr(), len);
            bytemuck::cast_slice_mut::<_, InputObjectId>(slice)
        };

        // Write in static geometry if it's dirty
        if static_geometry.dirty[frame] {
            // Reset state
            keys.clear();
            self.static_draws = 0;
            self.static_objects = 0;

            for key in &static_geometry.sorted_keys {
                let batch = static_geometry.batches.get(key).unwrap();

                // Skip this batch if we don't have compatible layers
                if batch.renderable.layers & layers == RenderLayer::empty() {
                    data_offset += batch.ids.len();
                    continue;
                }

                // Add in the key
                keys.push((*key, batch.ids.len()));

                // Resize IDs if needed
                if object_count + batch.ids.len() > cap {
                    expanded = true;
                    let new_size =
                        (object_count + batch.ids.len()) * std::mem::size_of::<InputObjectId>();
                    let mut new_buffer =
                        Buffer::expand(&self.input_ids, new_size as u64, true).unwrap();
                    std::mem::swap(&mut self.input_ids, &mut new_buffer);
                    cap = self.input_ids.size() as usize / std::mem::size_of::<InputObjectId>();
                    id_view = unsafe {
                        let view = self.input_ids.write(frame).unwrap();
                        let (ptr, len) = view.into_raw();
                        let slice = std::slice::from_raw_parts_mut(ptr.as_ptr(), len);
                        bytemuck::cast_slice_mut::<_, InputObjectId>(slice)
                    };
                }

                // Add in the input IDs
                for _ in 0..batch.ids.len() {
                    id_view[object_count] = InputObjectId {
                        data_idx: data_offset as u32,
                        draw_idx: [self.static_draws as u32, 0],
                    };
                    self.static_objects += 1;
                    object_count += 1;
                    data_offset += 1;
                }

                self.static_draws += 1;
            }
        }
        // Otherwise, just move the offset
        else {
            keys.truncate(self.static_draws);
            object_count = self.static_objects;
            data_offset = static_geometry.len;
        }

        // This loops over all dynamic objects that can possibly be rendered and filters out
        // objects that are disabled and/or missing a compatible render layer.
        self.dynamic_objects = 0;
        self.dynamic_input_ids.clear();
        for (data_idx, (_, (renderable, _), _)) in queries
            .make::<(Entity, (Read<Renderable>, Read<Model>), Read<Disabled>)>()
            .into_iter()
            .enumerate()
            .filter(|(_, (_, (renderable, _), disabled))| {
                // Filter out objects that don't share at least one layer with us or that are
                // disabled
                (renderable.layers & layers != RenderLayer::empty()) && disabled.is_none()
            })
        {
            object_count += 1;
            self.dynamic_objects += 1;

            // NOTE: Instead of writing the batch index here, we write the draw key so that we can
            // sort the object IDs here in place and then later determine the batch index. This is
            // fine as long as the size of object used for `batch_idx` is the same as the size used
            // for `DrawKey`.
            self.dynamic_input_ids.push(InputObjectId {
                data_idx: (data_offset + data_idx) as u32,
                draw_idx: bytemuck::cast(make_draw_key(&renderable.material, &renderable.mesh)),
            });
        }

        // Sort the object IDs based on the draw key we wrote previously
        self.dynamic_input_ids
            .sort_unstable_by_key(|id| bytemuck::cast::<_, DrawKey>(id.draw_idx));

        // Expand buffer if needed
        if object_count > cap {
            expanded = true;
            let new_size = object_count * std::mem::size_of::<InputObjectId>();
            let mut new_buffer = Buffer::expand(&self.input_ids, new_size as u64, true).unwrap();
            std::mem::swap(&mut self.input_ids, &mut new_buffer);
            id_view = unsafe {
                let view = self.input_ids.write(frame).unwrap();
                let (ptr, len) = view.into_raw();
                let slice = std::slice::from_raw_parts_mut(ptr.as_ptr(), len);
                bytemuck::cast_slice_mut::<_, InputObjectId>(slice)
            };
        }

        // Convert the draw keys into draw indices
        let mut cur_key = DrawKey::MAX;
        self.dynamic_draws = 0;
        for id in &mut self.dynamic_input_ids {
            // Different draw key = new draw call
            let new_key = bytemuck::cast(id.draw_idx);
            if new_key != cur_key {
                cur_key = new_key;
                keys.push((cur_key, 0));
                self.dynamic_draws += 1;
            }

            // Update the draw count
            let draw_idx = keys.len() - 1;
            keys[draw_idx].1 += 1;
            id.draw_idx[0] = draw_idx as u32;
        }

        // Write into the buffer
        id_view[self.static_objects..object_count].copy_from_slice(&self.dynamic_input_ids);

        // Expand the output buffer if needed
        if self.output_ids.size() as usize / std::mem::size_of::<OutputObjectId>() < object_count {
            self.output_ids = Buffer::expand(
                &mut self.output_ids,
                (std::mem::size_of::<OutputObjectId>() * object_count) as u64,
                false,
            )
            .unwrap();
        }

        expanded
    }

    /// Prepares the draw calls for generation based on the keys generated in `prepare_input_ids`.
    ///
    /// Returns `true` if the draw call buffer was expanded.
    pub fn prepare_draw_calls(
        &mut self,
        frame: usize,
        use_alternate: bool,
        factory: &Factory,
    ) -> bool {
        let meshes = factory.0.meshes.lock().unwrap();

        // Expand the draw calls buffer if needed
        // NOTE: Preserve is required for static draw calls.
        let expanded = match Buffer::expand(
            &mut self.draw_calls,
            (self.keys.len() * std::mem::size_of::<DrawCall>()) as u64,
            true,
        ) {
            Some(buffer) => {
                self.draw_calls = buffer;
                true
            }
            None => false,
        };

        let alternate_frame = (frame * 2) + use_alternate as usize;
        let mut cur_offset = 0;
        let mut draw_call_view = self.draw_calls.write(alternate_frame).unwrap();
        let draw_call_slice = bytemuck::cast_slice_mut::<_, DrawCall>(draw_call_view.deref_mut());

        for (i, (key, draw_count)) in self.keys[frame].iter().enumerate() {
            // Grab the mesh used by this draw
            let (_, _, mesh, _) = from_draw_key(*key);
            let mesh = meshes.get(mesh).unwrap();

            // Write in the draw call
            draw_call_slice[i] = DrawCall {
                indirect: DrawIndexedIndirect {
                    index_count: mesh.index_count as u32,
                    instance_count: 0,
                    first_index: mesh.index_block.base(),
                    vertex_offset: mesh.vertex_block.base() as i32,
                    first_instance: cur_offset as u32,
                },
                bounds: mesh.bounds,
            };
            cur_offset += draw_count;
        }

        expanded
    }

    /// Dispatches a compute pass to generate the draw calls.
    pub fn generate_draw_calls<'a>(
        &'a self,
        frame: usize,
        global: &GlobalRenderData,
        render_area: Vec2,
        commands: &mut CommandBuffer<'a>,
    ) {
        // Perform draw generation with the compute shader
        commands.compute_pass(|pass| {
            pass.bind_pipeline(global.draw_gen_pipeline.clone());
            pass.bind_sets(0, vec![&self.draw_gen_sets[frame]]);

            // Determine the number of groups to dispatch
            let object_count = self.static_objects + self.dynamic_objects;
            let group_count = if object_count as u32 % DRAW_GEN_WORKGROUP_SIZE != 0 {
                (object_count as u32 / DRAW_GEN_WORKGROUP_SIZE) + 1
            } else {
                object_count as u32 / DRAW_GEN_WORKGROUP_SIZE
            };
            let push_constants = [DrawGenPushConstants {
                render_area,
                object_count: object_count as u32,
            }];
            pass.push_constants(bytemuck::cast_slice(&push_constants));

            pass.dispatch(group_count, 1, 1);
        });
    }

    /// Performs actual rendering
    pub fn render<'a, 'b>(&'a self, frame: usize, use_alternate: bool, args: RenderArgs<'a, 'b>) {
        let alternate_frame = (frame * 2) + use_alternate as usize;

        // State:

        // These values keep track of the active resource type
        let mut last_material = ResourceId(usize::MAX);
        let mut last_mesh = ResourceId(usize::MAX);
        let mut last_vertex_layout = None;
        let mut last_ubo_size = u64::MAX;

        // The number of draws to perform (if needed) and the offset within the draw buffer to
        // pull draws from
        let mut draw_count = 0;
        let mut draw_offset = args.draw_offset as u64;

        // Bind our index buffer
        args.pass.bind_index_buffer(
            args.mesh_buffers.get_index_buffer().buffer(),
            0,
            0,
            IndexType::U32,
        );

        // Loop over every draw call (key)
        for (i, (key, _)) in self.keys[frame][..(args.draw_offset + args.draw_count)]
            .iter()
            .enumerate()
            .skip(args.draw_offset)
        {
            let (material_id, vertex_layout, mesh_id, _) = from_draw_key(*key);

            // Determine what needs a rebind
            let new_material = last_material != material_id;
            let new_vb = match &mut last_vertex_layout {
                Some(old_layout) => {
                    if *old_layout != vertex_layout {
                        true
                    } else {
                        false
                    }
                }
                None => true,
            };
            let new_mesh = last_mesh != mesh_id;

            // If anything needs a rebind, check if we should draw
            if new_material || new_vb || new_mesh {
                if draw_count > 0 {
                    args.pass.draw_indexed_indirect(
                        &self.draw_calls,
                        alternate_frame,
                        draw_offset * std::mem::size_of::<DrawCall>() as u64,
                        draw_count,
                        std::mem::size_of::<DrawCall>() as u64,
                    );
                }

                // If the new mesh we see is not ready, we must skip it
                draw_count = 0;
                if new_mesh && !args.meshes.get(mesh_id).unwrap().ready {
                    draw_offset = i as u64 + 1;
                    continue;
                }
                // Otherwise, set the offset to the mesh
                else {
                    last_mesh = mesh_id;
                    draw_offset = i as u64;
                }
            }

            // Rebind material
            if new_material {
                let material = args.materials.get(material_id).unwrap();
                args.pass
                    .bind_pipeline(material.pipelines.get(args.pipeline_ty).clone());

                // If the last material as the invalid ID, then we must also bind our global sets
                if last_material == ResourceId(usize::MAX) {
                    args.pass.bind_sets(
                        0,
                        vec![
                            &self.global_sets[frame][use_alternate as usize],
                            &self.camera_sets[frame],
                            args.texture_sets.set(frame),
                        ],
                    );
                }

                // A new material possibly means a new size of material instance data
                if material.data_size != last_ubo_size
                    && (material.data_size != 0 || material.texture_count != 0)
                {
                    args.pass.bind_sets(
                        3,
                        vec![args
                            .material_buffers
                            .get_set(material.data_size, frame)
                            .unwrap()],
                    );
                    last_ubo_size = material.data_size;
                }

                last_material = material_id;
            }

            // Rebind vertex buffers
            if new_vb {
                last_vertex_layout = Some(vertex_layout);
                let vbuffer = args.mesh_buffers.get_vertex_buffer(vertex_layout).unwrap();
                vbuffer.bind(args.pass, vertex_layout);
            }

            draw_count += 1;
        }

        // Perform a final draw if required
        if draw_count > 0 {
            args.pass.draw_indexed_indirect(
                &self.draw_calls,
                alternate_frame,
                draw_offset * std::mem::size_of::<DrawCall>() as u64,
                draw_count,
                std::mem::size_of::<DrawCall>() as u64,
            );
        }
    }
}

#[inline(always)]
pub(crate) fn make_draw_key(material: &MaterialInstance, mesh: &Mesh) -> DrawKey {
    // [Material][Vertex Layout][Mesh   ][MaterialInstance]
    // [ 24 bits][       8 bits][16 bits][         16 bits]

    // Upper 10 bits are pipeline. Middle 11 are material. Bottom 11 are mesh.
    let mut out = 0;
    out |= (material.material.id.0 as u64 & ((1 << 24) - 1)) << 40;
    out |= (mesh.layout.bits() as u64 & ((1 << 8) - 1)) << 32;
    out |= (mesh.id.0 as u64 & ((1 << 16) - 1)) << 16;
    out |= material.id.0 as u64 & ((1 << 16) - 1);
    out
}

/// Material, vertex layout, mesh, and material instance ids in that order.
#[inline(always)]
pub(crate) fn from_draw_key(key: DrawKey) -> (ResourceId, VertexLayout, ResourceId, ResourceId) {
    (
        ResourceId((key >> 40) as usize & ((1 << 24) - 1)),
        unsafe { VertexLayout::from_bits_unchecked(((key >> 32) as u64 & ((1 << 8) - 1)) as u8) },
        ResourceId((key >> 16) as usize & ((1 << 16) - 1)),
        ResourceId(key as usize & ((1 << 16) - 1)),
    )
}
