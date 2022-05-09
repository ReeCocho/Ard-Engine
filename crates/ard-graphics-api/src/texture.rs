use crate::{TextureFilter, TextureFormat, TextureTiling};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SamplerDescriptor {
    pub min_filter: TextureFilter,
    pub max_filter: TextureFilter,
    pub mip_filter: TextureFilter,
    pub x_tiling: TextureTiling,
    pub y_tiling: TextureTiling,
    pub anisotropic_filtering: bool,
}

pub struct TextureCreateInfo<'a> {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub data: &'a [u8],
    pub mip_type: MipType,
    pub mip_count: usize,
    pub sampler: SamplerDescriptor,
}

/// Indicates how a texture should get it's mip levels.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MipType {
    /// Mip maps will be autogenerated from the image data.
    Generate,
    /// Data contains only the highest level mip. Other mip levels will be provided later.
    Upload,
}

pub trait TextureApi: Clone + Send + Sync {}
