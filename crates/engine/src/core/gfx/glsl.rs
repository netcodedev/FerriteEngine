//! GLSL written for OpenGL, made fit for Vulkan (see
//! docs/render-backends.md, stage C).
//!
//! The engine's shaders set loose uniforms by name and samplers by
//! texture unit, which Vulkan's GLSL does not allow. Rather than rewrite
//! every shader, a program's two stages are rewritten together:
//!
//! Samplers and buffers go into descriptor set 0, which Vulkan fills with
//! push descriptors at draw time (only one set of a pipeline may be
//! pushed); the uniforms' blocks into set 1:
//!
//! - every loose non-opaque uniform of either stage becomes a member of
//!   a std140 block declared identically in both (`FerriteParams0`, and
//!   one block per array of `OWN_BLOCK_BYTES` or more, so a small change
//!   does not copy it), all in `PARAMS_SET` as dynamic uniform buffers
//!   (a draw changes only their offsets); the layout is computed here, so the
//!   by-name setters write the right bytes;
//! - every loose sampler gets a binding of its own below
//!   `BUFFER_BINDING_BASE` (the texture unit the program points it at
//!   with a uniform set decides, at draw time, what is bound there); one
//!   with an explicit `binding` keeps it as its unit and binding;
//! - buffer blocks with `binding = n` get `BUFFER_BINDING_BASE + n`;
//! - `gl_VertexID`/`gl_InstanceID` become Vulkan's `gl_VertexIndex`/
//!   `gl_InstanceIndex` (the engine draws them from 0, where they agree),
//!   and the version becomes 4.60.

use std::collections::HashMap;

/// Descriptor set of the loose uniforms' blocks (dynamic uniform
/// buffers: a draw changes only their offsets).
pub const PARAMS_SET: u32 = 1;
/// Arrays at least this big get a block of their own, so changing a
/// small uniform does not copy them again.
pub const OWN_BLOCK_BYTES: u32 = 1024;
/// Bindings below this are samplers; buffer slot `n` is bound at this
/// plus `n`.
pub const BUFFER_BINDING_BASE: u32 = 32;

/// A member of the params block.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: ScalarType,
    /// Components per element (1–4; 16 for mat4, 9 for mat3).
    pub components: u32,
    /// Elements of an array, 1 otherwise.
    pub count: u32,
    pub is_array: bool,
    /// The params block it is in (its binding in `PARAMS_SET`).
    pub block: u32,
    /// Byte offset in the block (std140).
    pub offset: u32,
    /// Bytes from one array element to the next (std140: 16 at least).
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarType {
    Float,
    Int,
    Uint,
    Bool,
}

/// A sampler and the binding it got in the sampler set.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    pub name: String,
    pub binding: u32,
    /// The texture unit it is fixed to, when the source gave it one.
    pub fixed_unit: Option<u32>,
    /// Its GLSL type (`sampler2D`, `sampler2DArrayShadow`, …): what kind
    /// of texture it reads.
    pub ty: String,
}

/// A buffer block (uniform or storage) and where it went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferBlock {
    pub slot: u32,
    pub storage: bool,
}

/// A program rewritten for Vulkan.
#[derive(Clone, Debug)]
pub struct VulkanProgram {
    pub vertex: String,
    pub fragment: String,
    pub params: Vec<Param>,
    /// Size of each params block in bytes, by block (empty without
    /// loose uniforms). Block 0 holds the small ones.
    pub blocks: Vec<u32>,
    pub samplers: Vec<Sampler>,
    pub buffers: Vec<BufferBlock>,
}

impl VulkanProgram {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    pub fn sampler(&self, name: &str) -> Option<&Sampler> {
        self.samplers.iter().find(|s| s.name == name)
    }
}

/// A loose `uniform` declaration found in a stage.
#[derive(Clone, Debug)]
struct Declaration {
    /// The whole statement, to take out of the source.
    text: String,
    ty: String,
    name: String,
    /// The array length expression, if an array.
    array: Option<String>,
    binding: Option<u32>,
}

fn is_opaque(ty: &str) -> bool {
    ty.contains("sampler") || ty.contains("image")
}

/// The statements of `source` at global scope that declare a loose
/// uniform (not a block).
fn loose_uniforms(source: &str) -> Vec<Declaration> {
    let mut found = Vec::new();
    let mut depth = 0i32;
    let code = strip_comments(source);
    let mut statement = String::new();
    for ch in code.chars() {
        match ch {
            '{' => {
                depth += 1;
                statement.clear();
            }
            '}' => {
                depth -= 1;
                statement.clear();
            }
            ';' if depth == 0 => {
                if let Some(d) = parse_uniform(statement.trim()) {
                    found.push(d);
                }
                statement.clear();
            }
            '\n' if statement.trim_start().starts_with('#') => statement.clear(),
            _ if depth == 0 => statement.push(ch),
            _ => {}
        }
    }
    found
}

