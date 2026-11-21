//! A material graph's programs: its code put into the engine's shader
//! templates, one program per (graph, variant, pipeline), built on first
//! use and kept.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::core::renderer::{depth_pass, pbr, shader::Shader};

use super::{codegen::Variant, CompiledGraph};

/// How a material is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pipeline {
    /// A model (skinned or not), shaded.
    Mesh,
    /// A model in a depth-only pass.
    MeshDepth,
    /// Scatter instances, shaded.
    Instanced,
    /// Scatter instances in a depth-only pass.
    InstancedDepth,
    /// A scatter model pictured for far away (colour and normal).
    ImpostorBake,
}

pub const INPUTS_GLSL: &str = include_str!("inputs.glsl");
pub const VERTEX_PRELUDE_GLSL: &str = include_str!("vertex_prelude.glsl");
pub const FRAGMENT_PRELUDE_GLSL: &str = include_str!("fragment_prelude.glsl");
pub const SURFACE_GLSL: &str = include_str!("surface.glsl");

const MESH_VERTEX: &str = include_str!("../model/vertex.glsl");
const MESH_FRAGMENT: &str = include_str!("../model/fragment.glsl");
const INSTANCED_VERTEX: &str = include_str!("../../terrain/scatter/scatter_instanced_vertex.glsl");
const INSTANCED_FRAGMENT: &str = include_str!("../../terrain/scatter/scatter_instanced_fragment.glsl");
const BAKE_VERTEX: &str = include_str!("../../terrain/scatter/impostor_bake_vertex.glsl");
const BAKE_FRAGMENT: &str = include_str!("../../terrain/scatter/impostor_bake_fragment.glsl");

thread_local! {
    static PROGRAMS: RefCell<HashMap<(u64, Variant, Pipeline), Rc<Shader>>> = RefCell::new(HashMap::new());
}

/// The program of `graph` (which must not be broken) for `pipeline`.
pub fn program(graph: &CompiledGraph, variant: Variant, pipeline: Pipeline) -> Rc<Shader> {
    // Depth-only programs of what is not cut out share one variant.
    let variant = match pipeline {
        Pipeline::MeshDepth | Pipeline::InstancedDepth => Variant { lod0: false, ..variant },
        _ => variant,
    };
    let key = (graph.key, variant, pipeline);
    if let Some(shader) = PROGRAMS.with(|p| p.borrow().get(&key).cloned()) {
        return shader;
    }
    let (vertex, fragment) = match graph.code(variant) {
        Some(code) => sources(&code, pipeline),
        None => {
            // Not reached for checked graphs; draw nothing rather than panic.
            log::warn!("material graph {}: no code", graph.name);
            (String::new(), String::new())
        }
    };
    let shader = Rc::new(Shader::new(&vertex, &fragment));
    super::forget_bound();
    PROGRAMS.with(|p| p.borrow_mut().insert(key, shader.clone()));
    shader
}

/// Drop every program (the context goes away, or shaders changed).
/// Materials still holding one keep it alive until they go.
pub fn release() {
    let _ = PROGRAMS.try_with(|p| p.borrow_mut().clear());
    super::forget_bound();
}

/// How many programs are built.
pub fn count() -> usize {
    PROGRAMS.with(|p| p.borrow().len())
}

/// Vertex and fragment source of `code` for `pipeline`.
pub fn sources(code: &super::Generated, pipeline: Pipeline) -> (String, String) {
    let vertex_template = match pipeline {
        Pipeline::Mesh | Pipeline::MeshDepth => MESH_VERTEX,
        Pipeline::Instanced | Pipeline::InstancedDepth => INSTANCED_VERTEX,
        Pipeline::ImpostorBake => BAKE_VERTEX,
    };
    let vertex = format!(
        "#version 460 core\n{INPUTS_GLSL}\n{VERTEX_PRELUDE_GLSL}\n{}\n{vertex_template}",
        code.vertex
    );
    let shaded = |body: &str| {
        format!(
            "#version 460 core\n{}\n{INPUTS_GLSL}\n{FRAGMENT_PRELUDE_GLSL}\n{SURFACE_GLSL}\n{}\n{}\n{body}",
            pbr::LIGHTING_GLSL,
            pbr::MATERIAL_GLSL,
            code.fragment
        )
    };
    let fragment = match pipeline {
        Pipeline::Mesh => shaded(MESH_FRAGMENT),
        Pipeline::Instanced => shaded(INSTANCED_FRAGMENT),
        Pipeline::MeshDepth | Pipeline::InstancedDepth => match &code.opacity {
            Some(opacity) => format!(
                "#version 460 core\n{INPUTS_GLSL}\n{FRAGMENT_PRELUDE_GLSL}\nuniform vec3 cameraPosition;\n{opacity}\n\
                 void main() {{\n    if (materialOpacity(materialInputs()) < materialCutoff) {{\n        discard;\n    }}\n}}\n"
            ),
            None => depth_pass::EMPTY_FRAGMENT.to_string(),
        },
        Pipeline::ImpostorBake => format!(
            "#version 460 core\n{INPUTS_GLSL}\n{FRAGMENT_PRELUDE_GLSL}\nuniform vec3 cameraPosition;\nconst int isShadowPass = 0;\n\
             {SURFACE_GLSL}\n{}\n{BAKE_FRAGMENT}",
            code.fragment
        ),
    };
    (vertex, fragment)
}
