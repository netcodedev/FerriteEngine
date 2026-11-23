//! Materials: what a model's parts are drawn with. See docs/materials.md.
//!
//! A `Material` names a material graph (`graph`) and sets its parameters
//! (textures, colours, numbers) and how it is drawn (cut-out, two-sided,
//! shadows). Many materials share a graph; its code is compiled once per
//! kind of program (`programs`) and the parameters are uniforms.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use serde::{Deserialize, Serialize};

use crate::core::{
    graphics::{self, Level, Param},
    renderer::{shader::Shader, texture::Texture},
};

pub mod asset;
pub mod codegen;
pub mod edit;
pub mod graph;
pub mod preview;
pub mod programs;
pub mod standard;
#[cfg(test)]
mod tests;
pub mod wind;

pub use codegen::{Generated, Variant};
pub use graph::{MaterialGraph, ParamDecl, ParamType};
pub use programs::Pipeline;

/// How a material's opacity is used.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    /// Every fragment drawn.
    Opaque,
    /// Fragments below this opacity are cut out (leaves, fences).
    Cutout(f32),
}

impl Default for AlphaMode {
    fn default() -> Self {
        AlphaMode::Opaque
    }
}

/// A graph, checked and ready to compile.
pub struct CompiledGraph {
    pub graph: MaterialGraph,
    /// Name shown in lists (the file's name, or the builtin's).
    pub name: String,
    /// Changes with the graph's content: programs are cached by it.
    pub key: u64,
    pub problems: Vec<graph::GraphProblem>,
    /// Per parameter: its uniform's name and, for textures, the flag's
    /// name and unit.
    uniforms: Vec<(String, Option<(String, u32)>)>,
    /// Graphics settings its level nodes read, with their uniforms.
    levels: Vec<(String, String)>,
    code: RefCell<HashMap<Variant, Option<Rc<Generated>>>>,
    /// Whether the output's opacity is wired.
    opacity_wired: bool,
}

impl CompiledGraph {
    pub fn new(name: &str, graph: MaterialGraph) -> Rc<Self> {
        let problems = graph.problems();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            graph.to_ron().hash(&mut hasher);
            hasher.finish()
        };
        let units: HashMap<String, u32> = codegen::texture_units(&graph).into_iter().collect();
        let uniforms = graph
            .params
            .iter()
            .map(|p| {
                let texture = units
                    .get(&p.name)
                    .map(|unit| (codegen::param_set_uniform(&p.name), *unit));
                (codegen::param_uniform(&p.name), texture)
            })
            .collect();
        let levels = graph
            .settings_used()
            .into_iter()
            .map(|s| {
                let uniform = codegen::level_uniform(&s);
                (s, uniform)
            })
            .collect();
        let opacity_wired = graph
            .output_node()
            .is_some_and(|o| graph.wire_into(o.id, graph::output::OPACITY).is_some());
        Rc::new(CompiledGraph {
            opacity_wired,
            graph,
            name: name.to_string(),
            key,
            problems,
            uniforms,
            levels,
            code: RefCell::new(HashMap::new()),
        })
    }

    pub fn is_broken(&self) -> bool {
        !self.problems.is_empty()
    }

    /// The code for `variant`; `None` for a broken graph.
    pub fn code(&self, variant: Variant) -> Option<Rc<Generated>> {
        self.code
            .borrow_mut()
            .entry(variant)
            .or_insert_with(|| codegen::generate(&self.graph, variant).ok().map(Rc::new))
            .clone()
    }

    /// Whether the graph wires an opacity (a cut-out can use it).
    pub fn has_opacity(&self) -> bool {
        self.opacity_wired
    }

    fn param_index(&self, name: &str) -> Option<usize> {
        self.graph.params.iter().position(|p| p.name == name)
    }
}

thread_local! {
    static STANDARD: Rc<CompiledGraph> = CompiledGraph::new(standard::STANDARD_PBR, standard::standard_pbr());
}

/// The builtin Standard PBR graph.
pub fn standard_graph() -> Rc<CompiledGraph> {
    STANDARD.with(Rc::clone)
}

/// The value of one parameter of a material.
pub enum ParamValue {
    /// Numbers (a float, vector or colour; a switch is 0 or 1). A float
    /// may follow a graphics level.
    Number(Param<f32>, [f32; 4]),
    Texture(Option<Texture>),
}

