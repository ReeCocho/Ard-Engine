use super::{ConstantsCodeGen, DescriptorSetCodeGen, StructCodeGen};
use crate::{
    binding::{GpuBinding, GpuBindingData},
    constants::{ConstantValue, GpuConstant},
    structure::GpuStructFieldType,
};
use convert_case::{Case, Casing};
use std::io::Write;

pub struct RustStructCodeGen<W: Write> {
    writer: std::io::BufWriter<W>,
}

pub struct RustSetsCodeGen<W: Write> {
    writer: std::io::BufWriter<W>,
    bindings: Vec<GpuBinding>,
    sets: Vec<String>,
}

pub struct RustConstantsCodeGen<W: Write> {
    writer: std::io::BufWriter<W>,
}

impl<W: Write> RustStructCodeGen<W> {
    pub fn new(writer: W) -> Self {
        let mut code_gen = Self {
            writer: std::io::BufWriter::new(writer),
        };

        // Write in warning about code gen
        writeln!(
            code_gen.writer,
            "/// WARNING: This file is autogenerated by the build script of"
        )
        .unwrap();
        writeln!(
            code_gen.writer,
            "/// `ard-render-base`. Modifications to this file will be overwritten.\n"
        )
        .unwrap();

        // Write in includes
        writeln!(code_gen.writer, "use ard_math::*;").unwrap();
        writeln!(code_gen.writer, "use bytemuck::{{Pod, Zeroable}};\n").unwrap();

        code_gen
    }

    fn field_name(ty: &GpuStructFieldType) -> String {
        match ty {
            GpuStructFieldType::Struct(name) => format!("Gpu{name}"),
            GpuStructFieldType::Pointer(_) => "u64".into(),
            GpuStructFieldType::USize => "usize".into(),
            GpuStructFieldType::U16 => "u16".into(),
            GpuStructFieldType::U32 => "u32".into(),
            GpuStructFieldType::I32 => "i32".into(),
            GpuStructFieldType::U64 => "u64".into(),
            GpuStructFieldType::F32 => "f32".into(),
            GpuStructFieldType::Bool => "bool".into(),
            GpuStructFieldType::UVec2 => "UVec2".into(),
            GpuStructFieldType::UVec4 => "UVec4".into(),
            GpuStructFieldType::IVec2 => "IVec2".into(),
            GpuStructFieldType::Vec2 => "Vec2".into(),
            GpuStructFieldType::Vec4 => "Vec4".into(),
            GpuStructFieldType::Mat4 => "Mat4".into(),
            GpuStructFieldType::Mat3x4 => "[Vec4; 3]".into(),
            GpuStructFieldType::Array { ty, len } => format!("[{}; {len}]", Self::field_name(ty)),
        }
    }
}

impl<W: Write> StructCodeGen for RustStructCodeGen<W> {
    fn begin_struct(&mut self, name: &str) {
        writeln!(self.writer, "#[repr(C)]").unwrap();
        writeln!(self.writer, "#[derive(Copy, Clone)]").unwrap();
        writeln!(self.writer, "pub struct Gpu{name} {{").unwrap();
    }

    fn write_field(&mut self, name: &str, ty: &GpuStructFieldType) {
        let field_name = Self::field_name(ty);
        writeln!(self.writer, "pub {name}: {field_name},").unwrap();
    }

    fn end_struct(&mut self, name: &str) {
        writeln!(self.writer, "}}\n").unwrap();
        writeln!(self.writer, "unsafe impl Zeroable for Gpu{name} {{}}").unwrap();
        writeln!(self.writer, "unsafe impl Pod for Gpu{name} {{}}\n").unwrap();
    }
}

impl<W: Write> RustSetsCodeGen<W> {
    pub fn new(writer: W) -> Self {
        let mut code_gen = Self {
            writer: std::io::BufWriter::new(writer),
            bindings: Vec::default(),
            sets: Vec::default(),
        };

        // Write in warning about code gen
        writeln!(
            code_gen.writer,
            "/// WARNING: This file is autogenerated by the build script of"
        )
        .unwrap();
        writeln!(
            code_gen.writer,
            "/// `ard-render-base`. Modifications to this file will be overwritten.\n"
        )
        .unwrap();

        // Write in includes
        writeln!(code_gen.writer, "use ard_math::*;").unwrap();
        writeln!(code_gen.writer, "use crate::consts::*;").unwrap();
        writeln!(code_gen.writer, "use ard_ecs::prelude::*;").unwrap();
        writeln!(code_gen.writer, "use ard_pal::prelude::*;\n").unwrap();

        code_gen
    }

