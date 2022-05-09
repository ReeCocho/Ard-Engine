pub mod camera;
pub mod context;
pub mod factory;
pub mod lighting;
pub mod material;
pub mod mesh;
pub mod pipeline;
pub mod renderer;
pub mod shader;
pub mod surface;
pub mod texture;

use bytemuck::{Pod, Zeroable};

pub mod prelude {
    pub use crate::camera::*;
    pub use crate::context::*;
    pub use crate::factory::*;
    pub use crate::lighting::*;
    pub use crate::material::*;
    pub use crate::mesh::*;
    pub use crate::pipeline::*;
    pub use crate::renderer::*;
    pub use crate::shader::*;
    pub use crate::surface::*;
    pub use crate::texture::*;
    pub use crate::*;
}

use glam::{Mat4, Vec4};
use prelude::*;
use std::{any::Any, hash::Hash};

/// Implemented by the backend. Wraps all graphics types into one trait.
pub trait Backend: 'static + Sized + Eq + Clone + Hash + Any + Send + Sync {
    const MAX_MESHES: usize;
    const MAX_SHADERS: usize;
    const MAX_PIPELINES: usize;
    const MAX_MATERIALS: usize;
    const MAX_CAMERA: usize;
    const MAX_TEXTURES: usize;
    const MAX_TEXTURES_PER_MATERIAL: usize;

    type GraphicsContext: GraphicsContextApi<Self>;
    type Surface: SurfaceApi;
    type Renderer: RendererApi<Self>;
    type Factory: FactoryApi<Self>;
    type Lighting: LightingApi<Self>;
    type StaticGeometry: StaticGeometryApi<Self>;
    type DebugDrawing: DebugDrawingApi<Self>;
    type Mesh: MeshApi;
    type Shader: ShaderApi;
    type Pipeline: PipelineApi;
    type Material: MaterialApi;
    type Camera: CameraApi;
    type Texture: TextureApi;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    R8G8B8A8Unorm,
    R8G8B8A8Srgb,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextureFilter {
    Nearest,
    Linear,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextureTiling {
    Repeat,
    MirroredRepeat,
    ClampToEdge,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum AnisotropyLevel {
    X1,
    X2,
    X4,
    X8,
    X16,
}

/// Volume bounded by the dimensions of a box and sphere.
#[derive(Debug, Default, Copy, Clone)]
#[repr(C)]
pub struct ObjectBounds {
    /// `w` component of `center` should be a bounding sphere radius.
    pub center: Vec4,
    pub half_extents: Vec4,
}

/// Describes a view frustum using planes.
#[derive(Debug, Default, Copy, Clone)]
#[repr(C)]
pub struct Frustum {
    /// Planes come in the following order:
    /// - Left
    /// - Right
    /// - Top
    /// - Bottom
    /// - Near
    /// - Far
    /// With inward facing normals.
    pub planes: [Vec4; 6],
}

unsafe impl Pod for ObjectBounds {}
unsafe impl Zeroable for ObjectBounds {}

unsafe impl Pod for Frustum {}
unsafe impl Zeroable for Frustum {}

impl From<Mat4> for Frustum {
    fn from(m: Mat4) -> Frustum {
        let mut frustum = Frustum {
            planes: [
                m.row(3) + m.row(0),
                m.row(3) - m.row(0),
                m.row(3) - m.row(1),
                m.row(3) + m.row(1),
                m.row(2),
                m.row(3) - m.row(2),
            ],
        };

        for plane in &mut frustum.planes {
            *plane /= Vec4::new(plane.x, plane.y, plane.z, 0.0).length();
        }

        frustum
    }
}