fn parse_uniform(statement: &str) -> Option<Declaration> {
    // Directives may precede it on earlier lines.
    let statement = statement.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
    let s = statement.trim();
    let (layout, rest) = if let Some(rest) = s.strip_prefix("layout") {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        (Some(rest[open + 1..close].to_string()), rest[close + 1..].trim())
    } else {
        (None, s)
    };
    let rest = rest.strip_prefix("uniform")?.trim();
    let mut words = rest.split_whitespace();
    let mut ty = words.next()?.to_string();
    // Precision qualifiers.
    if matches!(ty.as_str(), "highp" | "mediump" | "lowp") {
        ty = words.next()?.to_string();
    }
    let declarator: String = words.collect::<Vec<_>>().join("");
    let (name, array) = match declarator.find('[') {
        Some(i) => (declarator[..i].to_string(), Some(declarator[i + 1..declarator.rfind(']')?].to_string())),
        None => (declarator.clone(), None),
    };
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let binding = layout.as_deref().and_then(layout_binding);
    Some(Declaration { text: statement.trim().to_string(), ty, name, array, binding })
}

fn layout_binding(layout: &str) -> Option<u32> {
    layout.split(',').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim() == "binding").then(|| value.trim().parse().ok()).flatten()
    })
}

/// `source` without comments (kept line for line).
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut last = ' ';
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                }
                if last == '*' && c == '/' {
                    break;
                }
                last = c;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Scalar type and components of a GLSL type, `None` for types the
/// params block does not take.
fn type_shape(ty: &str) -> Option<(ScalarType, u32)> {
    Some(match ty {
        "float" => (ScalarType::Float, 1),
        "vec2" => (ScalarType::Float, 2),
        "vec3" => (ScalarType::Float, 3),
        "vec4" => (ScalarType::Float, 4),
        "int" => (ScalarType::Int, 1),
        "ivec2" => (ScalarType::Int, 2),
        "ivec3" => (ScalarType::Int, 3),
        "ivec4" => (ScalarType::Int, 4),
        "uint" => (ScalarType::Uint, 1),
        "uvec2" => (ScalarType::Uint, 2),
        "uvec3" => (ScalarType::Uint, 3),
        "uvec4" => (ScalarType::Uint, 4),
        "bool" => (ScalarType::Bool, 1),
        "mat3" => (ScalarType::Float, 9),
        "mat4" => (ScalarType::Float, 16),
        _ => return None,
    })
}

/// std140 size and alignment of one element.
fn std140(components: u32) -> (u32, u32) {
    match components {
        1 => (4, 4),
        2 => (8, 8),
        3 => (12, 16),
        4 => (16, 16),
        9 => (48, 16), // mat3: three vec4 columns
        16 => (64, 16),
        _ => (16, 16),
    }
}

/// Evaluate an array length: a number, or a `#define`d name.
fn array_len(expr: &str, defines: &HashMap<String, String>) -> Option<u32> {
    let expr = expr.trim();
    expr.parse().ok().or_else(|| defines.get(expr).and_then(|v| v.trim().parse().ok()))
}

