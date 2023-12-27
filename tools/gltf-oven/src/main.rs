use std::fs;
use std::ops::Div;
use std::path::{Path, PathBuf};

use ard_formats::material::{BlendType, MaterialHeader, MaterialType};
use ard_formats::mesh::{IndexData, MeshHeader, VertexDataBuilder, VertexLayout};
use ard_formats::model::{Light, MeshGroup, MeshInstance, ModelHeader, Node, NodeData};
use ard_formats::texture::{Sampler, TextureData, TextureHeader};
use ard_gltf::{GltfLight, GltfMesh, GltfMeshGroup, GltfTexture};
use ard_math::{Mat4, Vec4};
use ard_pal::prelude::Format;
use clap::Parser;
use image::GenericImageView;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the model to bake.
    #[arg(short, long)]
    path: PathBuf,
    /// Output path for the model.
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Compute tangents based on UVs.
    #[arg(long, default_value_t = false)]
    compute_tangents: bool,
    /// Compress textures.
    #[arg(long, default_value_t = false)]
    compress_textures: bool,
}

fn main() {
    let args = Args::parse();

    // Load in the model
    println!("Loading model...");
    let bin = fs::read(&args.path).unwrap();

    // Output folder path
    let out_path = match &args.out {
        Some(path) => path.clone(),
        None => {
            let mut out_path = PathBuf::from(args.path.parent().unwrap());
            out_path.push(format!(
                "{}.ard_mdl",
                args.path.file_stem().unwrap().to_str().unwrap()
            ));
            out_path
        }
    };

    std::fs::create_dir_all(&out_path).unwrap();

    // Parse the model
    println!("Parsing model...");
    let model = ard_gltf::GltfModel::from_slice(&bin).unwrap();
    std::mem::drop(bin);

    // Construct the model header and save it
    println!("Constructing header...");
    let header = create_header(&args, &model);
    let header_path = ModelHeader::header_path(out_path.clone());
    let mut file = std::fs::File::create(header_path).unwrap();
    bincode::serialize_into(&mut file, &header).unwrap();
    std::mem::drop(file);
    std::mem::drop(header);

    // Save everything
    println!("Saving...");
    rayon::join(
        || save_mesh_groups(&args, &out_path, &model.mesh_groups),
        || save_textures(&args, &out_path, &model.textures),
    );
}

