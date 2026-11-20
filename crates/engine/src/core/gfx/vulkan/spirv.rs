//! Just enough SPIR-V reading for the backend: the stage interface
//! (inputs and outputs with their names, locations and scalar types), and
//! moving a fragment stage's input locations onto the vertex stage's.
//!
//! GL links a program's stages by variable name; `shaderc` compiles each
//! stage alone and numbers locations in declaration order
//! (`auto_map_locations`), so a fragment stage that declares fewer
//! varyings, or in another order, would read the wrong ones. The
//! fragment stage's inputs are given the locations its vertex stage gave
//! the outputs of the same names.

use std::collections::HashMap;

const OP_NAME: u32 = 5;
const OP_TYPE_INT: u32 = 21;
const OP_TYPE_FLOAT: u32 = 22;
const OP_TYPE_VECTOR: u32 = 23;
const OP_TYPE_MATRIX: u32 = 24;
const OP_TYPE_ARRAY: u32 = 28;
const OP_TYPE_POINTER: u32 = 32;
const OP_VARIABLE: u32 = 59;
const OP_DECORATE: u32 = 71;
const DECORATION_LOCATION: u32 = 30;

pub const STORAGE_INPUT: u32 = 1;
pub const STORAGE_OUTPUT: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Float,
    Int,
    Uint,
    Other,
}

/// A variable of the stage interface.
#[derive(Clone, Debug)]
pub struct Interface {
    pub name: String,
    pub location: u32,
    /// Index of the location's literal in the word stream.
    word: usize,
    pub scalar: Scalar,
}

fn string_at(words: &[u32]) -> String {
    let mut bytes = Vec::new();
    'outer: for w in words {
        for b in w.to_le_bytes() {
            if b == 0 {
                break 'outer;
            }
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The located variables of `storage` class (`STORAGE_INPUT` or
/// `STORAGE_OUTPUT`).
pub fn interface(words: &[u32], storage: u32) -> Vec<Interface> {
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut locations: HashMap<u32, (u32, usize)> = HashMap::new();
    let mut variables: Vec<(u32, u32)> = Vec::new();
    // Type id -> what it is.
    let mut scalars: HashMap<u32, Scalar> = HashMap::new();
    let mut composites: HashMap<u32, u32> = HashMap::new();
    let mut pointers: HashMap<u32, u32> = HashMap::new();
    let mut i = 5;
    while i < words.len() {
        let count = (words[i] >> 16) as usize;
        let op = words[i] & 0xFFFF;
        if count == 0 || i + count > words.len() {
            break;
        }
        let ins = &words[i..i + count];
        match op {
            OP_NAME if count >= 3 => {
                names.insert(ins[1], string_at(&ins[2..]));
            }
            OP_DECORATE if count >= 4 && ins[2] == DECORATION_LOCATION => {
                locations.insert(ins[1], (ins[3], i + 3));
            }
            OP_TYPE_INT if count >= 4 => {
                scalars.insert(ins[1], if ins[3] != 0 { Scalar::Int } else { Scalar::Uint });
            }
            OP_TYPE_FLOAT if count >= 3 => {
                scalars.insert(ins[1], Scalar::Float);
            }
            OP_TYPE_VECTOR | OP_TYPE_MATRIX | OP_TYPE_ARRAY if count >= 3 => {
                composites.insert(ins[1], ins[2]);
            }
            OP_TYPE_POINTER if count >= 4 => {
                pointers.insert(ins[1], ins[3]);
            }
            OP_VARIABLE if count >= 4 && ins[3] == storage => {
                variables.push((ins[2], ins[1]));
            }
            _ => {}
        }
        i += count;
    }
    let scalar_of = |mut ty: u32| {
        for _ in 0..8 {
            if let Some(s) = scalars.get(&ty) {
                return *s;
            }
            match composites.get(&ty) {
                Some(&inner) => ty = inner,
                None => break,
            }
        }
        Scalar::Other
    };
    variables
        .into_iter()
        .filter_map(|(id, pointer)| {
            let (location, word) = *locations.get(&id)?;
            let scalar = pointers.get(&pointer).map_or(Scalar::Other, |&t| scalar_of(t));
            Some(Interface { name: names.get(&id).cloned().unwrap_or_default(), location, word, scalar })
        })
        .collect()
}

/// Give `fragment`'s inputs the locations `vertex` gave its outputs of the
/// same names.
pub fn link_locations(vertex: &[u32], fragment: &mut [u32]) {
    let outputs: HashMap<String, u32> =
        interface(vertex, STORAGE_OUTPUT).into_iter().map(|v| (v.name, v.location)).collect();
    for input in interface(fragment, STORAGE_INPUT) {
        if let Some(&location) = outputs.get(&input.name) {
            fragment[input.word] = location;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &str, kind: shaderc::ShaderKind) -> Vec<u32> {
        let compiler = shaderc::Compiler::new().unwrap();
        let mut options = shaderc::CompileOptions::new().unwrap();
        options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_3 as u32);
        options.set_auto_map_locations(true);
        compiler.compile_into_spirv(source, kind, "0", "main", Some(&options)).unwrap().as_binary().to_vec()
    }

    #[test]
    fn fragment_inputs_take_the_vertex_outputs_locations_by_name() {
        let vs = compile(
            "#version 460\nout vec3 Normal;\nout vec3 WorldPos;\nflat out ivec2 Ids;\nout vec2 TexCoords;\nvoid main(){Normal=vec3(0);WorldPos=vec3(1);Ids=ivec2(0);TexCoords=vec2(0);gl_Position=vec4(0);}\n",
            shaderc::ShaderKind::Vertex,
        );
        let mut fs = compile(
            "#version 460\nin vec2 TexCoords;\nout vec4 c;\nvoid main(){c=vec4(TexCoords,0,1);}\n",
            shaderc::ShaderKind::Fragment,
        );
        assert_eq!(interface(&fs, STORAGE_INPUT)[0].location, 0);
        link_locations(&vs, &mut fs);
        let outputs = interface(&vs, STORAGE_OUTPUT);
        let tex = outputs.iter().find(|v| v.name == "TexCoords").unwrap().location;
        assert_eq!(tex, 3);
        assert_eq!(interface(&fs, STORAGE_INPUT)[0].location, tex);
        let ids = outputs.iter().find(|v| v.name == "Ids").unwrap();
        assert_eq!(ids.scalar, Scalar::Int);
    }
}
