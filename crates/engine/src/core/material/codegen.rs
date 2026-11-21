//! Material graph → GLSL.
//!
//! A graph gives up to three pieces of code, put into the engine's shader
//! templates (see `programs`):
//!
//! - `evaluateMaterial(inout Surface s, MaterialInputs i)` for the shading
//!   pass: the opacity first (and, for a cut-out material, the `discard`),
//!   then a return in the shadow pass, then the other channels;
//! - `materialOpacity(MaterialInputs i)` for depth-only programs of
//!   cut-out materials;
//! - `materialWorldOffset(MaterialInputs i)` for every vertex shader.
//!
//! Each node becomes one local variable (`n<id>`), emitted after the
//! nodes it reads; nodes no output reaches are left out. Parameters
//! become uniforms `mp_<name>` (textures: a sampler and `mp_<name>_set`),
//! graphics levels `ml_<setting>`.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Write,
};

use super::graph::*;

/// Most texture parameters a graph may have (units, see `TEXTURE_UNITS`).
pub const MAX_TEXTURES: usize = 12;

/// The texture units material textures are bound to, in parameter order:
/// the units the engine's lighting leaves free.
pub const TEXTURE_UNITS: [u32; MAX_TEXTURES] = [0, 1, 2, 3, 4, 18, 19, 20, 21, 22, 23, 24];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// How the fragment code is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Variant {
    /// Discard below `materialCutoff`.
    pub cutout: bool,
    /// Textures read at full detail (`textureLod(.., 0)`), for pictures
    /// much smaller than the textures (impostors).
    pub lod0: bool,
}

/// What a graph compiled to.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    /// Uniforms and `evaluateMaterial`.
    pub fragment: String,
    /// Uniforms and `materialOpacity`; `None` when nothing is cut out.
    pub opacity: Option<String>,
    /// Uniforms and `materialWorldOffset`.
    pub vertex: String,
    /// Whether vertices move (the offset is wired).
    pub moves_vertices: bool,
}

/// Uniform name of a parameter.
pub fn param_uniform(name: &str) -> String {
    format!("mp_{name}")
}

/// Uniform name of a texture parameter's "is set" flag.
pub fn param_set_uniform(name: &str) -> String {
    format!("mp_{name}_set")
}

/// Uniform name of a graphics setting's level.
pub fn level_uniform(setting: &str) -> String {
    let clean: String = setting
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("ml_{clean}")
}

/// Texture unit of each texture parameter, by name.
pub fn texture_units(graph: &MaterialGraph) -> Vec<(String, u32)> {
    graph
        .params
        .iter()
        .filter(|p| matches!(p.ty, ParamType::Texture { .. }))
        .zip(TEXTURE_UNITS)
        .map(|(p, unit)| (p.name.clone(), unit))
        .collect()
}

/// Compile `graph`. Fails with its problems.
pub fn generate(graph: &MaterialGraph, variant: Variant) -> Result<Generated, Vec<GraphProblem>> {
    let problems = check(graph);
    if !problems.is_empty() {
        return Err(problems);
    }
    let output = graph.output_node().expect("checked").id;
    let wired = |pin: u8| graph.wire_into(output, pin).is_some();

    // Fragment.
    let mut gen = Gen::new(graph, Stage::Fragment, variant);
    let mut body = String::new();
    if wired(output::OPACITY) {
        let opacity = gen.input(output, output::OPACITY, 1);
        body.push_str(&gen.take_lines());
        let _ = writeln!(body, "    s.opacity = {opacity};");
        if variant.cutout {
            body.push_str("    if (s.opacity < materialCutoff) {\n        discard;\n    }\n");
        }
    }
    body.push_str("    if (isShadowPass == 1) {\n        return;\n    }\n");
    let channels = [
        (output::BASE_COLOR, "baseColor", 3),
        (output::NORMAL, "normal", 3),
        (output::ROUGHNESS, "roughness", 1),
        (output::METALLIC, "metallic", 1),
        (output::OCCLUSION, "occlusion", 1),
        (output::EMISSIVE, "emissive", 3),
    ];
    let mut assignments = String::new();
    for (pin, field, width) in channels {
        if wired(pin) {
            let expr = gen.input(output, pin, width);
            let _ = writeln!(assignments, "    s.{field} = {expr};");
        }
    }
    body.push_str(&gen.take_lines());
    body.push_str(&assignments);
    let mut fragment = gen.uniforms();
    let _ = write!(fragment, "void evaluateMaterial(inout Surface s, MaterialInputs i) {{\n{body}}}\n");

    // Opacity alone, for depth-only programs.
    let opacity = (variant.cutout && wired(output::OPACITY)).then(|| {
        let mut gen = Gen::new(graph, Stage::Fragment, variant);
        let expr = gen.input(output, output::OPACITY, 1);
        let lines = gen.take_lines();
        let mut code = gen.uniforms();
        let _ = write!(code, "float materialOpacity(MaterialInputs i) {{\n{lines}    return {expr};\n}}\n");
        code
    });

    // Vertex.
    let mut gen = Gen::new(graph, Stage::Vertex, variant);
    let moves_vertices = wired(output::WORLD_OFFSET);
    let expr = if moves_vertices {
        gen.input(output, output::WORLD_OFFSET, 3)
    } else {
        "vec3(0.0)".to_string()
    };
    let lines = gen.take_lines();
    let mut vertex = gen.uniforms();
    let _ = write!(vertex, "vec3 materialWorldOffset(MaterialInputs i) {{\n{lines}    return {expr};\n}}\n");

    Ok(Generated { fragment, opacity, vertex, moves_vertices })
}