impl Clone for ParamValue {
    fn clone(&self) -> Self {
        match self {
            ParamValue::Number(param, v) => ParamValue::Number(param.clone(), *v),
            ParamValue::Texture(t) => ParamValue::Texture(t.as_ref().map(Texture::share)),
        }
    }
}

impl ParamValue {
    fn default_for(decl: &ParamDecl) -> Self {
        match decl.ty {
            ParamType::Texture { .. } => ParamValue::Texture(None),
            _ => ParamValue::Number(Param::Fixed(decl.default[0]), decl.default),
        }
    }

    /// The numbers, the first resolved for the current graphics settings.
    fn numbers(&self) -> [f32; 4] {
        match self {
            ParamValue::Number(Param::Fixed(_), v) => *v,
            ParamValue::Number(param, v) => {
                let mut v = *v;
                v[0] = param.resolve(&graphics::current());
                v
            }
            ParamValue::Texture(_) => [0.0; 4],
        }
    }
}

/// What a model part is drawn with.
pub struct Material {
    pub name: String,
    graph: Rc<CompiledGraph>,
    /// One per parameter of `graph`, in its order.
    values: Vec<ParamValue>,
    pub alpha: AlphaMode,
    /// Back faces drawn too (leaves, cards).
    pub two_sided: bool,
    /// Left out of collision shapes.
    pub foliage: bool,
    /// Casts shadows.
    pub shadows: bool,
    /// Programs looked up already, by pipeline and cut-out (a model draws
    /// each material every pass: no global lookup per mesh).
    programs: RefCell<Vec<(Pipeline, bool, Rc<Shader>)>>,
    /// `wind_sway` as last worked out, and the graphics settings'
    /// generation it holds for (asked for every collected mesh each frame).
    wind: Cell<Option<(u64, Option<[f32; 3]>)>>,
}

impl Clone for Material {
    fn clone(&self) -> Self {
        Material {
            name: self.name.clone(),
            graph: self.graph.clone(),
            values: self.values.clone(),
            alpha: self.alpha,
            two_sided: self.two_sided,
            foliage: self.foliage,
            shadows: self.shadows,
            programs: RefCell::new(self.programs.borrow().clone()),
            wind: Cell::new(None),
        }
    }
}

impl Default for Material {
    fn default() -> Self {
        Material::standard()
    }
}

impl Material {
    /// A material of `graph` with every parameter at its default.
    pub fn new(graph: Rc<CompiledGraph>) -> Self {
        let values = graph.graph.params.iter().map(ParamValue::default_for).collect();
        Material {
            name: String::new(),
            graph,
            values,
            alpha: AlphaMode::Opaque,
            two_sided: false,
            foliage: false,
            shadows: true,
            programs: RefCell::new(Vec::new()),
            wind: Cell::new(None),
        }
    }

    /// Standard PBR, plain white.
    pub fn standard() -> Self {
        Material::new(standard_graph())
    }

    /// How the graph's wind node sways the material, for ray traced
    /// shadows that follow the wind: strength, flutter and bend height
    /// (`materialWind` in `vertex_prelude.glsl`), with each mask folded
    /// in where it is a fixed number (a mask that varies over the surface,
    /// such as a vertex colour, counts as 1). `None` without a wind node,
    /// or when it neither sways nor flutters.
    pub fn wind_sway(&self) -> Option<[f32; 3]> {
        let generation = graphics::generation();
        if let Some((at, wind)) = self.wind.get() {
            if at == generation {
                return wind;
            }
        }
        let wind = self.find_wind_sway();
        self.wind.set(Some((generation, wind)));
        wind
    }

    fn find_wind_sway(&self) -> Option<[f32; 3]> {
        use graph::NodeKind;
        let graph = &self.graph.graph;
        let node = graph.nodes.iter().find(|n| n.kind == NodeKind::WindSway)?;
        let pin = |pin: u8, unknown: f32| self.constant_input(node.id, pin).unwrap_or(unknown);
        let amount = pin(0, 0.0) * pin(1, 1.0);
        let flutter = pin(2, 0.0) * pin(3, 1.0);
        let height = pin(4, 1.0);
        (amount > 0.0 || flutter > 0.0).then_some([amount, flutter, height])
    }

