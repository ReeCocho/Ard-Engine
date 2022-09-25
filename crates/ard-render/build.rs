use std::{path::Path, process::Command};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    pub frames_in_flight: usize,
    pub froxel_table_dims: (usize, usize, usize),
    pub max_point_lights_per_froxel: usize,
    pub max_textures_per_material: usize,
    pub max_shadow_cascades: usize,
}

fn main() {
    // Load in the constants from file
    let config = std::fs::read_to_string(Path::new("./config.ron")).unwrap();
    let config = ron::from_str::<Config>(&config).unwrap();

    // Write shader constants file
    std::fs::write(
        Path::new("./src/shader_constants.rs"),
        format!(
            "\
            /// DO NOT MODIFY: Autogenerated by `build.rs`\n\n\
            pub(crate) const FRAMES_IN_FLIGHT: usize = {};\n\
            pub(crate) const FROXEL_TABLE_DIMS: (usize, usize, usize) = ({}, {}, {});\n\
            pub(crate) const MAX_POINT_LIGHTS_PER_FROXEL: usize = {};\n\
            pub(crate) const MAX_TEXTURES_PER_MATERIAL: usize = {};\n\
            pub(crate) const NO_TEXTURE: u32 = {};\n\
            pub(crate) const MAX_SHADOW_CASCADES: usize = {};\n\
            ",
            config.frames_in_flight,
            config.froxel_table_dims.0,
            config.froxel_table_dims.1,
            config.froxel_table_dims.2,
            config.max_point_lights_per_froxel,
            config.max_textures_per_material,
            u32::MAX,
            config.max_shadow_cascades,
        ),
    )
    .unwrap();

    // Shader constants
    std::fs::write(
        Path::new("./src/shaders/include/constants.glsl"),
        format!(
            "\
            #ifndef _CONSTANTS_GLSL\n\
            #define _CONSTANTS_GLSL\n\
            #define FROXEL_TABLE_X {}\n\
            #define FROXEL_TABLE_Y {}\n\
            #define FROXEL_TABLE_Z {}\n\
            #define MAX_POINT_LIGHTS_PER_FROXEL {}\n\
            #define MAX_TEXTURES_PER_MATERIAL {} \n\
            #define NO_TEXTURE {}\n\
            #define MAX_SHADOW_CASCADES {}\n\
            #endif\n\
            ",
            config.froxel_table_dims.0,
            config.froxel_table_dims.1,
            config.froxel_table_dims.2,
            config.max_point_lights_per_froxel,
            config.max_textures_per_material,
            u32::MAX,
            config.max_shadow_cascades,
        ),
    )
    .unwrap();

    // Data structures

    // Compile shaders
    compile(
        Path::new("./src/shaders/draw_gen.comp"),
        Path::new("./src/shaders/draw_gen.comp.spv"),
        &["HIGH_Z_CULLING"],
    );
    compile(
        Path::new("./src/shaders/draw_gen.comp"),
        Path::new("./src/shaders/draw_gen_no_highz.comp.spv"),
        &[],
    );
    compile(
        Path::new("./src/shaders/highz_gen.comp"),
        Path::new("./src/shaders/highz_gen.comp.spv"),
        &[],
    );
}

fn compile(in_path: &Path, out_path: &Path, flags: &[&str]) {
    // Construct flag arguments.
    let mut flag_args = Vec::with_capacity(flags.len());
    for flag in flags {
        flag_args.push(format!("-D{}", flag));
    }

    // Compile the shader
    let err = format!("unable to compile {:?}", out_path);
    let stderr = Command::new("glslc")
        .arg(in_path)
        .arg("-I./src/shaders/include/")
        .args(&flag_args)
        .arg("--target-env=vulkan1.2")
        .arg("-o")
        .arg(&out_path)
        .output()
        .expect(&err)
        .stderr;

    if !stderr.is_empty() {
        let err = String::from_utf8(stderr).unwrap();
        panic!("unable to compile {:?}:\n{}", in_path, err);
    }
}