    fn write_binding_rust(
        writer: &mut std::io::BufWriter<W>,
        binding: &GpuBinding,
        binding_const: &str,
    ) {
        let ty = match binding.data() {
            GpuBindingData::Ssbo { access, .. } => {
                format!(
                    "DescriptorType::StorageBuffer(AccessType::{:?})",
                    access.to_pal_access_type(),
                )
            }
            GpuBindingData::Ubo(_) => "DescriptorType::UniformBuffer".to_owned(),
            GpuBindingData::Texture(_)
            | GpuBindingData::MsTexture(_)
            | GpuBindingData::UTexture(_)
            | GpuBindingData::ITexture(_)
            | GpuBindingData::UnboundedTextureArray(_)
            | GpuBindingData::ShadowTextureArray(_) => "DescriptorType::Texture".to_owned(),
            GpuBindingData::CubeMap(_) => "DescriptorType::CubeMap".to_owned(),
            GpuBindingData::StorageImage { access, .. } => {
                format!(
                    "DescriptorType::StorageImage(AccessType::{:?})",
                    access.to_pal_access_type()
                )
            }
            GpuBindingData::Tlas(_) => "DescriptorType::TopLevelAccelerationStructure".to_owned(),
        };

        writeln!(writer, "DescriptorBinding {{").unwrap();
        writeln!(writer, "binding: {binding_const},").unwrap();
        writeln!(writer, "count: {},", binding.count()).unwrap();
        writeln!(writer, "ty: {ty},").unwrap();
        writeln!(writer, "stage: ShaderStage::{:?},", binding.stage()).unwrap();
        writeln!(writer, "}},").unwrap();
    }
}

impl<W: Write> Drop for RustSetsCodeGen<W> {
    fn drop(&mut self) {
        // Write in the global layouts container
        writeln!(self.writer, "#[derive(Resource, Clone)]").unwrap();
        writeln!(self.writer, "pub struct Layouts {{").unwrap();
        for set in &self.sets {
            let set = set.clone().to_case(Case::Snake);
            writeln!(self.writer, "pub {set}: DescriptorSetLayout,").unwrap();
        }
        writeln!(self.writer, "}}\n").unwrap();

        // Build constructor
        writeln!(self.writer, "impl Layouts {{").unwrap();
        writeln!(self.writer, "pub fn new(ctx: &Context) -> Self {{").unwrap();
        writeln!(self.writer, "Layouts {{").unwrap();

        for set in &self.sets {
            let field_name = set.clone().to_case(Case::Snake);
            writeln!(
                self.writer,
                "{field_name} : {set}Layout::create(ctx.clone()),"
            )
            .unwrap();
        }

        writeln!(self.writer, "}}").unwrap();
        writeln!(self.writer, "}}").unwrap();
        writeln!(self.writer, "}}").unwrap();
    }
}

impl<W: Write> DescriptorSetCodeGen for RustSetsCodeGen<W> {
    fn begin_set(&mut self, name: &str) {
        self.sets.push(name.into());
        self.bindings.clear();
    }

    fn write_binding(&mut self, _: &str, binding: &GpuBinding) {
        self.bindings.push(binding.clone());
    }

    fn end_set(&mut self, set_name: &str) {
        // Create binding ID constants
        let mut binding_consts = Vec::with_capacity(self.bindings.len());
        for binding in &self.bindings {
            binding_consts.push(format!(
                "{}_SET_{}_BINDING",
                set_name.to_owned().to_case(Case::UpperSnake),
                binding.name().to_owned().to_case(Case::UpperSnake),
            ));
        }

        // Write in binding ID constants
        for (i, name) in binding_consts.iter().enumerate() {
            writeln!(self.writer, "pub const {}: u32 = {};", name, i).unwrap();
        }
        writeln!(self.writer).unwrap();

        // Create binding struct
        writeln!(self.writer, "pub struct {}Layout;\n", set_name).unwrap();
        writeln!(self.writer, "impl {}Layout {{", set_name).unwrap();
        writeln!(self.writer, "pub fn create(ctx: Context)").unwrap();
        writeln!(self.writer, "-> DescriptorSetLayout {{").unwrap();
        writeln!(self.writer, "DescriptorSetLayout::new(").unwrap();
        writeln!(self.writer, "ctx, DescriptorSetLayoutCreateInfo {{").unwrap();
        writeln!(self.writer, "bindings: vec![").unwrap();

        for (i, binding) in self.bindings.iter().enumerate() {
            Self::write_binding_rust(&mut self.writer, binding, &binding_consts[i]);
        }

        writeln!(self.writer, "], }}, ).unwrap() }} }}\n").unwrap();
    }
}

impl<W: Write> RustConstantsCodeGen<W> {
    pub fn new(writer: W) -> Self {
        let mut code_gen = Self {
            writer: std::io::BufWriter::new(writer),
        };

        // Write in warning about code gen
        writeln!(
            code_gen.writer,
            "/// WARNING: This file is autogenerated by the build script of"
        )
        .unwrap();
        writeln!(
            code_gen.writer,
            "/// `ard-render-base`. Modifications to this file will be overwritten.\n"
        )
        .unwrap();

        code_gen
    }
}

impl<W: Write> ConstantsCodeGen for RustConstantsCodeGen<W> {
    fn write_constant(&mut self, constant: &GpuConstant) {
        let name = constant.name.to_case(Case::UpperSnake);

        let (value, ty) = match &constant.value {
            ConstantValue::Float(f) => (f.to_string(), "f32"),
            ConstantValue::Int(i) => (i.to_string(), "i32"),
            ConstantValue::UInt(u) => (u.to_string(), "u32"),
            ConstantValue::USize(u) => (u.to_string(), "usize"),
            ConstantValue::Custom(ty, c) => (c.clone(), ty.to_rust_type()),
        };

        writeln!(self.writer, "pub const {name}: {ty} = {value};").unwrap();
    }
}