fn create_header(args: &Args, gltf: &ard_gltf::GltfModel) -> ModelHeader {
    let mut header = ModelHeader::default();
    header.lights = Vec::with_capacity(gltf.lights.len());
    header.materials = Vec::with_capacity(gltf.materials.len());
    header.mesh_groups = Vec::with_capacity(gltf.mesh_groups.len());
    header.textures = Vec::with_capacity(gltf.textures.len());
    header.roots = Vec::with_capacity(gltf.roots.len());

    for light in &gltf.lights {
        header.lights.push(match light {
            GltfLight::Point {
                color,
                intensity,
                range,
            } => Light::Point {
                color: *color,
                intensity: *intensity,
                range: *range,
            },
            GltfLight::Spot {
                color,
                intensity,
                range,
                inner_angle,
                outer_angle,
            } => Light::Spot {
                color: *color,
                intensity: *intensity,
                range: *range,
                inner_angle: *inner_angle,
                outer_angle: *outer_angle,
            },
            GltfLight::Directional { color, intensity } => Light::Directional {
                color: *color,
                intensity: *intensity,
            },
        });
    }

    for material in &gltf.materials {
        header.materials.push(match material {
            ard_gltf::GltfMaterial::Pbr {
                base_color,
                metallic,
                roughness,
                alpha_cutoff,
                diffuse_map,
                normal_map,
                metallic_roughness_map,
                blending,
            } => MaterialHeader {
                blend_ty: match *blending {
                    ard_gltf::BlendType::Opaque => BlendType::Opaque,
                    ard_gltf::BlendType::Mask => BlendType::Mask,
                    ard_gltf::BlendType::Blend => BlendType::Blend,
                },
                ty: MaterialType::Pbr {
                    base_color: *base_color,
                    metallic: *metallic,
                    roughness: *roughness,
                    alpha_cutoff: *alpha_cutoff,
                    diffuse_map: diffuse_map.map(|v| v as u32),
                    normal_map: normal_map.map(|v| v as u32),
                    metallic_roughness_map: metallic_roughness_map.map(|v| v as u32),
                },
            },
        });
    }

    for mesh_group in &gltf.mesh_groups {
        let mut instances = Vec::with_capacity(mesh_group.0.len());
        for instance in &mesh_group.0 {
            let mut vertex_layout = VertexLayout::POSITION | VertexLayout::NORMAL;

            if instance.mesh.tangents.is_some() {
                vertex_layout |= VertexLayout::TANGENT;
            }

            if instance.mesh.colors.is_some() {
                vertex_layout |= VertexLayout::COLOR;
            }

            if instance.mesh.uv0.is_some() {
                vertex_layout |= VertexLayout::UV0;
            }

            if instance.mesh.uv1.is_some() {
                vertex_layout |= VertexLayout::UV1;
            }

            if instance.mesh.uv2.is_some() {
                vertex_layout |= VertexLayout::UV2;
            }

            if instance.mesh.uv3.is_some() {
                vertex_layout |= VertexLayout::UV3;
            }

            instances.push(MeshInstance {
                material: instance.material as u32,
                mesh: MeshHeader {
                    index_count: instance.mesh.indices.len() as u32,
                    vertex_count: instance.mesh.positions.len() as u32,
                    vertex_layout,
                },
            });
        }

        header.mesh_groups.push(MeshGroup(instances));
    }

    for texture in &gltf.textures {
        let image = image::load_from_memory_with_format(
            &texture.data,
            match texture.src_format {
                ard_gltf::TextureSourceFormat::Png => image::ImageFormat::Png,
                ard_gltf::TextureSourceFormat::Jpeg => image::ImageFormat::Jpeg,
            },
        )
        .unwrap();

        let compress = args.compress_textures && texture_needs_compression(&image);
        let mip_count = texture_mip_count(&image, compress);

        header.textures.push(TextureHeader {
            width: image.width(),
            height: image.height(),
            mip_count: mip_count as u32,
            format: if compress {
                texture.usage.into_compressed_format()
            } else {
                texture.usage.into_format()
            },
            sampler: Sampler {
                min_filter: texture.sampler.min_filter,
                mag_filter: texture.sampler.mag_filter,
                mipmap_filter: texture.sampler.mipmap_filter,
                address_u: texture.sampler.address_u,
                address_v: texture.sampler.address_v,
                anisotropy: true,
            },
        });
    }

    fn parse_node(node: &ard_gltf::GltfNode, root: bool) -> Node {
        let mut out_node = Node {
            name: node.name.clone(),
            model: if root {
                node.model * Mat4::from_cols(-Vec4::X, Vec4::Y, Vec4::Z, Vec4::W)
            } else {
                node.model
            },
            data: match &node.data {
                ard_gltf::GltfNodeData::Empty => NodeData::Empty,
                ard_gltf::GltfNodeData::MeshGroup(id) => NodeData::MeshGroup(*id as u32),
                ard_gltf::GltfNodeData::Light(id) => NodeData::Light(*id as u32),
            },
            children: Vec::with_capacity(node.children.len()),
        };

        for child in &node.children {
            out_node.children.push(parse_node(child, false));
        }

        out_node
    }

    for root in &gltf.roots {
        header.roots.push(parse_node(root, true));
    }

    header
}

fn save_mesh_groups(args: &Args, out: &Path, mesh_groups: &[GltfMeshGroup]) {
    use rayon::prelude::*;
    mesh_groups
        .par_iter()
        .enumerate()
        .for_each(|(i, mesh_group)| {
            // Path to the folder for the mesh group
            let mut mg_path = PathBuf::from(out);
            mg_path.push("mesh_groups");
            mg_path.push(i.to_string());

            // Save each mesh
            mesh_group.0.par_iter().enumerate().for_each(|(j, mesh)| {
                println!("Saving mesh {i}.{j}");
                let mesh_instance_path = ModelHeader::mesh_instance_path(out, i, j);
                fs::create_dir_all(&mesh_instance_path).unwrap();
                save_mesh(args, &mesh_instance_path, &mesh.mesh);
            });
        });
}