    /// The first number of input `pin` of `node` where it is the same
    /// everywhere: a wire from a parameter, a constant, or a blend of such;
    /// without a wire the pin's own value. `None` where it varies.
    fn constant_input(&self, node: graph::NodeId, pin: u8) -> Option<f32> {
        use graph::{MathOp, NodeKind};
        let graph = &self.graph.graph;
        let Some(wire) = graph.wire_into(node, pin) else {
            let n = graph.node(node)?;
            return n
                .values
                .get(&pin)
                .map(|v| v[0])
                .or_else(|| n.kind.inputs().get(pin as usize).map(|p| p.default[0]));
        };
        let from = graph.node(wire.from.node)?;
        match &from.kind {
            NodeKind::Parameter(name) => match self.value(name)? {
                v @ ParamValue::Number(..) => Some(v.numbers()[0]),
                ParamValue::Texture(_) => None,
            },
            NodeKind::Constant { value, .. } => Some(value[0]),
            NodeKind::Math(MathOp::Lerp) => {
                // An end the blend does not reach may vary.
                let t = self.constant_input(from.id, 2)?;
                let a = if t < 1.0 { self.constant_input(from.id, 0)? } else { 0.0 };
                let b = if t > 0.0 { self.constant_input(from.id, 1)? } else { 0.0 };
                Some(a + (b - a) * t)
            }
            NodeKind::Math(MathOp::Multiply) => {
                Some(self.constant_input(from.id, 0)? * self.constant_input(from.id, 1)?)
            }
            _ => None,
        }
    }

