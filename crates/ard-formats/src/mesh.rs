use std::{collections::HashMap, error::Error, path::PathBuf};

use ard_math::*;
use ard_pal::prelude::*;
use bitflags::bitflags;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use static_assertions::const_assert;
use thiserror::Error;

pub trait VertexSource {
    type Error: Error;

    /// Converts the vertex source into a combined raw buffer for uploading to the GPU.
    fn into_vertex_data(self) -> Result<VertexData, Self::Error>;

    /// Number of vertices within the source.
    fn vertex_count(&self) -> usize;

    /// Gets the layout of this vertex source.
    fn layout(&self) -> VertexLayout;
}

pub trait IndexSource {
    type Error: Error;

    /// Converts the index source into a combined raw buffer for uploading to the GPU.
    fn into_index_data(self) -> Result<IndexData, Self::Error>;

    /// Number of indicies within the source.
    fn index_count(&self) -> usize;
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshHeader {
    pub index_count: u32,
    pub vertex_count: u32,
    pub vertex_layout: VertexLayout,
}

/// Volume bounded by the dimensions of a box and sphere.
#[derive(Debug, Serialize, Deserialize, Default, Copy, Clone)]
#[repr(C)]
pub struct ObjectBounds {
    /// `w` component of `center` should be a bounding sphere radius.
    pub center: Vec4,
    pub half_extents: Vec4,
}

unsafe impl Pod for ObjectBounds {}
unsafe impl Zeroable for ObjectBounds {}

#[derive(Debug, Error, Copy, Clone)]
#[error("vertex layout must have only one bit enabled to be converted to an attribute")]
pub struct VertexLayoutToAttributeError;

bitflags! {
    #[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct VertexLayout: u8 {
        const POSITION  = 0b0000_0001;
        const NORMAL    = 0b0000_0010;
        const TANGENT   = 0b0000_0100;
        const COLOR     = 0b0000_1000;
        const UV0       = 0b0001_0000;
        const UV1       = 0b0010_0000;
        const UV2       = 0b0100_0000;
        const UV3       = 0b1000_0000;
    }
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexAttribute {
    Position,
    Normal,
    Tangent,
    Color,
    Uv0,
    Uv1,
    Uv2,
    Uv3,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexData {
    data: Vec<u8>,
    len: usize,
}

#[derive(Debug)]
pub struct VertexDataBuilder {
    data: Vec<u8>,
    bounds: ObjectBounds,
    len: usize,
    offsets: HashMap<VertexAttribute, u32>,
    layout: VertexLayout,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VertexData {
    /// The actual packed vertex data
    data: Vec<u8>,
    /// Object bounds for the contained vertex data in local space.
    bounds: ObjectBounds,
    /// The number of vertex elements.
    len: usize,
    /// Offsets within the data buffer for the beginning of each attribute.
    offsets: HashMap<VertexAttribute, u32>,
}

impl MeshHeader {
    pub fn index_path(root: impl Into<PathBuf>) -> PathBuf {
        let mut path: PathBuf = root.into();
        path.push("indices");
        path
    }

    pub fn vertex_path(root: impl Into<PathBuf>) -> PathBuf {
        let mut path: PathBuf = root.into();
        path.push("vertices");
        path
    }
}

impl VertexAttribute {
    // TODO: Use `std::mem::variant_count` when it comes out of nightly
    pub const COUNT: usize = 8;

    pub const fn size(&self) -> usize {
        match self {
            VertexAttribute::Position => std::mem::size_of::<f32>() * 4,
            VertexAttribute::Normal => std::mem::size_of::<i8>() * 4,
            VertexAttribute::Tangent => std::mem::size_of::<i8>() * 4,
            VertexAttribute::Color => std::mem::size_of::<u8>() * 4,
            VertexAttribute::Uv0 => std::mem::size_of::<f32>() * 2,
            VertexAttribute::Uv1 => std::mem::size_of::<f32>() * 2,
            VertexAttribute::Uv2 => std::mem::size_of::<f32>() * 2,
            VertexAttribute::Uv3 => std::mem::size_of::<f32>() * 2,
        }
    }

    pub const fn format(&self) -> Format {
        match self {
            VertexAttribute::Position => Format::Rgba32SFloat,
            VertexAttribute::Normal => Format::Rgba8Snorm,
            VertexAttribute::Tangent => Format::Rgba8Snorm,
            VertexAttribute::Color => Format::Rgba8Unorm,
            VertexAttribute::Uv0 => Format::Rg32SFloat,
            VertexAttribute::Uv1 => Format::Rg32SFloat,
            VertexAttribute::Uv2 => Format::Rg32SFloat,
            VertexAttribute::Uv3 => Format::Rg32SFloat,
        }
    }

    pub const fn idx(&self) -> usize {
        *self as usize
    }
}

impl VertexData {
    pub fn staging_buffer(
        &self,
        ctx: Context,
        queue: QueueType,
        debug_name: Option<String>,
    ) -> Result<Buffer, BufferCreateError> {
        Buffer::new_staging(ctx, queue, debug_name, &self.data)
    }

    #[inline(always)]
    pub fn bounds(&self) -> ObjectBounds {
        self.bounds
    }

    #[inline]
    pub fn layout(&self) -> VertexLayout {
        let mut layout = VertexLayout::empty();
        for attribute in self.offsets.keys() {
            layout |= match *attribute {
                VertexAttribute::Position => VertexLayout::POSITION,
                VertexAttribute::Normal => VertexLayout::NORMAL,
                VertexAttribute::Tangent => VertexLayout::TANGENT,
                VertexAttribute::Color => VertexLayout::COLOR,
                VertexAttribute::Uv0 => VertexLayout::UV0,
                VertexAttribute::Uv1 => VertexLayout::UV1,
                VertexAttribute::Uv2 => VertexLayout::UV2,
                VertexAttribute::Uv3 => VertexLayout::UV3,
            }
        }
        layout
    }

    #[inline(always)]
    pub fn raw(&self) -> &[u8] {
        &self.data
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn offsets(&self) -> &HashMap<VertexAttribute, u32> {
        &self.offsets
    }

    #[inline(always)]
    pub fn get_offset(&self, attribute: VertexAttribute) -> Option<u32> {
        self.offsets.get(&attribute).copied()
    }
}

#[derive(Debug, Error)]
#[error("unknown vertex error")]
pub struct VertexError;

impl VertexSource for VertexData {
    type Error = VertexError;

    fn into_vertex_data(self) -> Result<VertexData, Self::Error> {
        Ok(self)
    }

    fn vertex_count(&self) -> usize {
        self.len
    }

    fn layout(&self) -> VertexLayout {
        self.layout()
    }
}

impl IndexData {
    pub const TYPE: IndexType = IndexType::U32;
    pub const SIZE: usize = std::mem::size_of::<u32>();

    pub fn new(indices: &[u32]) -> Self {
        Self {
            data: bytemuck::cast_slice(indices).to_owned(),
            len: indices.len(),
        }
    }

    pub fn staging_buffer(
        &self,
        ctx: Context,
        queue: QueueType,
        debug_name: Option<String>,
    ) -> Result<Buffer, BufferCreateError> {
        Buffer::new_staging(ctx, queue, debug_name, &self.data)
    }

    #[inline(always)]
    pub fn raw(&self) -> &[u8] {
        &self.data
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl VertexLayout {
    /// Returns `true` if this vertex layout contains a subset of the vertex components of `other`.
    #[inline(always)]
    pub fn subset_of(&self, other: VertexLayout) -> bool {
        (*self | other) == other
    }

    pub fn vertex_input_state(&self) -> VertexInputState {
        let mut state = VertexInputState {
            attributes: Vec::with_capacity(VertexAttribute::COUNT),
            bindings: Vec::with_capacity(VertexAttribute::COUNT),
            topology: PrimitiveTopology::TriangleList,
        };

        // NOTE: This relies on the fact that the `bitflags` crate will iterate over the bits of
        // the `VertexLayout` in order, which I think is technically not guaranteed. Maybe look
        // into a better way of guaranteeing this
        for bit in self.iter() {
            // Safe to unwrap since bits map one-to-one with attributes
            let attribute: VertexAttribute = bit.try_into().unwrap();

            state.bindings.push(VertexInputBinding {
                binding: state.bindings.len() as u32,
                stride: attribute.size() as u32,
                input_rate: VertexInputRate::Vertex,
            });
            state.attributes.push(VertexInputAttribute {
                binding: state.attributes.len() as u32,
                location: state.attributes.len() as u32,
                format: attribute.format(),
                offset: 0,
            });
        }

        state
    }
}

impl VertexDataBuilder {
    pub fn new(layout: VertexLayout, len: usize) -> Self {
        let mut buff_size = 0;
        let mut offsets = HashMap::<VertexAttribute, u32>::default();

        for bit in layout.iter() {
            offsets.insert(bit.try_into().unwrap(), buff_size);
            buff_size += VertexAttribute::try_from(bit).unwrap().size() as u32 * len as u32;
        }

        Self {
            layout,
            bounds: ObjectBounds::default(),
            len,
            offsets,
            data: vec![0; buff_size as usize],
        }
    }

    pub fn add_positions(mut self, src: &[Vec4]) -> Self {
        const_assert!(matches!(
            VertexAttribute::Position.format(),
            Format::Rgba32SFloat
        ));

        let start = match self.offsets.get(&VertexAttribute::Position) {
            Some(start) => *start as usize,
            None => return self,
        };
        let end = start + (VertexAttribute::Position.size() * self.len);
        let dst: &mut [Vec4] = bytemuck::cast_slice_mut(&mut self.data[start..end]);

        dst.iter_mut().zip(src.iter()).for_each(|(dst, src)| {
            *dst = Vec4::new(src.x, src.y, src.z, 1.0);
        });

        self.bounds = ObjectBounds::from_positions(dst);

        self
    }

    pub fn add_vec4_normals(mut self, src: &[Vec4]) -> Self {
        const_assert!(matches!(
            VertexAttribute::Normal.format(),
            Format::Rgba8Snorm
        ));

        let start = match self.offsets.get(&VertexAttribute::Normal) {
            Some(start) => *start as usize,
            None => return self,
        };
        let end = start + (VertexAttribute::Normal.size() * self.len);
        let dst: &mut [i8] = bytemuck::cast_slice_mut(&mut self.data[start..end]);

        dst.chunks_exact_mut(4)
            .zip(src.iter())
            .for_each(|(dst, src)| {
                let norm = src.xyz().try_normalize().unwrap_or(Vec3::Z);

                dst[0] = if norm.x >= 0.0 {
                    (norm.x * 127.0) + 0.5
                } else {
                    (norm.x * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[1] = if norm.y >= 0.0 {
                    (norm.y * 127.0) + 0.5
                } else {
                    (norm.y * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[2] = if norm.z >= 0.0 {
                    (norm.z * 127.0) + 0.5
                } else {
                    (norm.z * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[3] = 0;
            });

        self
    }

    pub fn add_vec4_tangents(mut self, src: &[Vec4]) -> Self {
        const_assert!(matches!(
            VertexAttribute::Tangent.format(),
            Format::Rgba8Snorm
        ));

        if !self.layout.contains(VertexLayout::TANGENT) {
            return self;
        }

        let start = match self.offsets.get(&VertexAttribute::Tangent) {
            Some(start) => *start as usize,
            None => return self,
        };
        let end = start + (VertexAttribute::Tangent.size() * self.len);
        let dst: &mut [i8] = bytemuck::cast_slice_mut(&mut self.data[start..end]);

        dst.chunks_exact_mut(4)
            .zip(src.iter())
            .for_each(|(dst, src)| {
                let tang = src.xyz().try_normalize().unwrap_or(Vec3::Z);

                dst[0] = if tang.x >= 0.0 {
                    (tang.x * 127.0) + 0.5
                } else {
                    (tang.x * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[1] = if tang.y >= 0.0 {
                    (tang.y * 127.0) + 0.5
                } else {
                    (tang.y * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[2] = if tang.z >= 0.0 {
                    (tang.z * 127.0) + 0.5
                } else {
                    (tang.z * 127.0) - 0.5
                }
                .clamp(-127.0, 127.0) as i8;

                dst[3] = 0;

                let out_tan = Vec3::new(
                    dst[0] as f32 / 127.0,
                    dst[1] as f32 / 127.0,
                    dst[2] as f32 / 127.0,
                );

                if out_tan.length() < 0.5 {
                    println!("Null tangent! Src: {src} Dst: {dst:?}");
                }
            });

        self
    }

    pub fn add_vec4_colors(mut self, src: &[Vec4]) -> Self {
        const_assert!(matches!(
            VertexAttribute::Color.format(),
            Format::Rgba8Unorm
        ));

        if !self.layout.contains(VertexLayout::COLOR) {
            return self;
        }

        let start = match self.offsets.get(&VertexAttribute::Color) {
            Some(start) => *start as usize,
            None => return self,
        };
        let end = start + (VertexAttribute::Color.size() * self.len);
        let dst: &mut [u8] = bytemuck::cast_slice_mut(&mut self.data[start..end]);

        dst.chunks_exact_mut(4)
            .zip(src.iter())
            .for_each(|(dst, src)| {
                // Clamp color values between 0 and 1, then scale it from 0 to 255
                let color = (src.clamp(Vec4::ZERO, Vec4::ONE) * (Vec4::ONE * 255.0)).round();
                dst[0] = color.x as u8;
                dst[1] = color.y as u8;
                dst[2] = color.z as u8;
                dst[3] = color.w as u8;
            });

        self
    }

    pub fn add_vec2_uvs(mut self, src: &[Vec2], idx: usize) -> Self {
        const_assert!(matches!(VertexAttribute::Uv0.format(), Format::Rg32SFloat));
        const_assert!(matches!(VertexAttribute::Uv1.format(), Format::Rg32SFloat));
        const_assert!(matches!(VertexAttribute::Uv2.format(), Format::Rg32SFloat));
        const_assert!(matches!(VertexAttribute::Uv3.format(), Format::Rg32SFloat));

        if !self.layout.contains(VertexLayout::UV0) {
            return self;
        }

        let attribute = match idx {
            0 => VertexAttribute::Uv0,
            1 => VertexAttribute::Uv1,
            2 => VertexAttribute::Uv2,
            3 => VertexAttribute::Uv3,
            _ => return self,
        };
        let start = match self.offsets.get(&attribute) {
            Some(start) => *start as usize,
            None => return self,
        };
        let end = start + (VertexAttribute::Uv0.size() * self.len);
        let dst: &mut [Vec2] = bytemuck::cast_slice_mut(&mut self.data[start..end]);

        dst.copy_from_slice(src);

        self
    }

    pub fn build(self) -> VertexData {
        VertexData {
            data: self.data,
            bounds: self.bounds,
            len: self.len,
            offsets: self.offsets,
        }
    }
}

#[derive(Debug, Error)]
#[error("unknown index error")]
pub struct IndexError;

impl IndexSource for IndexData {
    type Error = IndexError;

    fn into_index_data(self) -> Result<IndexData, Self::Error> {
        Ok(self)
    }

    fn index_count(&self) -> usize {
        self.len
    }
}

impl<'a> IndexSource for &'a [u32] {
    type Error = IndexError;

    fn into_index_data(self) -> Result<IndexData, Self::Error> {
        Ok(IndexData::new(self))
    }

    fn index_count(&self) -> usize {
        self.len()
    }
}

impl<'a> IndexSource for &'a mut [u32] {
    type Error = IndexError;

    fn into_index_data(self) -> Result<IndexData, Self::Error> {
        Ok(IndexData::new(self))
    }

    fn index_count(&self) -> usize {
        self.len()
    }
}

impl From<VertexAttribute> for VertexLayout {
    fn from(value: VertexAttribute) -> Self {
        match value {
            VertexAttribute::Position => VertexLayout::empty(),
            VertexAttribute::Normal => VertexLayout::empty(),
            VertexAttribute::Tangent => VertexLayout::TANGENT,
            VertexAttribute::Color => VertexLayout::COLOR,
            VertexAttribute::Uv0 => VertexLayout::UV0,
            VertexAttribute::Uv1 => VertexLayout::UV1,
            VertexAttribute::Uv2 => VertexLayout::UV2,
            VertexAttribute::Uv3 => VertexLayout::UV3,
        }
    }
}

impl TryFrom<VertexLayout> for VertexAttribute {
    type Error = VertexLayoutToAttributeError;

    fn try_from(value: VertexLayout) -> Result<Self, Self::Error> {
        match value {
            VertexLayout::POSITION => Ok(VertexAttribute::Position),
            VertexLayout::NORMAL => Ok(VertexAttribute::Normal),
            VertexLayout::TANGENT => Ok(VertexAttribute::Tangent),
            VertexLayout::COLOR => Ok(VertexAttribute::Color),
            VertexLayout::UV0 => Ok(VertexAttribute::Uv0),
            VertexLayout::UV1 => Ok(VertexAttribute::Uv1),
            VertexLayout::UV2 => Ok(VertexAttribute::Uv2),
            VertexLayout::UV3 => Ok(VertexAttribute::Uv3),
            _ => Err(VertexLayoutToAttributeError),
        }
    }
}

impl ObjectBounds {
    pub fn from_positions(src: &[Vec4]) -> Self {
        if src.is_empty() {
            return ObjectBounds::default();
        }

        let mut min = src[0];
        let mut max = src[0];
        let mut sqr_radius = min.x.powi(2) + min.z.powi(2) + min.y.powi(2);

        for position in src {
            let new_sqr_radius = position.x.powi(2) + position.z.powi(2) + position.y.powi(2);

            if new_sqr_radius > sqr_radius {
                sqr_radius = new_sqr_radius;
            }

            if position.x < min.x {
                min.x = position.x;
            }

            if position.y < min.y {
                min.y = position.y;
            }

            if position.z < min.z {
                min.z = position.z;
            }

            if position.x > max.x {
                max.x = position.x;
            }

            if position.y > max.y {
                max.y = position.y;
            }

            if position.z > max.z {
                max.z = position.z;
            }
        }

        ObjectBounds {
            center: Vec4::new(
                (max.x + min.x) / 2.0,
                (max.y + min.y) / 2.0,
                (max.z + min.z) / 2.0,
                sqr_radius.sqrt(),
            ),
            half_extents: Vec4::new(
                (max.x - min.x) / 2.0,
                (max.y - min.y) / 2.0,
                (max.z - min.z) / 2.0,
                1.0,
            ),
        }
    }
}