struct Gen<'a> {
    graph: &'a MaterialGraph,
    stage: Stage,
    variant: Variant,
    lines: String,
    emitted: HashSet<NodeId>,
    params: BTreeSet<String>,
    levels: BTreeSet<String>,
    widths: HashMap<(NodeId, u8), u8>,
}

impl<'a> Gen<'a> {
    fn new(graph: &'a MaterialGraph, stage: Stage, variant: Variant) -> Self {
        Gen {
            graph,
            stage,
            variant,
            lines: String::new(),
            emitted: HashSet::new(),
            params: BTreeSet::new(),
            levels: BTreeSet::new(),
            widths: HashMap::new(),
        }
    }

    fn take_lines(&mut self) -> String {
        std::mem::take(&mut self.lines)
    }

    /// Declarations of what the code read.
    fn uniforms(&self) -> String {
        let mut out = String::new();
        for name in &self.params {
            let Some(decl) = self.graph.param(name) else { continue };
            match decl.ty {
                ParamType::Texture { .. } => {
                    let _ = writeln!(out, "uniform sampler2D {};", param_uniform(name));
                    let _ = writeln!(out, "uniform int {};", param_set_uniform(name));
                }
                ty => {
                    let _ = writeln!(out, "uniform {} {};", ty.value_type().glsl(), param_uniform(name));
                }
            }
        }
        for setting in &self.levels {
            let _ = writeln!(out, "uniform int {};", level_uniform(setting));
        }
        out
    }

    /// The expression for `node`'s input `pin`, as `width` components.
    fn input(&mut self, node: NodeId, pin: u8, width: u8) -> String {
        let (expr, from) = self.input_raw(node, pin);
        convert(&expr, from, width)
    }

    /// The expression for an input and its own width.
    fn input_raw(&mut self, node: NodeId, pin: u8) -> (String, u8) {
        if let Some(wire) = self.graph.wire_into(node, pin).copied() {
            let width = out_width(self.graph, wire.from.node, wire.from.pin, &mut self.widths, &mut Vec::new());
            let expr = self.output(wire.from.node, wire.from.pin);
            return (expr, width);
        }
        let Some(n) = self.graph.node(node) else {
            return ("0.0".into(), 1);
        };
        let pin_def = n.kind.inputs().get(pin as usize).copied();
        if let Some(implicit) = pin_def.and_then(|p| p.implicit) {
            return match implicit {
                DefaultInput::Uv => ("i.uv".into(), 2),
                DefaultInput::Normal => ("i.normal".into(), 3),
                DefaultInput::WorldPosition => ("i.worldPos".into(), 3),
            };
        }
        let value = n.input_value(pin);
        let width = match pin_def.and_then(|p| p.ty) {
            Some(ValueType::Texture) => 0,
            Some(ty) => ty.width(),
            None => 1,
        };
        (literal(value, width.max(1)), width.max(1))
    }

