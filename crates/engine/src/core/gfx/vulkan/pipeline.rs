//! Programs (GLSL rewritten and compiled to SPIR-V), their parameters and
//! descriptors, and pipelines.

use ash::vk;

use super::*;

/// shaderc's errors name the source `0`, so they read like GL's
/// (`0:12: error: …`) and get the source lines added the same way.
fn compile(
    compiler: &shaderc::Compiler,
    source: &str,
    kind: shaderc::ShaderKind,
    stage: &str,
    label: &str,
) -> Result<Vec<u32>, String> {
    let mut options = shaderc::CompileOptions::new().map_err(|e| format!("shaderc options: {e}"))?;
    options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_3 as u32);
    options.set_auto_map_locations(true);
    match compiler.compile_into_spirv(source, kind, "0", "main", Some(&options)) {
        Ok(artifact) => Ok(artifact.as_binary().to_vec()),
        Err(e) => Err(format!(
            "{stage} shader ({label}) failed to compile for Vulkan:\n{}",
            crate::core::renderer::shader::annotate_info_log(&e.to_string(), source)
        )),
    }
}

/// Bumped when what `compile` makes of a source changes (options, the
/// rewrite), so older cached SPIR-V is not used.
const SPIRV_CACHE_VERSION: u32 = 1;

/// `compile`, kept in the cache directory between runs by the source's
/// hash: compiling every program took a good part of the start.
fn compile_cached(
    compiler: &shaderc::Compiler,
    source: &str,
    kind: shaderc::ShaderKind,
    stage: &str,
    label: &str,
) -> Result<Vec<u32>, String> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (SPIRV_CACHE_VERSION, stage, source).hash(&mut hasher);
    let path = super::cache_dir().join("spirv").join(format!("{:016x}.spv", hasher.finish()));
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() % 4 == 0 && bytes.len() >= 20 {
            let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            // The SPIR-V magic number: a whole file, not a torn one.
            if words[0] == 0x0723_0203 {
                return Ok(words);
            }
        }
    }
    let words = compile(compiler, source, kind, stage, label)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let temp = path.with_extension("tmp");
    if std::fs::write(&temp, &bytes).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
    Ok(words)
}