/// `#define NAME value` and `const int|uint NAME = value;` constants.
fn defines(source: &str) -> HashMap<String, String> {
    let mut found: HashMap<String, String> = source
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("#define")?;
            let mut parts = rest.split_whitespace();
            Some((parts.next()?.to_string(), parts.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    for statement in strip_comments(source).split(';') {
        let statement: Vec<&str> = statement.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
        let statement = statement.join("\n");
        let s = statement.trim();
        let Some(rest) = s.strip_prefix("const") else { continue };
        let rest = rest.trim();
        let Some(rest) = rest.strip_prefix("int").or_else(|| rest.strip_prefix("uint")) else { continue };
        if let Some((name, value)) = rest.split_once('=') {
            let value = value.trim().trim_end_matches('u');
            found.insert(name.trim().to_string(), value.to_string());
        }
    }
    found
}

/// Rewrite a GL program (both stages' full sources) for Vulkan.
pub fn to_vulkan(vertex: &str, fragment: &str) -> Result<VulkanProgram, String> {
    let stages = [vertex, fragment];
    let mut params: Vec<Param> = Vec::new();
    let mut samplers: Vec<Sampler> = Vec::new();
    let mut offset = 0u32;
    let mut per_stage: Vec<Vec<Declaration>> = Vec::new();
    for source in stages {
        let found = loose_uniforms(source);
        let defs = defines(source);
        for d in &found {
            if is_opaque(&d.ty) {
                if samplers.iter().any(|s| s.name == d.name) {
                    continue;
                }
                let binding = d.binding.unwrap_or(0);
                samplers.push(Sampler { name: d.name.clone(), binding, fixed_unit: d.binding, ty: d.ty.clone() });
                continue;
            }
            if params.iter().any(|p| p.name == d.name) {
                continue;
            }
            let (ty, components) = type_shape(&d.ty)
                .ok_or_else(|| format!("uniform `{}` has a type the params block does not take: {}", d.name, d.ty))?;
            let (size, align) = std140(components);
            let (count, is_array, stride, total, align) = match &d.array {
                Some(expr) => {
                    let n = array_len(expr, &defs).ok_or_else(|| format!("array length of `{}`: {expr}", d.name))?;
                    let stride = size.max(16).next_multiple_of(16);
                    (n, true, stride, stride * n, 16)
                }
                None => (1, false, size, size, align),
            };
            if is_array && total >= OWN_BLOCK_BYTES {
                // Numbered once all are known (after block 0).
                params.push(Param { name: d.name.clone(), ty, components, count, is_array, block: u32::MAX, offset: 0, stride });
                continue;
            }
            offset = offset.next_multiple_of(align);
            params.push(Param { name: d.name.clone(), ty, components, count, is_array, block: 0, offset, stride });
            offset += total;
        }
        per_stage.push(found);
    }
    // Samplers with a fixed unit keep it; the others are numbered after
    // the last fixed one in the sampler set, so they do not collide.
    let fixed: Vec<u32> = samplers.iter().filter(|s| s.fixed_unit.is_some()).map(|s| s.binding).collect();
    let mut next = 0;
    for sampler in samplers.iter_mut().filter(|s| s.fixed_unit.is_none()) {
        while fixed.contains(&next) {
            next += 1;
        }
        sampler.binding = next;
        next += 1;
    }
    let mut blocks = Vec::new();
    if params.iter().any(|p| p.block == 0) {
        blocks.push(offset.next_multiple_of(16));
    } else if !params.is_empty() {
        blocks.push(16); // an empty block 0 keeps the numbering simple
    }
    for p in params.iter_mut().filter(|p| p.block == u32::MAX) {
        p.block = blocks.len() as u32;
        blocks.push(p.stride * p.count);
    }
    let mut block = String::new();
    for (index, _) in blocks.iter().enumerate() {
        let members: Vec<&Param> = params.iter().filter(|p| p.block == index as u32).collect();
        block.push_str(&format!("layout(std140, set = {PARAMS_SET}, binding = {index}) uniform FerriteParams{index} {{\n"));
        if members.is_empty() {
            block.push_str(&format!("    vec4 ferriteUnused{index};\n"));
        }
        for p in members {
            let ty = declaration_of(&per_stage, &p.name).map(|d| d.ty.clone()).unwrap_or_default();
            if p.is_array {
                block.push_str(&format!("    layout(offset = {}) {} {}[{}];\n", p.offset, ty, p.name, p.count));
            } else {
                block.push_str(&format!("    layout(offset = {}) {} {};\n", p.offset, ty, p.name));
            }
        }
        block.push_str("};\n");
    }
    let mut buffers = Vec::new();
    let mut out = Vec::new();
    for (source, found) in stages.iter().zip(&per_stage) {
        let mut text = source.to_string();
        for d in found {
            let replacement = if is_opaque(&d.ty) {
                let s = samplers.iter().find(|s| s.name == d.name).expect("collected above");
                format!("layout(set = 0, binding = {}) uniform {} {};", s.binding, d.ty, d.name)
            } else {
                String::new()
            };
            text = replace_statement(&text, &d.text, &replacement);
        }
        text = rebind_blocks(&text, &mut buffers);
        out.push(finish_stage(&text, &block, out.is_empty()));
    }
    let fragment = out.pop().unwrap_or_default();
    let vertex = out.pop().unwrap_or_default();
    Ok(VulkanProgram { vertex, fragment, params, blocks, samplers, buffers })
}

fn declaration_of<'a>(stages: &'a [Vec<Declaration>], name: &str) -> Option<&'a Declaration> {
    stages.iter().flatten().find(|d| d.name == name)
}