    /// The expression for `node`'s output `pin`, emitting the node first.
    fn output(&mut self, node: NodeId, pin: u8) -> String {
        let Some(n) = self.graph.node(node) else {
            return "0.0".into();
        };
        let kind = n.kind.clone();
        // Nodes that are plain expressions are not given a variable.
        match &kind {
            NodeKind::Parameter(name) => {
                self.params.insert(name.clone());
                return param_uniform(name);
            }
            NodeKind::Constant { value, width } => return literal(*value, (*width).clamp(1, 4)),
            NodeKind::Uv => return "i.uv".into(),
            NodeKind::Position(Space::World) => return "i.worldPos".into(),
            NodeKind::Position(Space::Object) => return "i.objectPos".into(),
            NodeKind::Normal(Space::World) => return "i.normal".into(),
            NodeKind::Normal(Space::Object) => return "i.objectNormal".into(),
            NodeKind::ViewDirection => return "normalize(cameraPosition - i.worldPos)".into(),
            NodeKind::Time => return "wind.w".into(),
            NodeKind::VertexColor => return "i.color".into(),
            NodeKind::InstanceTint => return "i.tint".into(),
            NodeKind::HeightOverPivot => return "(i.worldPos.y - i.pivot.y)".into(),
            _ => {}
        }
        let var = format!("n{}", node.0);
        if !self.emitted.contains(&node) {
            self.emitted.insert(node);
            self.emit(node, &kind, &var);
        }
        match kind {
            NodeKind::SampleTexture => match pin {
                0 => var,
                1 => format!("{var}.rgb"),
                2 => format!("{var}.r"),
                3 => format!("{var}.g"),
                4 => format!("{var}.b"),
                _ => format!("{var}.a"),
            },
            NodeKind::Split => format!("{var}.{}", ["x", "y", "z", "w"][(pin as usize).min(3)]),
            NodeKind::Combine => match pin {
                0 => var,
                1 => format!("{var}.xyz"),
                _ => format!("{var}.xy"),
            },
            _ => var,
        }
    }

    fn emit(&mut self, node: NodeId, kind: &NodeKind, var: &str) {
        let width = out_width(self.graph, node, 0, &mut self.widths, &mut Vec::new());
        // Nodes with several outputs keep the whole vector.
        let ty = match kind {
            NodeKind::SampleTexture | NodeKind::Split | NodeKind::Combine => "vec4",
            _ => ValueType::of_width(width).glsl(),
        };
        let expr = match kind {
            NodeKind::SampleTexture => {
                let texture = self.texture_input(node, 0);
                let uv = self.input(node, 1, 2);
                let fallback = self.input(node, 2, 4);
                match texture {
                    Some(name) => {
                        let read = if self.stage == Stage::Vertex || self.variant.lod0 {
                            format!("textureLod({}, {uv}, 0.0)", param_uniform(&name))
                        } else {
                            format!("texture({}, {uv})", param_uniform(&name))
                        };
                        format!("({} == 1 ? {read} : {fallback})", param_set_uniform(&name))
                    }
                    None => fallback,
                }
            }
            NodeKind::NormalMap => {
                let texture = self.texture_input(node, 0);
                let uv = self.input(node, 1, 2);
                let strength = self.input(node, 2, 1);
                match texture {
                    Some(name) => format!(
                        "materialNormalMap({}, {}, i.normal, i.worldPos, {uv}, {strength})",
                        param_uniform(&name),
                        param_set_uniform(&name)
                    ),
                    None => "i.normal".into(),
                }
            }
            NodeKind::Math(op) => self.math(node, *op, width),
            NodeKind::Split => self.input(node, 0, 4),
            NodeKind::Combine => {
                let parts: Vec<String> = (0..4).map(|pin| self.input(node, pin, 1)).collect();
                format!("vec4({})", parts.join(", "))
            }
            NodeKind::Remap => {
                let x = self.input(node, 0, width);
                let a = self.input(node, 1, width);
                let b = self.input(node, 2, width);
                let c = self.input(node, 3, width);
                let d = self.input(node, 4, width);
                format!("mix({c}, {d}, ({x} - {a}) / ({b} - {a}))")
            }
            NodeKind::Fresnel => {
                let normal = self.input(node, 0, 3);
                let power = self.input(node, 1, 1);
                format!(
                    "pow(1.0 - clamp(dot(normalize({normal}), normalize(cameraPosition - i.worldPos)), 0.0, 1.0), {power})"
                )
            }
            NodeKind::Noise => {
                let position = self.input(node, 0, 3);
                let scale = self.input(node, 1, 1);
                format!("materialNoise({position} * {scale})")
            }
            NodeKind::BendNormalUp => {
                let normal = self.input(node, 0, 3);
                let amount = self.input(node, 1, 1);
                format!("normalize(mix({normal}, vec3(0.0, 1.0, 0.0), {amount}))")
            }
            NodeKind::GraphicsLevel(setting) => {
                self.levels.insert(setting.clone());
                let level = level_uniform(setting);
                let v: Vec<String> = (0..4).map(|pin| self.input(node, pin, width)).collect();
                format!(
                    "({level} == 0 ? {} : {level} == 1 ? {} : {level} == 2 ? {} : {})",
                    v[0], v[1], v[2], v[3]
                )
            }
            NodeKind::WindSway => {
                let v: Vec<String> = (0..5).map(|pin| self.input(node, pin, 1)).collect();
                format!(
                    "materialWind(i.worldPos, i.pivot, i.normal, {}, {}, {}, {}, {})",
                    v[0], v[1], v[2], v[3], v[4]
                )
            }
            // Plain expressions, handled in `output`.
            _ => "0.0".into(),
        };
        let _ = writeln!(self.lines, "    {ty} {var} = {expr};");
    }

