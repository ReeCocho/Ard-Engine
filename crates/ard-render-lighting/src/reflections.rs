use std::sync::atomic::{AtomicU64, Ordering};

use ard_math::{UVec2, Vec2};
use ard_pal::prelude::*;
use ard_render_base::{resource::ResourceAllocator, Frame, FRAMES_IN_FLIGHT};
use ard_render_camera::{target::RenderTarget, ubo::CameraUbo};
use ard_render_material::{
    factory::{MaterialFactory, PassId, RtPassDefinition},
    material::MaterialResource,
};
use ard_render_meshes::factory::MeshFactory;
use ard_render_objects::objects::RenderObjects;
use ard_render_raytracing::pipeline::{
    RayTracingMaterialPipeline, RayTracingMaterialPipelineCreateInfo,
};
use ard_render_si::{bindings::*, types::*};
use ard_render_textures::factory::TextureFactory;
use ordered_float::NotNan;

use crate::{
    lights::Lights,
    proc_skybox::{ProceduralSkyBox, DI_MAP_SAMPLER},
};

pub const REFLECTIONS_PASS_ID: PassId = PassId::new(13);

pub struct Reflections {
    /// The image that stores reflection colors.
    target: Texture,
    canvas_size: (u32, u32),
    /// Buffer that contains data for each 8x8 tile.
    tiles: Buffer,
    /// Buffer that contains the sum of all tile max kS values.
    _max_ks_sum: Buffer,
    /// Buffer that contains rays.
    rays: Buffer,
    ray_budget: usize,
    /// Buffer containing indirect dispatch parameters.
    indirect_dispatch: Buffer,
    /// Counter used to determine which multi-sample to use for reflections.
    sample_counter: AtomicU64,
    /// Compute pipeline to reset the indirect dispatch and tile buffers.
    reset_pipeline: ComputePipeline,
    reset_set: DescriptorSet,
    /// Compute pipeline to classify tiles.
    classify_pipeline: ComputePipeline,
    classify_sets: [DescriptorSet; FRAMES_IN_FLIGHT],
    /// Compute pipeline for ray generation.
    ray_gen_pipeline: ComputePipeline,
    ray_gen_sets: [DescriptorSet; FRAMES_IN_FLIGHT],
    /// Compute pipeline for accumulating reflection history.
    accum_pipeline: ComputePipeline,
    accum_sets: [DescriptorSet; FRAMES_IN_FLIGHT],
    /// Graphics pipeline for applying reflections
    apply_pipeline: GraphicsPipeline,
    apply_sets: [DescriptorSet; FRAMES_IN_FLIGHT],
    // RT pipeline for ray traced reflections
    rt_pipeline: RayTracingMaterialPipeline,
    rt_sets: [DescriptorSet; FRAMES_IN_FLIGHT],
}

const TILE_SIZE: u32 = 8;
const RAY_BUDGET: usize = 700_000;

const REFLECTION_SAMPLER: Sampler = Sampler {
    min_filter: Filter::Linear,
    mag_filter: Filter::Linear,
    mipmap_filter: Filter::Nearest,
    address_u: SamplerAddressMode::ClampToEdge,
    address_v: SamplerAddressMode::ClampToEdge,
    address_w: SamplerAddressMode::ClampToEdge,
    anisotropy: None,
    compare: None,
    min_lod: unsafe { NotNan::new_unchecked(0.0) },
    max_lod: None,
    unnormalize_coords: false,
    border_color: None,
};

