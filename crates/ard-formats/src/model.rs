use std::path::PathBuf;

use crate::{material::MaterialHeader, mesh::MeshHeader, texture::TextureHeader};
use ard_math::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct ModelHeader {
    pub textures: Vec<TextureHeader>,
    pub materials: Vec<MaterialHeader<u32>>,
    pub lights: Vec<Light>,
    pub mesh_groups: Vec<MeshGroup>,
    pub roots: Vec<Node>,
}

#[derive(Serialize, Deserialize)]
pub enum Light {
    Point {
        color: Vec3,
        intensity: f32,
        range: f32,
    },
    Spot {
        color: Vec3,
        intensity: f32,
        range: f32,
        inner_angle: f32,
        outer_angle: f32,
    },
    Directional {
        color: Vec3,
        intensity: f32,
    },
}

#[derive(Serialize, Deserialize)]
pub struct MeshGroup(pub Vec<MeshInstance>);

#[derive(Serialize, Deserialize)]
pub struct MeshInstance {
    pub mesh: MeshHeader,
    pub material: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub model: Mat4,
    pub data: NodeData,
    pub children: Vec<Node>,
}

#[derive(Serialize, Deserialize)]
pub enum NodeData {
    Empty,
    MeshGroup(u32),
    Light(u32),
}

impl ModelHeader {
    pub fn header_path(root: impl Into<PathBuf>) -> PathBuf {
        let mut path: PathBuf = root.into();
        path.push("header");
        path
    }

    pub fn texture_path(root: impl Into<PathBuf>, idx: usize) -> PathBuf {
        let mut path: PathBuf = root.into();
        path.push("textures");
        path.push(idx.to_string());
        path
    }

    pub fn mesh_instance_path(root: impl Into<PathBuf>, group: usize, idx: usize) -> PathBuf {
        let mut path: PathBuf = root.into();
        path.push("mesh_groups");
        path.push(group.to_string());
        path.push(idx.to_string());
        path
    }
}