    /// The texture a cut-out material's opacity is read from, and its
    /// channel (0 to 3), followed upstream from the output's opacity: for
    /// ray traced shadows, which test leaves against it. `None` when the
    /// opacity comes from no texture, or the texture is not set.
    pub fn opacity_texture(&self) -> Option<(&Texture, u32)> {
        use graph::NodeKind;
        let graph = &self.graph.graph;
        let output = graph.output_node()?;
        let wire = graph.wire_into(output.id, graph::output::OPACITY)?;
        // Straight from a texture's R, G, B or A pin: that channel; through
        // other nodes: the first texture upstream, its alpha.
        let mut channel = match (graph.node(wire.from.node).map(|n| &n.kind), wire.from.pin) {
            (Some(NodeKind::SampleTexture), 2..=5) => Some(u32::from(wire.from.pin) - 2),
            _ => None,
        };
        let mut stack = vec![wire.from.node];
        let mut seen = Vec::new();
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(node) = graph.node(id) else {
                continue;
            };
            if node.kind == NodeKind::SampleTexture {
                let param = graph.wire_into(id, 0).and_then(|w| graph.node(w.from.node)).and_then(|n| match &n.kind {
                    NodeKind::Parameter(name) => Some(name.clone()),
                    _ => None,
                });
                let index = param.and_then(|name| graph.params.iter().position(|p| p.name == name));
                if let Some(ParamValue::Texture(Some(texture))) = index.and_then(|i| self.values.get(i)) {
                    return Some((texture, channel.take().unwrap_or(3)));
                }
                continue;
            }
            channel = None;
            stack.extend(graph.wires.iter().filter(|w| w.to.node == id).map(|w| w.from.node));
        }
        None
    }

    pub fn graph(&self) -> &Rc<CompiledGraph> {
        &self.graph
    }

    /// Use another graph; parameters of the same name and kind keep their
    /// values.
    pub fn set_graph(&mut self, graph: Rc<CompiledGraph>) {
        self.programs.borrow_mut().clear();
        self.wind.set(None);
        let old = std::mem::replace(&mut self.graph, graph);
        let mut old_values: HashMap<String, ParamValue> = old
            .graph
            .params
            .iter()
            .zip(std::mem::take(&mut self.values))
            .map(|(p, v)| (p.name.clone(), v))
            .collect();
        self.values = self
            .graph
            .graph
            .params
            .iter()
            .map(|decl| match old_values.remove(&decl.name) {
                Some(v @ ParamValue::Texture(_)) if matches!(decl.ty, ParamType::Texture { .. }) => v,
                Some(v @ ParamValue::Number(..)) if !matches!(decl.ty, ParamType::Texture { .. }) => v,
                _ => ParamValue::default_for(decl),
            })
            .collect();
    }


    pub fn value(&self, name: &str) -> Option<&ParamValue> {
        self.graph.param_index(name).map(|i| &self.values[i])
    }

    /// Set a parameter's numbers (a float uses the first). False if the
    /// graph has no such number.
    pub fn set_number(&mut self, name: &str, value: [f32; 4]) -> bool {
        self.set_param(name, Param::Fixed(value[0]), value)
    }

    pub fn set_float(&mut self, name: &str, value: f32) -> bool {
        self.set_number(name, [value; 4])
    }

    /// Set a float that may follow a graphics level.
    pub fn set_param(&mut self, name: &str, param: Param<f32>, value: [f32; 4]) -> bool {
        let Some(i) = self.graph.param_index(name) else { return false };
        if matches!(self.graph.graph.params[i].ty, ParamType::Texture { .. }) {
            return false;
        }
        let mut value = value;
        value[0] = param.high();
        self.values[i] = ParamValue::Number(param, value);
        self.wind.set(None);
        true
    }

    /// The numbers a parameter has (its first resolved at High).
    pub fn number(&self, name: &str) -> Option<[f32; 4]> {
        match self.value(name)? {
            ParamValue::Number(_, v) => Some(*v),
            ParamValue::Texture(_) => None,
        }
    }

    /// Set (or clear) a texture parameter. False if there is none.
    pub fn set_texture(&mut self, name: &str, texture: Option<Texture>) -> bool {
        let Some(i) = self.graph.param_index(name) else { return false };
        if !matches!(self.graph.graph.params[i].ty, ParamType::Texture { .. }) {
            return false;
        }
        self.values[i] = ParamValue::Texture(texture);
        true
    }

    pub fn texture(&self, name: &str) -> Option<&Texture> {
        match self.value(name)? {
            ParamValue::Texture(t) => t.as_ref(),
            ParamValue::Number(..) => None,
        }
    }

    /// Whether fragments are cut out.
    pub fn is_cutout(&self) -> bool {
        matches!(self.alpha, AlphaMode::Cutout(_)) && self.graph.has_opacity()
    }

    pub fn variant(&self) -> Variant {
        Variant { cutout: self.is_cutout(), lod0: false }
    }

    /// The graph drawn: a broken graph is drawn as plain magenta Standard
    /// PBR (logged once).
    fn drawn_graph(&self) -> Rc<CompiledGraph> {
        if self.graph.is_broken() {
            warn_broken(&self.graph);
            standard_graph()
        } else {
            self.graph.clone()
        }
    }

    /// The program for `pipeline`.
    pub fn program(&self, pipeline: Pipeline) -> Rc<Shader> {
        let cutout = self.is_cutout();
        if let Some((_, _, shader)) = self.programs.borrow().iter().find(|(p, c, _)| *p == pipeline && *c == cutout) {
            return shader.clone();
        }
        let shader = self.find_program(pipeline);
        self.programs.borrow_mut().push((pipeline, cutout, shader.clone()));
        shader
    }

    fn find_program(&self, pipeline: Pipeline) -> Rc<Shader> {
        let graph = self.drawn_graph();
        let mut variant = if self.graph.is_broken() { Variant::default() } else { self.variant() };
        if pipeline == Pipeline::ImpostorBake {
            variant.lod0 = true;
        }
        programs::program(&graph, variant, pipeline)
    }

    /// Set the parameters on the bound `shader` (a program of this
    /// material's) and bind its textures. Only values that differ from
    /// what the program holds are set (most materials share most values).
    pub fn bind(&self, shader: &Shader) {
        if self.graph.is_broken() {
            bind_broken(shader);
            return;
        }
        let graph = &self.graph;
        PROGRAM_VALUES.with(|state| {
            let mut state = state.borrow_mut();
            let held = state.entry(shader.id()).or_default();
            // One slot per parameter, then the cut-off, then the levels.
            let wanted = graph.uniforms.len() + 1 + graph.levels.len();
            if held.len() != wanted {
                held.clear();
                held.resize(wanted, [f32::NAN; 4]);
                // A program's samplers stay on their units.
                for (uniform, texture) in &graph.uniforms {
                    if let Some((_, unit)) = texture {
                        shader.set_uniform_1i(uniform, *unit as i32);
                    }
                }
            }
            for (i, (uniform, texture)) in graph.uniforms.iter().enumerate() {
                match (&self.values[i], texture) {
                    (ParamValue::Texture(t), Some((flag, unit))) => {
                        let v = [t.is_some() as i32 as f32; 4];
                        if held[i] != v {
                            shader.set_uniform_1i(flag, t.is_some() as i32);
                            held[i] = v;
                        }
                        if let Some(t) = t {
                            t.bind_to(*unit);
                        }
                    }
                    (value, None) => {
                        let v = value.numbers();
                        if held[i] != v {
                            match graph.graph.params[i].ty.value_type() {
                                graph::ValueType::Float => shader.set_uniform_1f(uniform, v[0]),
                                graph::ValueType::Vec2 => shader.set_uniform_2f(uniform, v[0], v[1]),
                                graph::ValueType::Vec3 => shader.set_uniform_3f(uniform, v[0], v[1], v[2]),
                                _ => shader.set_uniform_4f(uniform, v[0], v[1], v[2], v[3]),
                            }
                            held[i] = v;
                        }
                    }
                    _ => {}
                }
            }
            let cutoff = match self.alpha {
                AlphaMode::Cutout(c) => c,
                AlphaMode::Opaque => -1.0,
            };
            let at = graph.uniforms.len();
            if held[at] != [cutoff; 4] {
                shader.set_uniform_1f("materialCutoff", cutoff);
                held[at] = [cutoff; 4];
            }
            if !graph.levels.is_empty() {
                let settings = graphics::current();
                for (k, (setting, uniform)) in graph.levels.iter().enumerate() {
                    let level = match settings.level(setting) {
                        Level::Low => 0,
                        Level::Medium => 1,
                        Level::High => 2,
                        Level::Ultra => 3,
                    };
                    if held[at + 1 + k] != [level as f32; 4] {
                        shader.set_uniform_1i(uniform, level);
                        held[at + 1 + k] = [level as f32; 4];
                    }
                }
            }
        });
    }
}

