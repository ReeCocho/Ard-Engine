use ard_math::Vec2;
use ard_render_material::factory::{MaterialFactory, PassDefinition, PassId, RtPassDefinition};
use ard_render_si::{bindings::Layouts, types::*};

pub mod color;
pub mod depth_prepass;
pub mod entities;
pub mod hzb;
pub mod pathtracer;
pub mod shadow;
pub mod transparent;

/// The depth only pass is used for high-z occlusion culling depth generation.
pub const HIGH_Z_PASS_ID: PassId = PassId::new(0);

/// Pass used for shadow mapping
pub const SHADOW_OPAQUE_PASS_ID: PassId = PassId::new(1);

/// Pass used for shadow mapping
pub const SHADOW_ALPHA_CUTOFF_PASS_ID: PassId = PassId::new(2);

/// The depth prepass results in a depth image containing opaque geometry.
pub const DEPTH_OPAQUE_PREPASS_PASS_ID: PassId = PassId::new(3);

/// The depth prepass results in a depth image containing opaque geometry.
pub const DEPTH_ALPHA_CUTOFF_PREPASS_PASS_ID: PassId = PassId::new(4);

/// Entity pass used for pixel-perfect entity selection in the editor.
pub const ENTITIES_OPAQUE_PASS_ID: PassId = PassId::new(5);
pub const ENTITIES_ALPHA_CUTOFF_PASS_ID: PassId = PassId::new(6);
pub const ENTITIES_TRANSPARENT_PASS_ID: PassId = PassId::new(7);

/// The opaque pass renders only opaque geometry.
pub const COLOR_OPAQUE_PASS_ID: PassId = PassId::new(8);

/// The alpha-cutoff pass renders only opaque geometry.
pub const COLOR_ALPHA_CUTOFF_PASS_ID: PassId = PassId::new(9);

/// Transparent pre-pass that only renders G-buffer values.
pub const TRANSPARENT_PREPASS_ID: PassId = PassId::new(10);

/// The main transparent pass that renders color.
pub const TRANSPARENT_COLOR_PASS_ID: PassId = PassId::new(11);

/// Pass used for path tracing.
pub const PATH_TRACER_PASS_ID: PassId = PassId::new(12);

/// Defines primary passes.
pub fn define_passes(factory: &mut MaterialFactory, layouts: &Layouts) {
    factory
        .add_pass(
            HIGH_Z_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.hzb_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 0,
            },
        )
        .unwrap();

    factory
        .add_pass(
            SHADOW_OPAQUE_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.shadow_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 0,
            },
        )
        .unwrap();

    factory
        .add_pass(
            SHADOW_ALPHA_CUTOFF_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.shadow_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 0,
            },
        )
        .unwrap();

    factory
        .add_pass(
            DEPTH_OPAQUE_PREPASS_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.depth_prepass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_pass(
            DEPTH_ALPHA_CUTOFF_PREPASS_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.depth_prepass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_pass(
            ENTITIES_OPAQUE_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.entity_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_pass(
            ENTITIES_ALPHA_CUTOFF_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.entity_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_pass(
            ENTITIES_TRANSPARENT_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.entity_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_pass(
            COLOR_OPAQUE_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.color_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 5,
            },
        )
        .unwrap();

    factory
        .add_pass(
            COLOR_ALPHA_CUTOFF_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.color_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 5,
            },
        )
        .unwrap();

    factory
        .add_pass(
            TRANSPARENT_PREPASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.color_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 5,
            },
        )
        .unwrap();

    factory
        .add_pass(
            TRANSPARENT_COLOR_PASS_ID,
            PassDefinition {
                layouts: vec![
                    layouts.transparent_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                has_depth_stencil_attachment: true,
                color_attachment_count: 1,
            },
        )
        .unwrap();

    factory
        .add_rt_pass(
            PATH_TRACER_PASS_ID,
            RtPassDefinition {
                layouts: vec![
                    layouts.path_tracer_pass.clone(),
                    layouts.camera.clone(),
                    layouts.mesh_data.clone(),
                    layouts.texture_slots.clone(),
                    layouts.textures.clone(),
                ],
                push_constant_size: Some(std::mem::size_of::<GpuPathTracerPushConstants>() as u32),
                max_ray_recursion: 1,
                max_ray_hit_attribute_size: std::mem::size_of::<Vec2>() as u32,
                max_ray_payload_size: std::mem::size_of::<GpuPathTracerPayload>() as u32,
            },
        )
        .unwrap();
}