    fn math(&mut self, node: NodeId, op: MathOp, width: u8) -> String {
        use MathOp::*;
        // Dot reads its inputs at their common width.
        let w = if op == Dot { input_width(self.graph, node, &mut self.widths) } else { width };
        let mut arg = |pin: u8| self.input(node, pin, w);
        match op {
            Add => format!("({} + {})", arg(0), arg(1)),
            Subtract => format!("({} - {})", arg(0), arg(1)),
            Multiply => format!("({} * {})", arg(0), arg(1)),
            Divide => format!("({} / {})", arg(0), arg(1)),
            Min => format!("min({}, {})", arg(0), arg(1)),
            Max => format!("max({}, {})", arg(0), arg(1)),
            Power => format!("pow({}, {})", arg(0), arg(1)),
            Dot => format!("dot({}, {})", arg(0), arg(1)),
            Step => format!("step({}, {})", arg(0), arg(1)),
            OneMinus => format!("(1.0 - {})", arg(0)),
            Abs => format!("abs({})", arg(0)),
            Saturate => format!("clamp({}, 0.0, 1.0)", arg(0)),
            Normalize => format!("normalize({})", arg(0)),
            Fract => format!("fract({})", arg(0)),
            Sin => format!("sin({})", arg(0)),
            Cos => format!("cos({})", arg(0)),
            Sqrt => format!("sqrt({})", arg(0)),
            Lerp => {
                let a = arg(0);
                let b = arg(1);
                let t = self.input(node, 2, 1);
                format!("mix({a}, {b}, {t})")
            }
            Clamp => format!("clamp({}, {}, {})", arg(0), arg(1), arg(2)),
            Smoothstep => format!("smoothstep({}, {}, {})", arg(0), arg(1), arg(2)),
        }
    }

    /// The texture parameter wired into a texture input.
    fn texture_input(&mut self, node: NodeId, pin: u8) -> Option<String> {
        let wire = self.graph.wire_into(node, pin)?;
        match &self.graph.node(wire.from.node)?.kind {
            NodeKind::Parameter(name) => {
                self.params.insert(name.clone());
                Some(name.clone())
            }
            _ => None,
        }
    }
}

/// A GLSL literal of `width` components.
fn literal(value: [f32; 4], width: u8) -> String {
    let f = |v: f32| {
        let v = if v.is_finite() { v } else { 0.0 };
        let s = format!("{v:?}");
        if s.contains('.') || s.contains('e') {
            s
        } else {
            format!("{s}.0")
        }
    };
    match width {
        1 => f(value[0]),
        2 => format!("vec2({}, {})", f(value[0]), f(value[1])),
        3 => format!("vec3({}, {}, {})", f(value[0]), f(value[1]), f(value[2])),
        _ => format!("vec4({}, {}, {}, {})", f(value[0]), f(value[1]), f(value[2]), f(value[3])),
    }
}