thread_local! {
    /// What each program's material uniforms hold now (see `bind`).
    static PROGRAM_VALUES: RefCell<crate::core::gfx::fx::FxHashMap<u32, Vec<[f32; 4]>>> = RefCell::new(Default::default());
    static WARNED: RefCell<std::collections::HashSet<u64>> = RefCell::new(Default::default());
}

/// Forget what was set on programs (a program was rebuilt, or uniforms
/// were set from elsewhere).
pub fn forget_bound() {
    PROGRAM_VALUES.with(|b| b.borrow_mut().clear());
}

fn warn_broken(graph: &CompiledGraph) {
    WARNED.with(|warned| {
        if warned.borrow_mut().insert(graph.key) {
            let problems: Vec<String> = graph.problems.iter().map(|p| p.to_string()).collect();
            log::warn!("material graph {}: drawn magenta: {}", graph.name, problems.join("; "));
        }
    });
}

fn bind_broken(shader: &Shader) {
    PROGRAM_VALUES.with(|b| {
        b.borrow_mut().remove(&shader.id());
    });
    let standard = standard_graph();
    for (uniform, texture) in &standard.uniforms {
        if let Some((flag, unit)) = texture {
            shader.set_uniform_1i(uniform, *unit as i32);
            shader.set_uniform_1i(flag, 0);
        }
    }
    for decl in &standard.graph.params {
        let uniform = codegen::param_uniform(&decl.name);
        match decl.ty.value_type() {
            graph::ValueType::Float => shader.set_uniform_1f(&uniform, decl.default[0]),
            graph::ValueType::Vec2 => shader.set_uniform_2f(&uniform, decl.default[0], decl.default[1]),
            graph::ValueType::Vec3 => shader.set_uniform_3f(&uniform, 0.0, 0.0, 0.0),
            _ => {}
        }
    }
    shader.set_uniform_4f(&codegen::param_uniform(standard::params::BASE_COLOR), 1.0, 0.0, 1.0, 1.0);
    shader.set_uniform_1f("materialCutoff", -1.0);
}

/// sRGB (0..1, as 8 bits) to linear: what the GPU reads from an sRGB map.
pub fn srgb8_to_linear(c: f32) -> f32 {
    let c = (c.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

thread_local! {
    /// Called when the context goes away (see `release_gl_resources`).
    static ON_RELEASE: RefCell<Vec<fn()>> = RefCell::new(Vec::new());
}

/// Run `f` too when the material resources are freed (the editor's preview
/// shapes, say).
pub fn on_release(f: fn()) {
    ON_RELEASE.with(|r| r.borrow_mut().push(f));
}

/// Free every program, loaded texture and asset material while the
/// context is still alive.
pub fn release_gl_resources() {
    programs::release();
    asset::release();
    let _ = ON_RELEASE.try_with(|r| {
        for f in r.borrow().iter() {
            f();
        }
    });
}