impl State {
    pub(super) fn create_program(&mut self, vertex: &str, fragment: &str, label: &str) -> Result<RawId, String> {
        let mut reflect = glsl::to_vulkan(vertex, fragment)
            .map_err(|e| format!("shader program ({label}) cannot be made for Vulkan: {e}"))?;
        let compiler = self.compiler.as_ref().ok_or("no shader compiler")?;
        let vs = compile_cached(compiler, &reflect.vertex, shaderc::ShaderKind::Vertex, "vertex", label)?;
        let mut fs = compile_cached(compiler, &reflect.fragment, shaderc::ShaderKind::Fragment, "fragment", label)?;
        spirv::link_locations(&vs, &mut fs);
        // The reflection reads the source without its `#ifdef`s: a
        // structure declared under `RAY_QUERIES` where the device cannot
        // trace is not in the program, and needs no binding.
        if !spirv::declares_accel(&vs) && !spirv::declares_accel(&fs) {
            reflect.accels.clear();
        }
        let inputs = spirv::interface(&vs, spirv::STORAGE_INPUT).into_iter().map(|v| (v.location, v.scalar)).collect();

        let mut buffers: Vec<glsl::BufferBlock> = Vec::new();
        for b in &reflect.buffers {
            if !buffers.iter().any(|x| x.slot == b.slot) {
                buffers.push(b.clone());
            }
        }
        let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
        let mut bindings = Vec::new();
        for s in &reflect.samplers {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(s.binding)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(stages),
            );
        }
        for b in &buffers {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(glsl::BUFFER_BINDING_BASE + b.slot)
                    .descriptor_type(if b.storage {
                        vk::DescriptorType::STORAGE_BUFFER
                    } else {
                        vk::DescriptorType::UNIFORM_BUFFER
                    })
                    .descriptor_count(1)
                    .stage_flags(stages),
            );
        }
        for &slot in &reflect.accels {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(glsl::ACCEL_BINDING_BASE + slot)
                    .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .descriptor_count(1)
                    .stage_flags(stages),
            );
        }
        if reflect.blocks.len() > MAX_PARAM_BLOCKS {
            return Err(format!("shader program ({label}) has {} params blocks, at most {MAX_PARAM_BLOCKS}", reflect.blocks.len()));
        }
        let params_bindings: Vec<vk::DescriptorSetLayoutBinding> = (0..reflect.blocks.len() as u32)
            .map(|i| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(i)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(1)
                    .stage_flags(stages)
            })
            .collect();
        if bindings.len() as u32 > self.gpu.max_push_descriptors {
            return Err(format!(
                "shader program ({label}) needs {} descriptors, the device pushes at most {}",
                bindings.len(),
                self.gpu.max_push_descriptors
            ));
        }
        let mut seen = std::collections::HashSet::new();
        if let Some(b) = bindings.iter().find(|b| !seen.insert(b.binding)) {
            return Err(format!("shader program ({label}): two descriptors at binding {}", b.binding));
        }
        unsafe {
            let device = &self.gpu.device;
            let module = |words: &[u32]| device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(words), None);
            let vertex = module(&vs).map_err(|e| format!("shader program ({label}): vertex module: {e}"))?;
            let fragment = module(&fs).map_err(|e| format!("shader program ({label}): fragment module: {e}"))?;
            let set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default()
                        .flags(vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR)
                        .bindings(&bindings),
                    None,
                )
                .map_err(|e| format!("shader program ({label}): descriptor set layout: {e}"))?;
            let params_layout = if params_bindings.is_empty() {
                vk::DescriptorSetLayout::null()
            } else {
                device
                    .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&params_bindings), None)
                    .map_err(|e| format!("shader program ({label}): params set layout: {e}"))?
            };
            let both = [set_layout, params_layout];
            let layouts = if params_bindings.is_empty() { &both[..1] } else { &both[..] };
            let layout = device
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(layouts), None)
                .map_err(|e| format!("shader program ({label}): pipeline layout: {e}"))?;
            let sampler_kinds = reflect.samplers.iter().map(|s| convert::sampler_kind(&s.ty)).collect();
            let id = self.id();
            self.programs.insert(
                id,
                Prog {
                    label: label.to_owned(),
                    blocks: reflect.blocks.iter().map(|&size| ParamBlock { data: vec![0; size as usize], upload: None }).collect(),
                    params_layout,
                    units: vec![0; reflect.samplers.len()],
                    sampler_kinds,
                    buffers,
                    inputs,
                    names: reflect
                        .samplers
                        .iter()
                        .enumerate()
                        .map(|(i, s)| (s.name.clone(), Ok(i)))
                        .chain(reflect.params.iter().enumerate().map(|(i, p)| (p.name.clone(), Err(i))))
                        .collect(),
                    reflect,
                    vertex,
                    fragment,
                    set_layout,
                    layout,
                },
            );
            Ok(id)
        }
    }

    pub(super) fn set_uniform(&mut self, program: RawId, name: &str, value: UniformValue) {
        let Some(p) = self.programs.get_mut(&program) else {
            return;
        };
        let (base, index) = match name.strip_suffix(']').and_then(|n| n.split_once('[')) {
            Some((base, i)) => (base, i.trim().parse::<u32>().unwrap_or(0)),
            None => (name, 0),
        };
        let param_index = match p.names.get(base) {
            Some(&Ok(i)) => {
                if let UniformValue::Int(unit) = value {
                    p.units[i] = unit.max(0) as u32;
                }
                return;
            }
            Some(&Err(i)) => i,
            None => return,
        };
        let q = &p.reflect.params[param_index];
        let param = glsl::Param { name: String::new(), ..*q };
        if index >= param.count {
            return;
        }
        // What GL's glProgramUniform* accepts for the declared type; the
        // rest is an error there and changes nothing.
        use glsl::ScalarType as T;
        let fits = match value {
            UniformValue::Int(_) => matches!(param.ty, T::Int | T::Bool) && param.components == 1,
            UniformValue::Int2(_) => matches!(param.ty, T::Int | T::Bool) && param.components == 2,
            UniformValue::Float(_) => matches!(param.ty, T::Float | T::Bool) && param.components == 1,
            UniformValue::Float2(_) => matches!(param.ty, T::Float | T::Bool) && param.components == 2,
            UniformValue::Float3(_) => matches!(param.ty, T::Float | T::Bool) && param.components == 3,
            UniformValue::Float4(_) => matches!(param.ty, T::Float | T::Bool) && param.components == 4,
            UniformValue::Mat4(_) => param.ty == T::Float && param.components == 16,
            UniformValue::Floats(v) => {
                matches!(param.ty, T::Float | T::Bool) && param.components == 1 && (param.is_array || v.len() <= 1)
            }
            UniformValue::Mat4s(m) => param.ty == T::Float && param.components == 16 && (param.is_array || m.len() <= 1),
        };
        if !fits {
            let message = format!("uniform `{name}` of {} set with {value:?} (ignored, as GL does)", p.label);
            self.warn_once(message);
            return;
        }
        let Some(p) = self.programs.get_mut(&program) else {
            return;
        };
        let Some(block) = p.blocks.get_mut(param.block as usize) else {
            return;
        };
        // Only a real change makes the block go up again.
        let mut changed = false;
        let params = &mut block.data;
        let mut write = |offset: u32, bytes: &[u8]| {
            let o = offset as usize;
            if o + bytes.len() <= params.len() && params[o..o + bytes.len()] != *bytes {
                params[o..o + bytes.len()].copy_from_slice(bytes);
                changed = true;
            }
        };
        let at = |i: u32| param.offset + (index + i) * param.stride;
        let floats = |write: &mut dyn FnMut(u32, &[u8]), offset: u32, values: &[f32]| {
            for (i, v) in values.iter().enumerate() {
                write(offset + i as u32 * 4, &v.to_le_bytes());
            }
        };
        match value {
            UniformValue::Int(v) => match param.ty {
                glsl::ScalarType::Bool => write(at(0), &((v != 0) as u32).to_le_bytes()),
                _ => write(at(0), &v.to_le_bytes()),
            },
            UniformValue::Int2([a, b]) => {
                write(at(0), &a.to_le_bytes());
                write(at(0) + 4, &b.to_le_bytes());
            }
            UniformValue::Float(v) => match param.ty {
                glsl::ScalarType::Bool => write(at(0), &((v != 0.0) as u32).to_le_bytes()),
                _ => write(at(0), &v.to_le_bytes()),
            },
            UniformValue::Float2(v) => floats(&mut write, at(0), &v),
            UniformValue::Float3(v) => floats(&mut write, at(0), &v),
            UniformValue::Float4(v) => floats(&mut write, at(0), &v),
            UniformValue::Mat4(m) => floats(&mut write, at(0), m),
            UniformValue::Floats(values) => {
                for (i, v) in values.iter().enumerate() {
                    if index + (i as u32) < param.count {
                        write(at(i as u32), &v.to_le_bytes());
                    }
                }
            }
            UniformValue::Mat4s(ms) => {
                for (i, m) in ms.iter().enumerate() {
                    if index + (i as u32) < param.count {
                        floats(&mut write, at(i as u32), m);
                    }
                }
            }
        }
        if changed {
            block.upload = None;
        }
    }

    pub(super) fn set_pipeline(&mut self, id: RawId) {
        let Some(entry) = self.pipelines.get(&id) else {
            return;
        };
        let resolved = super::super::resolve_pipeline(&entry.desc);
        self.line = match resolved.polygon {
            PolygonMode::Line => true,
            PolygonMode::Solid => false,
            PolygonMode::Fill => super::super::wireframe(),
        };
        self.resolved = Some(resolved);
        self.pipeline = Some(id);
        self.stencil_reference = super::super::stencil_reference();
        self.dynamic_dirty = true;
    }

    /// Vertex stage inputs no attribute of `layouts` feeds.
    pub(super) fn missing_inputs(&self, program: RawId, layouts: &[VertexLayout]) -> Vec<(u32, spirv::Scalar)> {
        let Some(p) = self.programs.get(&program) else {
            return Vec::new();
        };
        p.inputs
            .iter()
            .filter(|(location, _)| !layouts.iter().any(|l| l.attributes.iter().any(|a| a.location == *location)))
            .copied()
            .collect()
    }

    /// The pipeline for `desc` drawing into the current rendering.
    pub(super) fn vk_pipeline(&mut self, program: RawId, desc: &PipelineDesc, line: bool) -> Option<vk::Pipeline> {
        let (colors, depth, stencil, samples) = self.rendering_key;
        let key = PipelineKey { program, desc: *desc, line, colors, depth, stencil, samples };
        if let Some((last, p)) = self.last_vk_pipeline {
            if last == key {
                return Some(p);
            }
        }
        if let Some(&p) = self.vk_pipelines.get(&key) {
            self.last_vk_pipeline = Some((key, p));
            return Some(p);
        }
        let missing = self.missing_inputs(program, &desc.vertex_layouts);
        let p = self.programs.get(&program)?;
        let entry = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(p.vertex).name(entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(p.fragment)
                .name(entry),
        ];
        let mut bindings = Vec::new();
        let mut attributes = Vec::new();
        for (slot, layout) in desc.vertex_layouts.iter().enumerate() {
            bindings.push(vk::VertexInputBindingDescription {
                binding: slot as u32,
                stride: layout.stride,
                input_rate: match layout.step {
                    StepMode::Vertex => vk::VertexInputRate::VERTEX,
                    StepMode::Instance => vk::VertexInputRate::INSTANCE,
                },
            });
            // Only what the shader reads (the validation layer warns
            // about the rest).
            for a in layout.attributes.iter().filter(|a| p.inputs.iter().any(|&(l, _)| l == a.location)) {
                // GL feeds `ivec` inputs from unsigned data too; Vulkan wants
                // the signedness the shader declares.
                let signed = p.inputs.iter().any(|&(l, s)| l == a.location && s == spirv::Scalar::Int);
                let format = match (convert::vertex_format(a.format), signed) {
                    (vk::Format::R32_UINT, true) => vk::Format::R32_SINT,
                    (vk::Format::R32G32_UINT, true) => vk::Format::R32G32_SINT,
                    (vk::Format::R32G32B32_UINT, true) => vk::Format::R32G32B32_SINT,
                    (vk::Format::R32G32B32A32_UINT, true) => vk::Format::R32G32B32A32_SINT,
                    (f, _) => f,
                };
                attributes.push(vk::VertexInputAttributeDescription {
                    location: a.location,
                    binding: slot as u32,
                    format,
                    offset: a.offset,
                });
            }
        }
        if !missing.is_empty() {
            let binding = desc.vertex_layouts.len() as u32;
            bindings.push(vk::VertexInputBindingDescription {
                binding,
                stride: 0,
                input_rate: vk::VertexInputRate::VERTEX,
            });
            for (location, scalar) in &missing {
                attributes.push(vk::VertexInputAttributeDescription {
                    location: *location,
                    binding,
                    format: match scalar {
                        spirv::Scalar::Int => vk::Format::R32G32B32A32_SINT,
                        spirv::Scalar::Uint => vk::Format::R32G32B32A32_UINT,
                        _ => vk::Format::R32G32B32A32_SFLOAT,
                    },
                    offset: 0,
                });
            }
        }
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(convert::topology(desc.topology));
        let mut clip = vk::PipelineViewportDepthClipControlCreateInfoEXT::default().negative_one_to_one(true);
        let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1).push_next(&mut clip);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(if line { vk::PolygonMode::LINE } else { vk::PolygonMode::FILL })
            .cull_mode(match desc.cull {
                Cull::None => vk::CullModeFlags::NONE,
                Cull::Back => vk::CullModeFlags::BACK,
                Cull::Front => vk::CullModeFlags::FRONT,
            })
            // GL's counter-clockwise, seen through the flipped viewport.
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);
        let locations = convert::gl_sample_locations(key.samples);
        let mut sample_locations = vk::PipelineSampleLocationsStateCreateInfoEXT::default()
            .sample_locations_enable(true)
            .sample_locations_info(
                vk::SampleLocationsInfoEXT::default()
                    .sample_locations_per_pixel(convert::samples(key.samples))
                    .sample_location_grid_size(vk::Extent2D { width: 1, height: 1 })
                    .sample_locations(&locations),
            );
        let mut multisample =
            vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(convert::samples(key.samples));
        if self.gpu.sample_locations && !locations.is_empty() {
            multisample = multisample.push_next(&mut sample_locations);
        }
        let has_depth = key.depth != vk::Format::UNDEFINED;
        let stencil_state = desc.stencil.filter(|_| key.stencil);
        let op = |s: &StencilState| vk::StencilOpState {
            fail_op: convert::stencil_op(s.fail),
            pass_op: convert::stencil_op(s.pass),
            depth_fail_op: convert::stencil_op(s.depth_fail),
            compare_op: convert::compare(s.func),
            compare_mask: s.read_mask as u32,
            write_mask: s.write_mask as u32,
            reference: 0,
        };
        let mut depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(has_depth && desc.depth.test)
            .depth_write_enable(has_depth && desc.depth.test && desc.depth.write)
            .depth_compare_op(convert::compare(desc.depth.func));
        if let Some(s) = stencil_state {
            depth_stencil = depth_stencil.stencil_test_enable(true).front(op(&s)).back(op(&s));
        }
        let write_mask = if desc.color_write { vk::ColorComponentFlags::RGBA } else { vk::ColorComponentFlags::empty() };
        let color_formats = &key.colors.0[..key.colors.1 as usize];
        let blend_attachments: Vec<_> = color_formats
            .iter()
            .map(|_| {
                let mut a = vk::PipelineColorBlendAttachmentState::default().color_write_mask(write_mask);
                if let Some(b) = desc.blend {
                    a = a
                        .blend_enable(true)
                        .src_color_blend_factor(convert::blend_factor(b.src))
                        .dst_color_blend_factor(convert::blend_factor(b.dst))
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(convert::blend_factor(b.src))
                        .dst_alpha_blend_factor(convert::blend_factor(b.dst))
                        .alpha_blend_op(vk::BlendOp::ADD);
                }
                a
            })
            .collect();
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
            vk::DynamicState::STENCIL_REFERENCE,
            vk::DynamicState::VERTEX_INPUT_BINDING_STRIDE,
        ];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(color_formats)
            .depth_attachment_format(key.depth)
            .stencil_attachment_format(if key.stencil { key.depth } else { vk::Format::UNDEFINED });
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(p.layout)
            .push_next(&mut rendering);
        let result = unsafe { self.gpu.device.create_graphics_pipelines(self.pipeline_cache, &[info], None) };
        match result {
            Ok(p) => {
                self.vk_pipelines.insert(key, p[0]);
                self.last_vk_pipeline = Some((key, p[0]));
                Some(p[0])
            }
            Err((_, e)) => {
                self.warn_once(format!("pipeline for program {program} failed: {e}"));
                None
            }
        }
    }

    /// For each sampler of `program`, into `out`: binding, view, sampler
    /// and the texture read (a stand-in where nothing fitting is bound).
    pub(super) fn sampler_bindings(&mut self, program: RawId, out: &mut Vec<ImageBinding>) {
        out.clear();
        let Some(p) = self.programs.get(&program) else {
            return;
        };
        let count = p.reflect.samplers.len();
        for i in 0..count {
            let Some(p) = self.programs.get(&program) else {
                return;
            };
            let s = &p.reflect.samplers[i];
            let (kind, shadow) = p.sampler_kinds[i];
            let (binding, unit) = (s.binding, s.fixed_unit.unwrap_or(p.units[i]));
            let (mut texture, sampler) = self.units.get(&(unit, kind)).copied().unwrap_or((0, 0));
            let fits = self.textures.get(&texture).is_some_and(|t| {
                t.desc.kind == kind
                    && t.desc.samples == 1
                    && t.view != vk::ImageView::null()
                    && (!shadow || t.desc.format.is_depth())
            });
            let attached = texture != 0 && self.is_attached(texture);
            if !fits || attached {
                if attached {
                    self.warn_once("a texture drawn into is also sampled; a stand-in is read".into());
                }
                texture = self.dummy_textures[&(kind, shadow)];
            }
            let variant = shadow as usize;
            let cached = self.samplers.get(&sampler).map(|s| s.1[variant]).filter(|s| *s != vk::Sampler::null());
            let vk_sampler = match cached {
                Some(s) => s,
                None => {
                    let mut desc = self.samplers.get(&sampler).map_or(SamplerDesc::LINEAR_CLAMP, |s| s.0);
                    if shadow && desc.compare.is_none() {
                        desc.compare = Some(CompareFunc::LessEqual);
                    }
                    if !shadow {
                        desc.compare = None;
                    }
                    let s = self.sampler_for(&desc);
                    if let Some(entry) = self.samplers.get_mut(&sampler) {
                        entry.1[variant] = s;
                    }
                    s
                }
            };
            out.push((binding, self.textures[&texture].view, vk_sampler, texture));
        }
    }

    /// Push the program's descriptors: `images` from `sampler_bindings`,
    /// the bound buffers, the params block.
    pub(super) fn push_descriptors(&mut self, program: RawId, images: &[ImageBinding]) -> bool {
        let Some(p) = self.programs.get(&program) else {
            return false;
        };
        let block_count = p.buffers.len();
        let layout = p.layout;
        let max_uniform = self.gpu.props.limits.max_uniform_buffer_range as u64;
        let mut buffer_infos = Vec::with_capacity(block_count + 1);
        for i in 0..block_count {
            let b = self.programs[&program].buffers[i];
            let bound = if b.storage { self.storage_buffers.get(&b.slot) } else { self.uniform_buffers.get(&b.slot) }.copied();
            let found = bound.and_then(|(id, offset, size)| self.use_buffer(id).map(|(buf, total)| (buf, offset, size, total)));
            let (buffer, offset, size, total) = match found {
                Some(f) => f,
                None => {
                    let (buf, total) = self.use_buffer(self.dummy_buffer).unwrap_or_default();
                    (buf, 0, total, total)
                }
            };
            let mut range = (size.min(total.saturating_sub(offset))).max(4) as u64;
            if !b.storage {
                range = range.min(max_uniform);
            }
            buffer_infos.push((
                glsl::BUFFER_BINDING_BASE + b.slot,
                if b.storage { vk::DescriptorType::STORAGE_BUFFER } else { vk::DescriptorType::UNIFORM_BUFFER },
                vk::DescriptorBufferInfo { buffer, offset: offset as u64, range },
            ));
        }
        // The acceleration structures the program traces rays through: a
        // draw without one bound would read nothing valid.
        let mut accels: Vec<(u32, [vk::AccelerationStructureKHR; 1])> = Vec::new();
        for i in 0..self.programs[&program].reflect.accels.len() {
            let slot = self.programs[&program].reflect.accels[i];
            // Nothing bound (the lit shaders while reflections are not ray
            // traced, or before the scene's structure is built): the empty
            // one, which such programs do not trace through.
            let bound = self.bound_accels.get(&slot).and_then(|id| self.accels.get(id));
            match bound.or_else(|| self.accels.get(&self.dummy_tlas)) {
                Some(accel) => accels.push((glsl::ACCEL_BINDING_BASE + slot, [accel.handle])),
                None => {
                    self.warn_once(format!("a draw that traces rays without a scene bound at slot {slot} is skipped"));
                    return false;
                }
            }
        }
        let image_infos: Vec<(u32, vk::DescriptorImageInfo)> = images
            .iter()
            .map(|&(binding, view, sampler, _)| {
                (
                    binding,
                    vk::DescriptorImageInfo { sampler, image_view: view, image_layout: vk::ImageLayout::READ_ONLY_OPTIMAL },
                )
            })
            .collect();
        // The same descriptors as pushed last, with the same layout: they
        // are still there.
        let mut key = std::mem::take(&mut self.scratch_push);
        key.clear();
        key.push(vk::Handle::as_raw(layout));
        for (binding, info) in &image_infos {
            key.extend([*binding as u64, vk::Handle::as_raw(info.image_view), vk::Handle::as_raw(info.sampler)]);
        }
        for (binding, _, info) in &buffer_infos {
            key.extend([*binding as u64, vk::Handle::as_raw(info.buffer), info.offset, info.range]);
        }
        for (binding, handle) in &accels {
            key.extend([*binding as u64, vk::Handle::as_raw(handle[0])]);
        }
        if key == self.last_push {
            self.scratch_push = key;
            return true;
        }
        self.scratch_push = std::mem::replace(&mut self.last_push, key);
        let mut writes = Vec::with_capacity(image_infos.len() + buffer_infos.len());
        for (binding, info) in &image_infos {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_binding(*binding)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(info)),
            );
        }
        for (binding, kind, info) in &buffer_infos {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_binding(*binding)
                    .descriptor_type(*kind)
                    .buffer_info(std::slice::from_ref(info)),
            );
        }
        let mut accel_writes: Vec<vk::WriteDescriptorSetAccelerationStructureKHR> = accels
            .iter()
            .map(|(_, handle)| vk::WriteDescriptorSetAccelerationStructureKHR::default().acceleration_structures(handle))
            .collect();
        for ((binding, _), accel) in accels.iter().zip(accel_writes.iter_mut()) {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_binding(*binding)
                    .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .descriptor_count(1)
                    .push_next(accel),
            );
        }
        if !writes.is_empty() {
            unsafe {
                self.gpu.push_fn.cmd_push_descriptor_set(self.cmd(), vk::PipelineBindPoint::GRAPHICS, layout, 0, &writes);
            }
        }
        true
    }

    /// Upload the program's changed params blocks (and those not yet in
    /// this frame's params memory) and bind its params set with their
    /// offsets, unless the same is bound already.
    pub(super) fn bind_params(&mut self, program: RawId) -> bool {
        let Some(p) = self.programs.get(&program) else {
            return false;
        };
        let count = p.blocks.len();
        if count == 0 {
            return true;
        }
        let (layout, params_layout) = (p.layout, p.params_layout);
        let serial = self.serial;
        let align = self.gpu.props.limits.min_uniform_buffer_offset_alignment.max(16);
        let mut offsets = [0u32; MAX_PARAM_BLOCKS];
        // Twice at most: an upload that replaces the params memory makes
        // the blocks uploaded before it go up again.
        for _attempt in 0..2 {
        for i in 0..count {
            let p = &self.programs[&program];
            match p.blocks[i].upload {
                Some((s, offset)) if s == serial => offsets[i] = offset,
                _ => {
                    let size = p.blocks[i].data.len() as u64;
                    let Some((_, offset, ptr)) = self.params_alloc(size, align) else {
                        return false;
                    };
                    let block = &mut self.programs.get_mut(&program).expect("looked up above").blocks[i];
                    unsafe { std::ptr::copy_nonoverlapping(block.data.as_ptr(), ptr, block.data.len()) };
                    block.upload = Some((serial, offset as u32));
                    offsets[i] = offset as u32;
                }
            }
        }
        if self.programs[&program].blocks.iter().all(|b| b.upload.is_some_and(|(s, _)| s == serial)) {
            break;
        }
        }
        // The params memory now (an upload above may have replaced it).
        let Some(memory) = self.frames[self.frame].params.as_ref().map(|c| c.buffer) else {
            return false;
        };
        let set = match self.params_sets.get(&(program, memory)) {
            Some(&set) => set,
            None => {
                let layouts = [params_layout];
                let set = match unsafe {
                    self.gpu.device.allocate_descriptor_sets(
                        &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.params_pool).set_layouts(&layouts),
                    )
                } {
                    Ok(sets) => sets[0],
                    Err(e) => {
                        self.warn_once(format!("no params descriptor set: {e}"));
                        return false;
                    }
                };
                let sizes: Vec<u64> = self.programs[&program].blocks.iter().map(|b| b.data.len() as u64).collect();
                let infos: Vec<vk::DescriptorBufferInfo> =
                    sizes.iter().map(|&range| vk::DescriptorBufferInfo { buffer: memory, offset: 0, range }).collect();
                let writes: Vec<vk::WriteDescriptorSet> = infos
                    .iter()
                    .enumerate()
                    .map(|(i, info)| {
                        vk::WriteDescriptorSet::default()
                            .dst_set(set)
                            .dst_binding(i as u32)
                            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                            .buffer_info(std::slice::from_ref(info))
                    })
                    .collect();
                unsafe { self.gpu.device.update_descriptor_sets(&writes, &[]) };
                self.params_sets.insert((program, memory), set);
                set
            }
        };
        if !self.programs[&program].blocks.iter().all(|b| b.upload.is_some_and(|(s, _)| s == serial)) {
            return false;
        }
        if self.last_params == Some((layout, set, offsets)) {
            return true;
        }
        unsafe {
            self.gpu.device.cmd_bind_descriptor_sets(
                self.cmd(),
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                glsl::PARAMS_SET,
                &[set],
                &offsets[..count],
            );
        }
        self.last_params = Some((layout, set, offsets));
        true
    }
}