fn save_mesh(_args: &Args, out: &Path, mesh: &GltfMesh) {
    rayon::join(
        // Save indices
        || {
            let indices_path = MeshHeader::index_path(out);
            let index_data = IndexData::new(&mesh.indices);
            fs::write(&indices_path, bincode::serialize(&index_data).unwrap()).unwrap();
        },
        // Save vertices
        || {
            let vertices_path = MeshHeader::vertex_path(out);

            let mut vertex_layout = VertexLayout::POSITION | VertexLayout::NORMAL;

            if mesh.tangents.is_some() {
                vertex_layout |= VertexLayout::TANGENT;
            }

            if mesh.colors.is_some() {
                vertex_layout |= VertexLayout::COLOR;
            }

            if mesh.uv0.is_some() {
                vertex_layout |= VertexLayout::UV0;
            }

            if mesh.uv1.is_some() {
                vertex_layout |= VertexLayout::UV1;
            }

            if mesh.uv2.is_some() {
                vertex_layout |= VertexLayout::UV2;
            }

            if mesh.uv3.is_some() {
                vertex_layout |= VertexLayout::UV3;
            }

            // Build vertex data
            let mut vertices = VertexDataBuilder::new(vertex_layout, mesh.positions.len());

            vertices = vertices.add_positions(&mesh.positions);

            vertices = match &mesh.normals {
                Some(normals) => vertices.add_vec4_normals(normals),
                None => {
                    println!("WARNING: Vertices at {vertices_path:?} are missing normals. Generating dummy normals...");
                    vertices.add_vec4_normals(&vec![
                        Vec4::new(0.0, 0.0, 1.0, 0.0);
                        mesh.positions.len()
                    ])
                }
            };

            if let Some(tangents) = &mesh.tangents {
                vertices = vertices.add_vec4_tangents(&tangents);
            }

            if let Some(colors) = &mesh.colors {
                vertices = vertices.add_vec4_colors(&colors);
            }

            if let Some(uv0) = &mesh.uv0 {
                vertices = vertices.add_vec2_uvs(&uv0, 0);
            }

            if let Some(uv1) = &mesh.uv1 {
                vertices = vertices.add_vec2_uvs(&uv1, 1);
            }

            if let Some(uv2) = &mesh.uv2 {
                vertices = vertices.add_vec2_uvs(&uv2, 2);
            }

            if let Some(uv3) = &mesh.uv3 {
                vertices = vertices.add_vec2_uvs(&uv3, 3);
            }

            // Save the buffer
            let raw = bincode::serialize(&vertices.build()).unwrap();
            fs::write(&vertices_path, raw).unwrap();
        },
    );
}

fn save_textures(args: &Args, out: &Path, textures: &[GltfTexture]) {
    use rayon::prelude::*;
    textures.par_iter().enumerate().for_each(|(i, texture)| {
        println!("Saving texture {i}");

        // Path to the folder for the texture
        let tex_path = ModelHeader::texture_path(out, i);

        // Create the texture path if it doesn't exist
        fs::create_dir_all(&tex_path).unwrap();

        // Parse the image
        let image_fmt = match texture.src_format {
            ard_gltf::TextureSourceFormat::Png => image::ImageFormat::Png,
            ard_gltf::TextureSourceFormat::Jpeg => image::ImageFormat::Jpeg,
        };
        let mut image = image::load_from_memory_with_format(&texture.data, image_fmt).unwrap();

        let compress = args.compress_textures && texture_needs_compression(&image);
        let mip_count = texture_mip_count(&image, compress);

        // Compute each mip and save to disc
        for mip in 0..mip_count {
            // Convert the image into a byte array
            let (width, height) = image.dimensions();
            let mut bytes = image.to_rgba8().to_vec();

            // Compress if requested
            if compress {
                let surface = intel_tex_2::RgbaSurface {
                    width,
                    height,
                    stride: width * 4,
                    data: &bytes,
                };
                bytes = intel_tex_2::bc7::compress_blocks(
                    &intel_tex_2::bc7::alpha_ultra_fast_settings(),
                    &surface,
                );
            }

            let tex_data = TextureData::new(
                bytes,
                width,
                height,
                if compress {
                    Format::BC7Srgb
                } else {
                    Format::Rgba8Srgb
                },
            );

            // Save the file to disk
            let mip_path = TextureHeader::mip_path(&tex_path, mip as u32);
            fs::write(mip_path, bincode::serialize(&tex_data).unwrap()).unwrap();

            // Downsample the image for the next mip
            if mip != mip_count - 1 {
                image = image.resize(
                    width.div(2).max(1),
                    height.div(2).max(1),
                    image::imageops::FilterType::Gaussian,
                );
            }
        }
    });
}

/// Helper to determine if a texture needs compression.
#[inline]
fn texture_needs_compression(image: &image::DynamicImage) -> bool {
    let (width, height) = image.dimensions();

    // We only need to compress if our image is at least as big as a block
    width >= 4 && height >= 4
}

/// Helper to determine how many mips a texture needs.
#[inline]
fn texture_mip_count(image: &image::DynamicImage, compressed: bool) -> usize {
    let (width, height) = image.dimensions();
    if compressed {
        (width.div(4).max(height.div(4)) as f32).log2() as usize + 1
    } else {
        (width.max(height) as f32).log2() as usize + 1
    }
}