/// `expr` (of `from` components) as `to` components: a number fills every
/// component, a shorter vector is filled with 0 (w with 1), a longer one
/// cut.
pub fn convert(expr: &str, from: u8, to: u8) -> String {
    let from = from.max(1);
    let to = to.max(1);
    if from == to {
        return expr.to_string();
    }
    match (from, to) {
        (1, t) => format!("{}({expr})", ValueType::of_width(t).glsl()),
        (_, 1) => format!("({expr}).x"),
        (2, 3) => format!("vec3({expr}, 0.0)"),
        (2, 4) => format!("vec4({expr}, 0.0, 1.0)"),
        (3, 4) => format!("vec4({expr}, 1.0)"),
        (_, 2) => format!("({expr}).xy"),
        _ => format!("({expr}).xyz"),
    }
}

/// The widest input of `node` (unwired inputs count as numbers).
fn input_width(graph: &MaterialGraph, node: NodeId, memo: &mut HashMap<(NodeId, u8), u8>) -> u8 {
    let Some(n) = graph.node(node) else { return 1 };
    let mut width = 1;
    for (pin, def) in n.kind.inputs().iter().enumerate() {
        if def.ty.is_some() {
            continue;
        }
        if let Some(wire) = graph.wire_into(node, pin as u8) {
            width = width.max(out_width(graph, wire.from.node, wire.from.pin, memo, &mut Vec::new()));
        }
    }
    width
}

/// Components of `node`'s output `pin` (0 for a texture).
fn out_width(
    graph: &MaterialGraph,
    node: NodeId,
    pin: u8,
    memo: &mut HashMap<(NodeId, u8), u8>,
    visiting: &mut Vec<NodeId>,
) -> u8 {
    if let Some(&w) = memo.get(&(node, pin)) {
        return w;
    }
    if visiting.contains(&node) {
        return 1;
    }
    let Some(n) = graph.node(node) else { return 1 };
    visiting.push(node);
    let widest_any = |memo: &mut HashMap<(NodeId, u8), u8>, visiting: &mut Vec<NodeId>| {
        let mut width = 1;
        for (i, def) in n.kind.inputs().iter().enumerate() {
            if def.ty.is_some() {
                continue;
            }
            if let Some(wire) = graph.wire_into(node, i as u8) {
                width = width.max(out_width(graph, wire.from.node, wire.from.pin, memo, visiting));
            }
        }
        width
    };
    let w = match &n.kind {
        NodeKind::Parameter(name) => graph.param(name).map(|p| p.ty.value_type().width()).unwrap_or(1),
        NodeKind::Constant { width, .. } => (*width).clamp(1, 4),
        NodeKind::Uv => 2,
        NodeKind::Position(_) | NodeKind::Normal(_) | NodeKind::ViewDirection => 3,
        NodeKind::Time | NodeKind::HeightOverPivot => 1,
        NodeKind::VertexColor => 4,
        NodeKind::InstanceTint => 3,
        NodeKind::SampleTexture => match pin {
            0 => 4,
            1 => 3,
            _ => 1,
        },
        NodeKind::NormalMap | NodeKind::BendNormalUp | NodeKind::WindSway => 3,
        NodeKind::Math(MathOp::Dot) => 1,
        NodeKind::Math(_) | NodeKind::Remap | NodeKind::GraphicsLevel(_) => widest_any(memo, visiting),
        NodeKind::Split | NodeKind::Fresnel | NodeKind::Noise => 1,
        NodeKind::Combine => [4, 3, 2][(pin as usize).min(2)],
        NodeKind::Output => 0,
    };
    visiting.pop();
    memo.insert((node, pin), w);
    w
}