impl Reflections {
    pub fn new(
        ctx: &Context,
        layouts: &Layouts,
        dims: (u32, u32),
        materials: &ResourceAllocator<MaterialResource>,
        factory: &MaterialFactory,
    ) -> Self {
        let reset_pipeline = ComputePipeline::new(
            ctx.clone(),
            ComputePipelineCreateInfo {
                layouts: vec![layouts.reflection_reset.clone()],
                module: Shader::new(
                    ctx.clone(),
                    ShaderCreateInfo {
                        code: include_bytes!(concat!(
                            env!("OUT_DIR"),
                            "./reflection_reset.comp.spv"
                        )),
                        debug_name: Some("reflection_reset_shader".into()),
                    },
                )
                .unwrap(),
                work_group_size: (1, 1, 1),
                push_constants_size: None,
                debug_name: Some("reflection_reset_pipeline".into()),
            },
        )
        .unwrap();

        let classify_pipeline = ComputePipeline::new(
            ctx.clone(),
            ComputePipelineCreateInfo {
                layouts: vec![layouts.reflection_tile_classifier.clone()],
                module: Shader::new(
                    ctx.clone(),
                    ShaderCreateInfo {
                        code: include_bytes!(concat!(
                            env!("OUT_DIR"),
                            "./tile_classifier.comp.spv"
                        )),
                        debug_name: Some("tile_classifier_shader".into()),
                    },
                )
                .unwrap(),
                work_group_size: (TILE_SIZE, TILE_SIZE, 1),
                push_constants_size: Some(
                    size_of::<GpuReflectionTileClassifierPushConstants>() as u32
                ),
                debug_name: Some("tile_classifier_pipeline".into()),
            },
        )
        .unwrap();

        let ray_gen_pipeline = ComputePipeline::new(
            ctx.clone(),
            ComputePipelineCreateInfo {
                layouts: vec![layouts.reflections_ray_gen.clone(), layouts.camera.clone()],
                module: Shader::new(
                    ctx.clone(),
                    ShaderCreateInfo {
                        code: include_bytes!(concat!(env!("OUT_DIR"), "./raygen.comp.spv")),
                        debug_name: Some("reflection_raygen_shader".into()),
                    },
                )
                .unwrap(),
                work_group_size: (TILE_SIZE, TILE_SIZE, 1),
                push_constants_size: Some(size_of::<GpuReflectionRayGenPushConstants>() as u32),
                debug_name: Some("reflection_raygen_pipeline".into()),
            },
        )
        .unwrap();

        let accum_pipeline = ComputePipeline::new(
            ctx.clone(),
            ComputePipelineCreateInfo {
                layouts: vec![layouts.reflection_accum.clone()],
                module: Shader::new(
                    ctx.clone(),
                    ShaderCreateInfo {
                        code: include_bytes!(concat!(
                            env!("OUT_DIR"),
                            "./reflection_accum.comp.spv"
                        )),
                        debug_name: Some("reflection_accum_shader".into()),
                    },
                )
                .unwrap(),
                work_group_size: (TILE_SIZE, TILE_SIZE, 1),
                push_constants_size: Some(size_of::<GpuSsrPushConstants>() as u32),
                debug_name: Some("reflection_accum_pipeline".into()),
            },
        )
        .unwrap();

        let apply_pipeline = GraphicsPipeline::new(
            ctx.clone(),
            GraphicsPipelineCreateInfo {
                stages: ShaderStages::Traditional {
                    vertex: Shader::new(
                        ctx.clone(),
                        ShaderCreateInfo {
                            code: include_bytes!(concat!(
                                env!("OUT_DIR"),
                                "./reflection_apply.vert.spv"
                            )),
                            debug_name: Some("reflection_apply_vertex_shader".into()),
                        },
                    )
                    .unwrap(),
                    fragment: Some(
                        Shader::new(
                            ctx.clone(),
                            ShaderCreateInfo {
                                code: include_bytes!(concat!(
                                    env!("OUT_DIR"),
                                    "./reflection_apply.frag.spv"
                                )),
                                debug_name: Some("reflection_apply_fragment_shader".into()),
                            },
                        )
                        .unwrap(),
                    ),
                },
                layouts: vec![layouts.reflection_apply.clone()],
                vertex_input: VertexInputState {
                    attributes: Vec::default(),
                    bindings: Vec::default(),
                    topology: PrimitiveTopology::TriangleList,
                },
                rasterization: RasterizationState {
                    polygon_mode: PolygonMode::Fill,
                    cull_mode: CullMode::None,
                    front_face: FrontFace::CounterClockwise,
                    alpha_to_coverage: false,
                },
                depth_stencil: None,
                color_blend: ColorBlendState {
                    attachments: vec![ColorBlendAttachment {
                        write_mask: ColorComponents::R
                            | ColorComponents::G
                            | ColorComponents::B,
                        blend: true,
                        src_color_blend_factor: BlendFactor::One,
                        dst_color_blend_factor: BlendFactor::One,
                        src_alpha_blend_factor: BlendFactor::One,
                        dst_alpha_blend_factor: BlendFactor::Zero,
                        color_blend_op: BlendOp::Add,
                        alpha_blend_op: BlendOp::Add,
                    }],
                },
                push_constants_size: None,
                debug_name: Some("reflections_apply_pipeline".into()),
            },
        )
        .unwrap();

        let raygen = Shader::new(
            ctx.clone(),
            ShaderCreateInfo {
                code: include_bytes!(concat!(env!("OUT_DIR"), "./reflections.rgen.spv")),
                debug_name: Some("reflections_ray_gen_shader".into()),
            },
        )
        .unwrap();

        let miss = Shader::new(
            ctx.clone(),
            ShaderCreateInfo {
                code: include_bytes!(concat!(env!("OUT_DIR"), "./reflections.rmiss.spv")),
                debug_name: Some("reflections_miss_shader".into()),
            },
        )
        .unwrap();

        let rt_pipeline = RayTracingMaterialPipeline::new(
            ctx,
            RayTracingMaterialPipelineCreateInfo {
                pass: REFLECTIONS_PASS_ID,
                layouts: vec![
                    layouts.reflections_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                materials,
                factory,
                raygen,
                miss,
                debug_name: Some("reflections_pipeline".into()),
            },
        );

        let indirect_dispatch = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: size_of::<DispatchIndirect>() as u64,
                array_elements: 1,
                buffer_usage: BufferUsage::STORAGE_BUFFER | BufferUsage::INDIRECT_BUFFER,
                memory_usage: MemoryUsage::GpuOnly,
                queue_types: QueueTypes::MAIN,
                sharing_mode: SharingMode::Exclusive,
                debug_name: Some("reflection_indirect_dispatch".into()),
            },
        )
        .unwrap();