/// Replace the declaration statement `statement` (as found, trimmed, up
/// to its `;`) in `source`.
fn replace_statement(source: &str, statement: &str, replacement: &str) -> String {
    let needle = format!("{statement};");
    if let Some(i) = source.find(&needle) {
        return format!("{}{}{}", &source[..i], replacement, &source[i + needle.len()..]);
    }
    // Whitespace may differ (the statement was re-joined): match the
    // tokens loosely, one line at a time.
    let tokens: Vec<&str> = statement.split_whitespace().collect();
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    for line in lines.iter_mut() {
        let code = line.split("//").next().unwrap_or("");
        let line_tokens: Vec<&str> = code.trim_end().trim_end_matches(';').split_whitespace().collect();
        if line_tokens == tokens && code.trim_end().ends_with(';') {
            *line = replacement.to_string();
            return lines.join("\n") + if source.ends_with('\n') { "\n" } else { "" };
        }
    }
    source.to_string()
}

/// Give `layout(... binding = n ...) uniform|buffer Block` declarations
/// the set their slot maps to.
fn rebind_blocks(source: &str, buffers: &mut Vec<BufferBlock>) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("layout") && (line.contains(" uniform ") || line.contains(" buffer ") || line.contains(")uniform") || line.contains(")buffer")) {
            if let (Some(open), Some(close)) = (line.find('('), line.find(')')) {
                let inner = &line[open + 1..close];
                let rest = &line[close + 1..];
                let storage = rest.trim_start().starts_with("buffer")
                    || rest.contains(" buffer ")
                    || rest.trim_start().starts_with("readonly buffer")
                    || rest.trim_start().starts_with("readonly");
                let is_block = !rest.contains("sampler") && !rest.trim_end().ends_with(';')
                    || rest.contains('{');
                if let (Some(slot), true) = (layout_binding(inner), is_block) {
                    let (set, binding) = (0, BUFFER_BINDING_BASE + slot);
                    let parts: Vec<String> = inner
                        .split(',')
                        .filter(|p| p.split_once('=').map_or(true, |(k, _)| k.trim() != "binding" && k.trim() != "set"))
                        .map(|p| p.trim().to_string())
                        .chain([format!("set = {set}"), format!("binding = {binding}")])
                        .collect();
                    buffers.push(BufferBlock { slot, storage });
                    out.push_str(&line[..open + 1]);
                    out.push_str(&parts.join(", "));
                    out.push_str(&line[close..]);
                    out.push('\n');
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The version line made 4.60, the built-ins renamed, and the params
/// block put after the version (and any `#extension` lines).
fn finish_stage(source: &str, block: &str, vertex: bool) -> String {
    let mut out = String::new();
    let mut lines = source.lines().peekable();
    // Leading blank lines and comments before #version are not allowed
    // to stay before it; drop them.
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    if let Some(first) = lines.peek() {
        if first.trim_start().starts_with("#version") {
            lines.next();
        }
    }
    out.push_str("#version 460\n");
    let mut rest: Vec<&str> = lines.collect();
    let mut extensions = Vec::new();
    rest.retain(|l| {
        if l.trim_start().starts_with("#extension") {
            extensions.push(*l);
            false
        } else {
            true
        }
    });
    for e in extensions {
        out.push_str(e);
        out.push('\n');
    }
    out.push_str("#define FERRITE_VULKAN 1\n#define gl_VertexID gl_VertexIndex\n#define gl_InstanceID gl_InstanceIndex\n");
    if vertex {
        // The depth prepass and the passes after it draw the same surfaces
        // with other programs and test them `LEQUAL`: the positions must
        // come out the same to the bit (GL's driver does that unasked).
        out.push_str("invariant gl_Position;\n");
    }
    out.push_str(block);
    for line in rest {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VS: &str = "#version 330 core
layout (location = 0) in vec3 position;
uniform mat4 viewProjection; // the camera
uniform float kernel[4];
uniform vec3 offset;
out vec2 uv;
void main() {
    gl_Position = viewProjection * vec4(position + offset, 1.0);
    uv = vec2(gl_VertexID);
}
";
    const FS: &str = "#version 460 core
in vec2 uv;
out vec4 color;
uniform vec3 offset;
uniform int mode;
uniform sampler2D image;
layout(binding = 17) uniform sampler3D noise;
layout(std140, binding = 1) uniform Lights {
    vec4 data[4];
};
layout(std430, binding = 3) readonly buffer Ranges { uint ranges[]; };
void main() { color = texture(image, uv) * float(mode) + vec4(offset, 1.0); }
";

    #[test]
    fn loose_uniforms_of_both_stages_share_one_std140_block() {
        let p = to_vulkan(VS, FS).unwrap();
        let layout: Vec<(&str, u32, u32)> = p.params.iter().map(|p| (p.name.as_str(), p.offset, p.stride)).collect();
        assert_eq!(layout, [("viewProjection", 0, 64), ("kernel", 64, 16), ("offset", 128, 12), ("mode", 140, 4)]);
        assert_eq!(p.blocks, [144]);
        assert!(p.vertex.contains("uniform FerriteParams"));
        assert!(p.fragment.contains("uniform FerriteParams"));
        assert!(!p.vertex.contains("uniform mat4 viewProjection;"));
        assert!(p.vertex.starts_with("#version 460\n"));
        assert!(p.vertex.contains("#define gl_VertexID gl_VertexIndex"));
        assert!(p.vertex.contains("invariant gl_Position;"));
        assert!(!p.fragment.contains("invariant gl_Position;"));
    }

    #[test]
    fn samplers_get_bindings_and_blocks_get_sets() {
        let p = to_vulkan(VS, FS).unwrap();
        let image = p.sampler("image").unwrap();
        assert_eq!((image.binding, image.fixed_unit), (0, None));
        let noise = p.sampler("noise").unwrap();
        assert_eq!(noise.fixed_unit, Some(17));
        assert_eq!((image.ty.as_str(), noise.ty.as_str()), ("sampler2D", "sampler3D"));
        assert!(p.fragment.contains("layout(set = 0, binding = 0) uniform sampler2D image;"));
        assert!(p.fragment.contains("layout(set = 0, binding = 17) uniform sampler3D noise;"));
        assert!(p.fragment.contains("layout(std140, set = 0, binding = 33) uniform Lights"), "{}", p.fragment);
        assert!(p.fragment.contains("layout(std430, set = 0, binding = 35) readonly buffer Ranges"), "{}", p.fragment);
        assert_eq!(p.buffers.len(), 2);
    }

    #[test]
    fn mat4_arrays_and_defined_lengths() {
        let vs = "#version 460 core\n#define MAX_BONES 100\nuniform mat4 bones[MAX_BONES];\nuniform float a;\nvoid main(){}\n";
        let p = to_vulkan(vs, "#version 460 core\nvoid main(){}\n").unwrap();
        // The big array has a block of its own; the float is in block 0.
        assert_eq!((p.params[0].count, p.params[0].block, p.params[0].offset), (100, 1, 0));
        assert_eq!((p.params[1].block, p.params[1].offset), (0, 0));
        assert_eq!(p.blocks, [16, 6400]);
        assert!(p.vertex.contains("set = 1, binding = 1) uniform FerriteParams1"));
    }

    /// Every program dumped with `FERRITE_SHADER_DUMP` (from a run of the
    /// editor) rewritten and compiled for Vulkan with `glslc`:
    /// `FERRITE_SHADER_DUMP=<dir> cargo test -p ferrite --lib dumped_programs -- --ignored`.
    #[test]
    #[ignore]
    fn dumped_programs_compile_for_vulkan() {
        let dir = std::env::var("FERRITE_SHADER_DUMP").expect("FERRITE_SHADER_DUMP");
        let out = std::env::temp_dir().join("ferrite-spirv-check");
        std::fs::create_dir_all(&out).unwrap();
        let mut failures = Vec::new();
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("vert") {
                continue;
            }
            count += 1;
            let vertex = std::fs::read_to_string(&path).unwrap();
            let fragment = std::fs::read_to_string(path.with_extension("frag")).unwrap();
            let label = std::fs::read_to_string(path.with_extension("label")).unwrap_or_default();
            let program = match to_vulkan(&vertex, &fragment) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!("{label}: {e}"));
                    continue;
                }
            };
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            for (stage, source) in [("vert", &program.vertex), ("frag", &program.fragment)] {
                let file = out.join(format!("{stem}.{stage}"));
                std::fs::write(&file, source).unwrap();
                let result = std::process::Command::new("glslc")
                    .args(["--target-env=vulkan1.3", "-fauto-map-locations", "-o"])
                    .arg(out.join(format!("{stem}.{stage}.spv")))
                    .arg(&file)
                    .output()
                    .expect("glslc");
                if !result.status.success() {
                    failures.push(format!("{label} ({stage}, {}):\n{}", file.display(), String::from_utf8_lossy(&result.stderr)));
                }
            }
        }
        assert!(count > 0, "no programs in {dir}");
        assert!(failures.is_empty(), "{} of {count} programs fail:\n{}", failures.len(), failures.join("\n"));
    }
}