/// Everything wrong with `graph`.
pub fn check(graph: &MaterialGraph) -> Vec<GraphProblem> {
    let mut problems = Vec::new();
    let mut problem = |node: Option<NodeId>, message: String| problems.push(GraphProblem { node, message });

    // Parameters.
    let mut names = HashSet::new();
    for p in &graph.params {
        if !valid_param_name(&p.name) {
            problem(None, format!("Parametername \"{}\" ungültig (nur Buchstaben, Ziffern, _)", p.name));
        }
        if !names.insert(p.name.as_str()) {
            problem(None, format!("Parameter \"{}\" doppelt", p.name));
        }
    }
    let textures = graph.params.iter().filter(|p| matches!(p.ty, ParamType::Texture { .. })).count();
    if textures > MAX_TEXTURES {
        problem(None, format!("{textures} Texturparameter, höchstens {MAX_TEXTURES}"));
    }

    // Nodes.
    let outputs: Vec<&GraphNode> = graph.nodes.iter().filter(|n| n.kind == NodeKind::Output).collect();
    match outputs.len() {
        0 => problem(None, "kein Material-Ausgang".into()),
        1 => {}
        _ => problem(Some(outputs[1].id), "mehr als ein Material-Ausgang".into()),
    }
    let mut ids = HashSet::new();
    for n in &graph.nodes {
        if !ids.insert(n.id) {
            problem(Some(n.id), "Node-ID doppelt".into());
        }
        if let NodeKind::Parameter(name) = &n.kind {
            if graph.param(name).is_none() {
                problem(Some(n.id), format!("Parameter \"{name}\" gibt es nicht"));
            }
        }
    }

    // Wires.
    let mut inputs_taken = HashSet::new();
    let mut memo = HashMap::new();
    for w in &graph.wires {
        let (Some(from), Some(to)) = (graph.node(w.from.node), graph.node(w.to.node)) else {
            problem(None, "Verbindung zu einem Node, den es nicht gibt".into());
            continue;
        };
        if w.from.pin as usize >= from.kind.outputs().len() {
            problem(Some(from.id), format!("Ausgang {} gibt es nicht", w.from.pin));
            continue;
        }
        let Some(input) = to.kind.inputs().get(w.to.pin as usize) else {
            problem(Some(to.id), format!("Eingang {} gibt es nicht", w.to.pin));
            continue;
        };
        if !inputs_taken.insert(w.to) {
            problem(Some(to.id), format!("mehrere Verbindungen in \"{}\"", input.label));
        }
        let from_texture = out_width(graph, from.id, w.from.pin, &mut memo, &mut Vec::new()) == 0;
        let to_texture = input.ty == Some(ValueType::Texture);
        if from_texture && !to_texture {
            problem(Some(to.id), format!("\"{}\" nimmt keine Textur", input.label));
        }
        if to_texture && !from_texture {
            problem(Some(to.id), format!("\"{}\" braucht einen Texturparameter", input.label));
        }
    }

    // Cycles.
    if let Some(node) = find_cycle(graph) {
        problem(Some(node), "Kreis: der Node hängt von sich selbst ab".into());
    }

    // Stages.
    if let Some(out) = outputs.first() {
        let vertex = reached(graph, out.id, &[output::WORLD_OFFSET]);
        let fragment = reached(
            graph,
            out.id,
            &[
                output::BASE_COLOR,
                output::OPACITY,
                output::NORMAL,
                output::ROUGHNESS,
                output::METALLIC,
                output::OCCLUSION,
                output::EMISSIVE,
            ],
        );
        for n in &graph.nodes {
            if vertex.contains(&n.id) && n.kind.fragment_only() {
                problem(Some(n.id), "geht nicht im World Offset (nur pro Pixel)".into());
            }
            if fragment.contains(&n.id) && n.kind.vertex_only() {
                problem(Some(n.id), "geht nur im World Offset".into());
            }
            if fragment.contains(&n.id) && n.kind == NodeKind::Normal(Space::Object) {
                problem(Some(n.id), "Objektnormale gibt es nur im World Offset".into());
            }
        }
    }
    problems
}

/// Nodes `node`'s inputs `pins` read, directly or not.
pub fn reached(graph: &MaterialGraph, node: NodeId, pins: &[u8]) -> HashSet<NodeId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<NodeId> = graph
        .wires
        .iter()
        .filter(|w| w.to.node == node && pins.contains(&w.to.pin))
        .map(|w| w.from.node)
        .collect();
    while let Some(id) = stack.pop() {
        if seen.insert(id) {
            stack.extend(graph.wires.iter().filter(|w| w.to.node == id).map(|w| w.from.node));
        }
    }
    seen
}

fn find_cycle(graph: &MaterialGraph) -> Option<NodeId> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit(graph: &MaterialGraph, id: NodeId, marks: &mut HashMap<NodeId, Mark>) -> Option<NodeId> {
        match marks.get(&id) {
            Some(Mark::Done) => return None,
            Some(Mark::Visiting) => return Some(id),
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        for w in graph.wires.iter().filter(|w| w.to.node == id) {
            if let Some(found) = visit(graph, w.from.node, marks) {
                return Some(found);
            }
        }
        marks.insert(id, Mark::Done);
        None
    }
    let mut marks = HashMap::new();
    graph.nodes.iter().find_map(|n| visit(graph, n.id, &mut marks))
}