        let max_ks_sum = Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: size_of::<u32>() as u64,
                array_elements: 1,
                buffer_usage: BufferUsage::STORAGE_BUFFER,
                memory_usage: MemoryUsage::GpuOnly,
                queue_types: QueueTypes::MAIN,
                sharing_mode: SharingMode::Exclusive,
                debug_name: Some("max_ks_sum".into()),
            },
        )
        .unwrap();

        let (rays, ray_budget) = Self::make_rays(ctx, dims);

        let mut reset_set = DescriptorSet::new(
            ctx.clone(),
            DescriptorSetCreateInfo {
                layout: layouts.reflection_reset.clone(),
                debug_name: Some("reflection_reset_set".into()),
            },
        )
        .unwrap();

        reset_set.update(&[
            DescriptorSetUpdate {
                binding: REFLECTION_RESET_SET_INDIRECT_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &indirect_dispatch,
                    array_element: 0,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_RESET_SET_KS_SUM_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &max_ks_sum,
                    array_element: 0,
                },
            },
        ]);

        let classify_sets = std::array::from_fn(|_| {
            let mut set = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: layouts.reflection_tile_classifier.clone(),
                    debug_name: Some("reflection_tile_classifier_set".into()),
                },
            )
            .unwrap();

            set.update(&[DescriptorSetUpdate {
                binding: REFLECTION_TILE_CLASSIFIER_SET_KS_SUM_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &max_ks_sum,
                    array_element: 0,
                },
            }]);

            set
        });

        let ray_gen_sets = std::array::from_fn(|_| {
            let mut set = DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: layouts.reflections_ray_gen.clone(),
                    debug_name: Some("reflection_ray_gen_set".into()),
                },
            )
            .unwrap();

            set.update(&[
                DescriptorSetUpdate {
                    binding: REFLECTIONS_RAY_GEN_SET_KS_SUM_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &max_ks_sum,
                        array_element: 0,
                    },
                },
                DescriptorSetUpdate {
                    binding: REFLECTIONS_RAY_GEN_SET_INDIRECT_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &indirect_dispatch,
                        array_element: 0,
                    },
                },
            ]);

            set
        });

        let accum_sets = std::array::from_fn(|_| {
            DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: layouts.reflection_accum.clone(),
                    debug_name: Some("reflection_accum_set".into()),
                },
            )
            .unwrap()
        });

        let apply_sets = std::array::from_fn(|_| {
            DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: layouts.reflection_apply.clone(),
                    debug_name: Some("reflection_apply_set".into()),
                },
            )
            .unwrap()
        });

        let rt_sets = std::array::from_fn(|_| {
            DescriptorSet::new(
                ctx.clone(),
                DescriptorSetCreateInfo {
                    layout: layouts.reflections_pass.clone(),
                    debug_name: Some("reflection_pass_set".into()),
                },
            )
            .unwrap()
        });

        Self {
            target: Self::make_target(ctx, dims),
            canvas_size: dims,
            tiles: Self::make_tiles(ctx, dims),
            rays,
            ray_budget,
            indirect_dispatch,
            _max_ks_sum: max_ks_sum,
            sample_counter: AtomicU64::new(0),
            reset_pipeline,
            reset_set,
            ray_gen_pipeline,
            ray_gen_sets,
            classify_pipeline,
            classify_sets,
            accum_pipeline,
            accum_sets,
            apply_pipeline,
            apply_sets,
            rt_pipeline,
            rt_sets,
        }
    }

    pub fn add_pass(factory: &mut MaterialFactory, layouts: &Layouts) {
        factory
            .add_rt_pass(
                REFLECTIONS_PASS_ID,
                RtPassDefinition {
                    layouts: vec![
                        layouts.reflections_pass.clone(),
                        layouts.camera.clone(),
                        layouts.mesh_data.clone(),
                        layouts.texture_slots.clone(),
                        layouts.textures.clone(),
                    ],
                    push_constant_size: Some(
                        std::mem::size_of::<GpuRtReflectionsPushConstants>() as u32
                    ),
                    max_ray_recursion: 1,
                    max_ray_hit_attribute_size: std::mem::size_of::<Vec2>() as u32,
                    max_ray_payload_size: std::mem::size_of::<GpuRtReflectionsPayload>() as u32,
                },
            )
            .unwrap();
    }

    #[inline(always)]
    pub fn image(&self) -> &Texture {
        &self.target
    }

    fn make_target(ctx: &Context, dims: (u32, u32)) -> Texture {
        Texture::new(
            ctx.clone(),
            TextureCreateInfo {
                format: Format::Rgba16SFloat,
                ty: TextureType::Type2D,
                width: dims.0,
                height: dims.1,
                depth: 1,
                // Two images to ping-pong between
                array_elements: 2,
                mip_levels: 1,
                sample_count: MultiSamples::Count1,
                texture_usage: TextureUsage::STORAGE | TextureUsage::SAMPLED,
                memory_usage: MemoryUsage::GpuOnly,
                queue_types: QueueTypes::MAIN,
                sharing_mode: SharingMode::Exclusive,
                debug_name: Some("reflections_target".into()),
            },
        )
        .unwrap()
    }

    fn make_tiles(ctx: &Context, dims: (u32, u32)) -> Buffer {
        let width = dims.0.div_ceil(TILE_SIZE) as u64;
        let height = dims.1.div_ceil(TILE_SIZE) as u64;
        Buffer::new(
            ctx.clone(),
            BufferCreateInfo {
                size: size_of::<GpuReflectionTile>() as u64 * width * height,
                array_elements: 1,
                buffer_usage: BufferUsage::STORAGE_BUFFER,
                memory_usage: MemoryUsage::GpuOnly,
                queue_types: QueueTypes::MAIN,
                sharing_mode: SharingMode::Exclusive,
                debug_name: Some("tile_buffer".into()),
            },
        )
        .unwrap()
    }

    fn make_rays(ctx: &Context, dims: (u32, u32)) -> (Buffer, usize) {
        let width = dims.0.div_ceil(TILE_SIZE) as usize;
        let height = dims.1.div_ceil(TILE_SIZE) as usize;
        let ray_budget = (width * height)
            .max(RAY_BUDGET)
            .next_multiple_of((TILE_SIZE * TILE_SIZE) as usize);
        let ray_size = size_of::<GpuReflectionRay>().next_multiple_of(16);
        let size = (size_of::<u32>() * 4) + (ray_budget * ray_size);
        (
            Buffer::new(
                ctx.clone(),
                BufferCreateInfo {
                    size: size as u64,
                    array_elements: 1,
                    buffer_usage: BufferUsage::STORAGE_BUFFER,
                    memory_usage: MemoryUsage::GpuOnly,
                    queue_types: QueueTypes::MAIN,
                    sharing_mode: SharingMode::Exclusive,
                    debug_name: Some("reflection_ray_buffer".into()),
                },
            )
            .unwrap(),
            ray_budget,
        )
    }

    pub fn resize(&mut self, ctx: &Context, dims: (u32, u32)) {
        let (rays, ray_budget) = Self::make_rays(ctx, dims);
        self.target = Self::make_target(ctx, dims);
        self.canvas_size = dims;
        self.tiles = Self::make_tiles(ctx, dims);
        self.rays = rays;
        self.ray_budget = ray_budget;

        self.classify_sets.iter_mut().for_each(|set| {
            set.update(&[DescriptorSetUpdate {
                binding: REFLECTION_TILE_CLASSIFIER_SET_TILES_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &self.tiles,
                    array_element: 0,
                },
            }]);
        });

        self.rt_sets.iter_mut().for_each(|set| {
            set.update(&[
                DescriptorSetUpdate {
                    binding: REFLECTIONS_PASS_SET_TILES_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &self.tiles,
                        array_element: 0,
                    },
                },
                DescriptorSetUpdate {
                    binding: REFLECTIONS_PASS_SET_RAYS_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &self.rays,
                        array_element: 0,
                    },
                },
            ]);
        });

        self.reset_set.update(&[DescriptorSetUpdate {
            binding: REFLECTION_RESET_SET_RAYS_BINDING,
            array_element: 0,
            value: DescriptorValue::StorageBuffer {
                buffer: &self.rays,
                array_element: 0,
            },
        }]);

        self.ray_gen_sets.iter_mut().for_each(|set| {
            set.update(&[
                DescriptorSetUpdate {
                    binding: REFLECTIONS_RAY_GEN_SET_TILES_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &self.tiles,
                        array_element: 0,
                    },
                },
                DescriptorSetUpdate {
                    binding: REFLECTIONS_RAY_GEN_SET_RAYS_BINDING,
                    array_element: 0,
                    value: DescriptorValue::StorageBuffer {
                        buffer: &self.rays,
                        array_element: 0,
                    },
                },
            ]);
        });

        self.accum_sets.iter_mut().for_each(|set| {
            set.update(&[DescriptorSetUpdate {
                binding: REFLECTION_ACCUM_SET_TILES_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: &self.tiles,
                    array_element: 0,
                },
            }]);
        });
    }

    pub fn update_sky_box_bindings(&mut self, frame: Frame, proc_skybox: &ProceduralSkyBox) {
        let set = &mut self.rt_sets[usize::from(frame)];
        set.update(&[
            DescriptorSetUpdate {
                binding: REFLECTIONS_PASS_SET_ENV_MAP_BINDING,
                array_element: 0,
                value: DescriptorValue::CubeMap {
                    cube_map: proc_skybox.prefiltered_env_map(),
                    array_element: 0,
                    sampler: DI_MAP_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_PASS_SET_DI_MAP_BINDING,
                array_element: 0,
                value: DescriptorValue::CubeMap {
                    cube_map: proc_skybox.di_map(),
                    array_element: 0,
                    sampler: DI_MAP_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
        ]);
    }

    pub fn update_lights_binding(&mut self, frame: Frame, lights: &Lights) {
        let set = &mut self.rt_sets[usize::from(frame)];
        set.update(&[DescriptorSetUpdate {
            binding: REFLECTIONS_PASS_SET_GLOBAL_LIGHTING_INFO_BINDING,
            array_element: 0,
            value: DescriptorValue::UniformBuffer {
                buffer: lights.global_buffer(),
                array_element: 0,
            },
        }]);
    }

    pub fn check_for_rebuild(
        &mut self,
        ctx: &Context,
        materials: &ResourceAllocator<MaterialResource>,
        factory: &MaterialFactory,
    ) {
        self.rt_pipeline.check_for_rebuild(ctx, materials, factory);
    }

    pub fn update_bindings(
        &mut self,
        frame: Frame,
        tlas: &TopLevelAccelerationStructure,
        objects: &RenderObjects,
        target: &RenderTarget,
    ) {
        let frame = usize::from(frame);
        let (src_idx, dst_idx) = if *self.sample_counter.get_mut() % 2 == 0 {
            (0, 1)
        } else {
            (1, 0)
        };

        self.accum_sets[frame].update(&[
            DescriptorSetUpdate {
                binding: REFLECTION_ACCUM_SET_HISTORY_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageImage {
                    texture: &self.target,
                    array_element: src_idx,
                    mip: 0,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_ACCUM_SET_DST_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageImage {
                    texture: &self.target,
                    array_element: dst_idx,
                    mip: 0,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_ACCUM_SET_VEL_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_vel(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_ACCUM_SET_DEPTH_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_depth(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
        ]);

        self.ray_gen_sets[frame].update(&[
            DescriptorSetUpdate {
                binding: REFLECTIONS_RAY_GEN_SET_NORM_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_norm(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_RAY_GEN_SET_TAN_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_tan(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_RAY_GEN_SET_THIN_G_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_thin_g(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_RAY_GEN_SET_DEPTH_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_depth(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
        ]);

        self.apply_sets[frame].update(&[
            DescriptorSetUpdate {
                binding: REFLECTION_APPLY_SET_SRC_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: &self.target,
                    array_element: dst_idx,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_APPLY_SET_THIN_G_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_thin_g(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
        ]);

        self.classify_sets[frame].update(&[
            DescriptorSetUpdate {
                binding: REFLECTION_TILE_CLASSIFIER_SET_THIN_G_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_thin_g(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTION_TILE_CLASSIFIER_SET_VEL_BINDING,
                array_element: 0,
                value: DescriptorValue::Texture {
                    texture: target.final_vel(),
                    array_element: 0,
                    sampler: REFLECTION_SAMPLER,
                    base_mip: 0,
                    mip_count: 1,
                },
            },
        ]);

        self.rt_sets[frame].update(&[
            DescriptorSetUpdate {
                binding: REFLECTIONS_PASS_SET_DST_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageImage {
                    texture: &self.target,
                    array_element: dst_idx,
                    mip: 0,
                },
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_PASS_SET_TLAS_BINDING,
                array_element: 0,
                value: DescriptorValue::TopLevelAccelerationStructure(tlas),
            },
            DescriptorSetUpdate {
                binding: REFLECTIONS_PASS_SET_GLOBAL_OBJECT_DATA_BINDING,
                array_element: 0,
                value: DescriptorValue::StorageBuffer {
                    buffer: objects.object_data(),
                    array_element: 0,
                },
            },
        ]);
    }

    pub fn render<'a>(
        &'a self,
        commands: &mut CommandBuffer<'a>,
        frame: Frame,
        apply_pass: RenderPassDescriptor<'a>,
        camera: &'a CameraUbo,
        mesh_factory: &'a MeshFactory,
        material_factory: &'a MaterialFactory,
        texture_factory: &'a TextureFactory,
        samples: MultiSamples,
    ) {
        let frame_idx = usize::from(frame);
        let sample_count = self.sample_counter.fetch_add(1, Ordering::Relaxed);
        let ms_sample_count = samples.count() as u32;
        let sample_idx = sample_count % ms_sample_count as u64;

        let canvas_dims = UVec2::new(self.canvas_size.0, self.canvas_size.1);
        let inv_canvas_dims = Vec2::ONE / canvas_dims.as_vec2();
        let target_dims = UVec2::new(self.target.dims().0, self.target.dims().1);
        let inv_target_dims = Vec2::ONE / target_dims.as_vec2();

        let classify_consts = [GpuReflectionTileClassifierPushConstants {
            canvas_dims,
            target_dims,
            inv_target_dims,
            sample_count: ms_sample_count,
            frame_count: (sample_count % std::u32::MAX as u64) as u32,
        }];

        let ssr_consts = [GpuSsrPushConstants {
            canvas_dims,
            inv_canvas_dims,
            target_dims,
            inv_target_dims,
            sample_idx: sample_idx as u32,
            sample_count: ms_sample_count,
            frame_count: (sample_count % std::u32::MAX as u64) as u32,
            max_distance: 100.0,
            coarse_thickness: 0.5,
            refine_thickness: 0.5,
            search_skips: 0,
            search_steps: 40,
            refine_steps: 8,
            camera_near_clip: camera.last().near,
        }];

        let ray_gen_consts = [GpuReflectionRayGenPushConstants {
            target_dims,
            inv_target_dims,
            ray_budget: self.ray_budget as u32,
            frame_count: (sample_count % std::u32::MAX as u64) as u32,
        }];

        let rt_consts = [GpuRtReflectionsPushConstants {
            canvas_dims,
            target_dims,
            inv_target_dims,
            sample_idx: sample_idx as i32,
            sample_count: ms_sample_count,
            max_distance: 100.0,
            frame_count: (sample_count % std::u32::MAX as u64) as u32,
        }];

        // Determine which tiles need reflections
        commands.compute_pass(&self.reset_pipeline, Some("refl_reset"), |pass| {
            pass.bind_sets(0, vec![&self.reset_set]);
            ComputePassDispatch::Inline(1, 1, 1)
        });

        commands.compute_pass(&self.classify_pipeline, Some("refl_tile_clasify"), |pass| {
            pass.bind_sets(0, vec![&self.classify_sets[frame_idx]]);
            pass.push_constants(bytemuck::cast_slice(&classify_consts));
            ComputePassDispatch::Inline(
                self.target.dims().0.div_ceil(TILE_SIZE),
                self.target.dims().1.div_ceil(TILE_SIZE),
                1,
            )
        });

        // Allocate and generate rays
        commands.compute_pass(&self.ray_gen_pipeline, Some("refl_ray_gen"), |pass| {
            pass.bind_sets(
                0,
                vec![&self.ray_gen_sets[frame_idx], camera.get_set(frame)],
            );
            pass.push_constants(bytemuck::cast_slice(&ray_gen_consts));
            ComputePassDispatch::Inline(
                self.target.dims().0.div_ceil(TILE_SIZE),
                self.target.dims().1.div_ceil(TILE_SIZE),
                1,
            )
        });

        // Trace rays
        commands.ray_trace_pass(
            &self.rt_pipeline.pipeline(),
            Some("rt_reflections"),
            |pass| {
                pass.bind_sets(
                    0,
                    vec![
                        &self.rt_sets[frame_idx],
                        camera.get_set(frame),
                        mesh_factory.mesh_data_set(frame),
                        material_factory.get_texture_slots_set(frame),
                    ],
                );

                unsafe {
                    pass.bind_sets_unchecked(4, vec![texture_factory.get_set(frame)]);
                }

                pass.push_constants(bytemuck::cast_slice(&rt_consts));

                RayTracingDispatch {
                    src: RayTracingDispatchSource::Indirect {
                        buffer: &self.indirect_dispatch,
                        array_element: 0,
                        offset: 0,
                    },
                    shader_binding_table: self.rt_pipeline.sbt(),
                    raygen_offset: self.rt_pipeline.raygen_offset(),
                    miss_offset: self.rt_pipeline.miss_offset(),
                    hit_range: self.rt_pipeline.hit_range(),
                }
            },
        );

        // Accumulate reflections
        commands.compute_pass(&self.accum_pipeline, Some("accum_reflections"), |pass| {
            pass.bind_sets(0, vec![&self.accum_sets[frame_idx]]);
            pass.push_constants(bytemuck::cast_slice(&ssr_consts));
            ComputePassDispatch::Inline(
                self.target.dims().0.div_ceil(TILE_SIZE),
                self.target.dims().1.div_ceil(TILE_SIZE),
                1,
            )
        });

        // Apply reflection lighting
        commands.render_pass(apply_pass, Some("reflection_apply"), |pass| {
            pass.bind_pipeline(self.apply_pipeline.clone());
            pass.bind_sets(0, vec![&self.apply_sets[frame_idx]]);
            pass.draw(3, 1, 0, 0);
        });
    }
}
