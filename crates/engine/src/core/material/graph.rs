//! Material graphs: what a surface looks like (and how its vertices move),
//! put together as a graph of nodes. See docs/materials.md.
//!
//! A graph is pure data (a `*.matgraph.ron` asset): parameters a material
//! sets, nodes with stable ids, and wires from an output pin to an input
//! pin. `codegen` turns it into GLSL; the same graph gives the same code
//! every time, so programs are cached by their code.

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

/// What a material graph file's name ends with.
pub const MATERIAL_GRAPH_SUFFIX: &str = ".matgraph.ron";

/// Whether `path` names a material graph file.
pub fn is_material_graph_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().ends_with(MATERIAL_GRAPH_SUFFIX))
}

/// A node's id, stable for the life of the graph (wires refer to it).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// One pin of a node: `pin` counts the node's inputs (or outputs) from 0.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PinRef {
    pub node: NodeId,
    pub pin: u8,
}

/// A value flows from `from` (an output pin) to `to` (an input pin).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wire {
    pub from: PinRef,
    pub to: PinRef,
}

/// The type of a value on a wire or of a parameter.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Float,
    Vec2,
    Vec3,
    Vec4,
    /// A texture parameter: only `Sample Texture` and `Normal Map` take it.
    Texture,
}

impl ValueType {
    /// Components of a vector type (0 for a texture).
    pub fn width(self) -> u8 {
        match self {
            ValueType::Float => 1,
            ValueType::Vec2 => 2,
            ValueType::Vec3 => 3,
            ValueType::Vec4 => 4,
            ValueType::Texture => 0,
        }
    }

    pub fn of_width(width: u8) -> ValueType {
        match width {
            0 | 1 => ValueType::Float,
            2 => ValueType::Vec2,
            3 => ValueType::Vec3,
            _ => ValueType::Vec4,
        }
    }

    pub fn glsl(self) -> &'static str {
        match self {
            ValueType::Float => "float",
            ValueType::Vec2 => "vec2",
            ValueType::Vec3 => "vec3",
            ValueType::Vec4 => "vec4",
            ValueType::Texture => "sampler2D",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ValueType::Float => "Zahl",
            ValueType::Vec2 => "Vec2",
            ValueType::Vec3 => "Vec3",
            ValueType::Vec4 => "Vec4",
            ValueType::Texture => "Textur",
        }
    }
}

/// What a parameter holds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamType {
    Float,
    Vec2,
    Vec3,
    /// RGBA, linear.
    Color,
    /// 0 or 1 in the shader.
    Bool,
    /// A texture; `color` ones are read as sRGB.
    Texture { color: bool },
}

impl ParamType {
    /// The type its node puts out.
    pub fn value_type(self) -> ValueType {
        match self {
            ParamType::Float | ParamType::Bool => ValueType::Float,
            ParamType::Vec2 => ValueType::Vec2,
            ParamType::Vec3 => ValueType::Vec3,
            ParamType::Color => ValueType::Vec4,
            ParamType::Texture { .. } => ValueType::Texture,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ParamType::Float => "Zahl",
            ParamType::Vec2 => "Vec2",
            ParamType::Vec3 => "Vec3",
            ParamType::Color => "Farbe",
            ParamType::Bool => "Schalter",
            ParamType::Texture { color: true } => "Farbtextur",
            ParamType::Texture { color: false } => "Datentextur",
        }
    }

    pub const ALL: [ParamType; 7] = [
        ParamType::Float,
        ParamType::Vec2,
        ParamType::Vec3,
        ParamType::Color,
        ParamType::Bool,
        ParamType::Texture { color: true },
        ParamType::Texture { color: false },
    ];
}

/// A parameter of a graph: what a material sets.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ParamDecl {
    /// Letters, digits and `_`; unique in the graph.
    pub name: String,
    pub ty: ParamType,
    /// The value a material that does not set it gets (textures: none).
    #[serde(default)]
    pub default: [f32; 4],
    /// Section in the inspector; empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Shown in the inspector instead of the name ("UV Scale"); empty:
    /// made from the name (see `display_name`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl ParamDecl {
    /// What the inspector calls it.
    pub fn display(&self) -> String {
        if self.label.is_empty() { display_name(&self.name) } else { self.label.clone() }
    }
}

/// A name for people: `uv_scale` → "UV Scale", `two_sided` → "Two Sided".
pub fn display_name(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| match w.to_lowercase().as_str() {
            "uv" | "pbr" | "rgb" | "rgba" | "ao" => w.to_uppercase(),
            _ => {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a `Normal` or `Position` node takes its value from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    World,
    Object,
}

/// A maths node's operation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
    Power,
    Dot,
    Step,
    OneMinus,
    Abs,
    Saturate,
    Normalize,
    Fract,
    Sin,
    Cos,
    Sqrt,
    Lerp,
    Clamp,
    Smoothstep,
}

impl MathOp {
    pub const ALL: [MathOp; 20] = [
        MathOp::Add,
        MathOp::Subtract,
        MathOp::Multiply,
        MathOp::Divide,
        MathOp::Min,
        MathOp::Max,
        MathOp::Power,
        MathOp::Dot,
        MathOp::Step,
        MathOp::OneMinus,
        MathOp::Abs,
        MathOp::Saturate,
        MathOp::Normalize,
        MathOp::Fract,
        MathOp::Sin,
        MathOp::Cos,
        MathOp::Sqrt,
        MathOp::Lerp,
        MathOp::Clamp,
        MathOp::Smoothstep,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MathOp::Add => "Add",
            MathOp::Subtract => "Subtract",
            MathOp::Multiply => "Multiply",
            MathOp::Divide => "Divide",
            MathOp::Min => "Min",
            MathOp::Max => "Max",
            MathOp::Power => "Power",
            MathOp::Dot => "Dot",
            MathOp::Step => "Step",
            MathOp::OneMinus => "One Minus",
            MathOp::Abs => "Abs",
            MathOp::Saturate => "Saturate",
            MathOp::Normalize => "Normalize",
            MathOp::Fract => "Fract",
            MathOp::Sin => "Sin",
            MathOp::Cos => "Cos",
            MathOp::Sqrt => "Sqrt",
            MathOp::Lerp => "Lerp",
            MathOp::Clamp => "Clamp",
            MathOp::Smoothstep => "Smoothstep",
        }
    }

    /// Input pins: (label, default).
    fn inputs(self) -> &'static [InputPin] {
        const A: InputPin = InputPin::any("A", [0.0; 4]);
        const B: InputPin = InputPin::any("B", [0.0; 4]);
        const B1: InputPin = InputPin::any("B", [1.0; 4]);
        const X: InputPin = InputPin::any("X", [0.0; 4]);
        match self {
            MathOp::Add | MathOp::Subtract | MathOp::Min | MathOp::Max | MathOp::Dot => &const { [A, B] },
            MathOp::Multiply | MathOp::Divide | MathOp::Power => &const { [InputPin::any("A", [1.0; 4]), B1] },
            MathOp::Step => &const { [InputPin::any("Kante", [0.5; 4]), X] },
            MathOp::OneMinus
            | MathOp::Abs
            | MathOp::Saturate
            | MathOp::Normalize
            | MathOp::Fract
            | MathOp::Sin
            | MathOp::Cos
            | MathOp::Sqrt => &const { [X] },
            MathOp::Lerp => &const { [A, B1, InputPin::float("T", 0.5)] },
            MathOp::Clamp => &const { [X, InputPin::any("Min", [0.0; 4]), InputPin::any("Max", [1.0; 4])] },
            MathOp::Smoothstep => &const { [
                InputPin::any("Kante 0", [0.0; 4]),
                InputPin::any("Kante 1", [1.0; 4]),
                X,
            ] },
        }
    }
}

/// What a node does.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// A parameter of the graph, by name.
    Parameter(String),
    /// A fixed value; `width` 1 to 4.
    Constant { value: [f32; 4], width: u8 },
    Uv,
    Position(Space),
    Normal(Space),
    ViewDirection,
    /// Seconds, the wind's clock.
    Time,
    VertexColor,
    /// A scatter instance's colour (white elsewhere).
    InstanceTint,
    /// Metres above the model's origin (its pivot).
    HeightOverPivot,
    /// Texture, UV, fallback (when no texture is set) → RGBA, RGB, R, G, B, A.
    SampleTexture,
    /// Texture, UV, strength → the shading normal (world space).
    NormalMap,
    Math(MathOp),
    /// Vector → X, Y, Z, W.
    Split,
    /// X, Y, Z, W → Vec4, Vec3, Vec2.
    Combine,
    /// X from [in min, in max] to [out min, out max].
    Remap,
    /// Normal, power → how much the surface faces away from the eye.
    Fresnel,
    /// Position, scale → smooth value noise 0..1.
    Noise,
    /// Normal, amount → the normal bent towards the sky (grass lit like
    /// the ground).
    BendNormalUp,
    /// One of four inputs by the level of a graphics setting.
    GraphicsLevel(String),
    /// Amount, mask, flutter, flutter mask, height → a world offset: the
    /// model bends with the wind about its pivot (by the square of the
    /// height over it, in units of `height`: 1 m for grass, the tree's
    /// height for a tree), leaves flutter along their normal.
    WindSway,
    /// The material's channels.
    Output,
}

/// How a node is shown and grouped in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeGroup {
    Input,
    Texture,
    Math,
    Effect,
    Output,
}

/// An input pin: its label, the type it wants and its value when nothing
/// is wired to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputPin {
    pub label: &'static str,
    /// `None`: takes any vector width (the node's own width follows the
    /// widest input).
    pub ty: Option<ValueType>,
    pub default: [f32; 4],
    /// Without a wire: a value of the surface instead of `default` (see
    /// `DefaultInput`).
    pub implicit: Option<DefaultInput>,
}

/// What an unwired input takes when it is not a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultInput {
    Uv,
    Normal,
    WorldPosition,
}

impl InputPin {
    const fn any(label: &'static str, default: [f32; 4]) -> Self {
        InputPin { label, ty: None, default, implicit: None }
    }

    const fn float(label: &'static str, value: f32) -> Self {
        InputPin { label, ty: Some(ValueType::Float), default: [value; 4], implicit: None }
    }

    const fn typed(label: &'static str, ty: ValueType, default: [f32; 4]) -> Self {
        InputPin { label, ty: Some(ty), default, implicit: None }
    }

    const fn implicit(label: &'static str, ty: ValueType, implicit: DefaultInput) -> Self {
        InputPin { label, ty: Some(ty), default: [0.0; 4], implicit: Some(implicit) }
    }
}

/// The output node's pins, in order.
pub mod output {
    pub const BASE_COLOR: u8 = 0;
    pub const OPACITY: u8 = 1;
    pub const NORMAL: u8 = 2;
    pub const ROUGHNESS: u8 = 3;
    pub const METALLIC: u8 = 4;
    pub const OCCLUSION: u8 = 5;
    pub const EMISSIVE: u8 = 6;
    pub const WORLD_OFFSET: u8 = 7;
}

impl NodeKind {
    /// Every kind the editor offers to add, with a placeholder where a
    /// kind takes a value.
    pub fn palette() -> Vec<NodeKind> {
        let mut kinds = vec![
            NodeKind::Parameter(String::new()),
            NodeKind::Constant { value: [1.0; 4], width: 1 },
            NodeKind::Constant { value: [1.0; 4], width: 3 },
            NodeKind::Uv,
            NodeKind::Position(Space::World),
            NodeKind::Position(Space::Object),
            NodeKind::Normal(Space::World),
            NodeKind::ViewDirection,
            NodeKind::Time,
            NodeKind::VertexColor,
            NodeKind::InstanceTint,
            NodeKind::HeightOverPivot,
            NodeKind::SampleTexture,
            NodeKind::NormalMap,
        ];
        kinds.extend(MathOp::ALL.into_iter().map(NodeKind::Math));
        kinds.extend([
            NodeKind::Split,
            NodeKind::Combine,
            NodeKind::Remap,
            NodeKind::Fresnel,
            NodeKind::Noise,
            NodeKind::BendNormalUp,
            NodeKind::GraphicsLevel("vegetation".into()),
            NodeKind::WindSway,
        ]);
        kinds
    }

    pub fn label(&self) -> String {
        match self {
            NodeKind::Parameter(name) if name.is_empty() => "Parameter".into(),
            NodeKind::Parameter(name) => format!("Param: {name}"),
            NodeKind::Constant { width, .. } => match width {
                1 => "Konstante".into(),
                _ => format!("Konstante Vec{width}"),
            },
            NodeKind::Uv => "UV".into(),
            NodeKind::Position(Space::World) => "Weltposition".into(),
            NodeKind::Position(Space::Object) => "Objektposition".into(),
            NodeKind::Normal(Space::World) => "Normale".into(),
            NodeKind::Normal(Space::Object) => "Normale (Objekt)".into(),
            NodeKind::ViewDirection => "Blickrichtung".into(),
            NodeKind::Time => "Zeit".into(),
            NodeKind::VertexColor => "Vertexfarbe".into(),
            NodeKind::InstanceTint => "Instanz-Tönung".into(),
            NodeKind::HeightOverPivot => "Höhe über Pivot".into(),
            NodeKind::SampleTexture => "Textur lesen".into(),
            NodeKind::NormalMap => "Normal Map".into(),
            NodeKind::Math(op) => op.label().into(),
            NodeKind::Split => "Aufteilen".into(),
            NodeKind::Combine => "Zusammensetzen".into(),
            NodeKind::Remap => "Umrechnen".into(),
            NodeKind::Fresnel => "Fresnel".into(),
            NodeKind::Noise => "Rauschen".into(),
            NodeKind::BendNormalUp => "Normale nach oben".into(),
            NodeKind::GraphicsLevel(setting) => format!("Grafikstufe: {setting}"),
            NodeKind::WindSway => "Wind".into(),
            NodeKind::Output => "Material".into(),
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            NodeKind::Parameter(_) => "Ein Wert, den jedes Material selbst setzt (Textur, Farbe, Zahl).",
            NodeKind::Constant { .. } => "Ein fester Wert.",
            NodeKind::Uv => "Die Texturkoordinaten des Meshes.",
            NodeKind::Position(Space::World) => "Die Position in der Welt, in Metern.",
            NodeKind::Position(Space::Object) => "Die Position im Modell, vor dem Platzieren.",
            NodeKind::Normal(_) => "Die Normale der Fläche (Welt), zur Kamera gedreht.",
            NodeKind::ViewDirection => "Von der Fläche zur Kamera, Länge 1.",
            NodeKind::Time => "Sekunden, die Uhr des Winds.",
            NodeKind::VertexColor => "Die Vertexfarbe aus der Datei (weiß ohne).",
            NodeKind::InstanceTint => "Die Tönung gestreuter Instanzen (sonst weiß).",
            NodeKind::HeightOverPivot => "Meter über dem Ursprung des Modells.",
            NodeKind::SampleTexture => "Liest eine Textur; ohne gesetzte Textur der Ersatzwert.",
            NodeKind::NormalMap => "Biegt die Normale mit einer Normal Map (OpenGL-Stil).",
            NodeKind::Math(_) => "Rechnet komponentenweise; eine Zahl gilt für alle Komponenten.",
            NodeKind::Split => "Zerlegt einen Vektor in Zahlen.",
            NodeKind::Combine => "Setzt Zahlen zu einem Vektor zusammen.",
            NodeKind::Remap => "Rechnet X von einem Bereich in einen anderen um.",
            NodeKind::Fresnel => "0 frontal, 1 am Rand der Silhouette.",
            NodeKind::Noise => "Weiches Rauschen 0..1 über eine Position.",
            NodeKind::BendNormalUp => "Biegt die Normale zum Himmel (Gras wie der Boden beleuchtet).",
            NodeKind::GraphicsLevel(_) => "Einer von vier Eingängen, nach der Stufe einer Grafikeinstellung.",
            NodeKind::WindSway => {
                "Verschiebung im Wind: biegt um den Pivot (Höhe²), Blätter flattern entlang der Normale. \
                 Maske 0 hält einen Teil still."
            }
            NodeKind::Output => "Die Kanäle des Materials; World Offset bewegt die Vertices.",
        }
    }

    pub fn group(&self) -> NodeGroup {
        match self {
            NodeKind::Parameter(_)
            | NodeKind::Constant { .. }
            | NodeKind::Uv
            | NodeKind::Position(_)
            | NodeKind::Normal(_)
            | NodeKind::ViewDirection
            | NodeKind::Time
            | NodeKind::VertexColor
            | NodeKind::InstanceTint
            | NodeKind::HeightOverPivot => NodeGroup::Input,
            NodeKind::SampleTexture | NodeKind::NormalMap => NodeGroup::Texture,
            NodeKind::Math(_) | NodeKind::Split | NodeKind::Combine | NodeKind::Remap => NodeGroup::Math,
            NodeKind::Fresnel
            | NodeKind::Noise
            | NodeKind::BendNormalUp
            | NodeKind::GraphicsLevel(_)
            | NodeKind::WindSway => NodeGroup::Effect,
            NodeKind::Output => NodeGroup::Output,
        }
    }

    pub fn inputs(&self) -> &'static [InputPin] {
        use ValueType::*;
        match self {
            NodeKind::Parameter(_)
            | NodeKind::Constant { .. }
            | NodeKind::Uv
            | NodeKind::Position(_)
            | NodeKind::Normal(_)
            | NodeKind::ViewDirection
            | NodeKind::Time
            | NodeKind::VertexColor
            | NodeKind::InstanceTint
            | NodeKind::HeightOverPivot => &const { [] },
            NodeKind::SampleTexture => &const { [
                InputPin::typed("Textur", Texture, [0.0; 4]),
                InputPin::implicit("UV", Vec2, DefaultInput::Uv),
                InputPin::typed("Ersatz", Vec4, [1.0; 4]),
            ] },
            NodeKind::NormalMap => &const { [
                InputPin::typed("Textur", Texture, [0.0; 4]),
                InputPin::implicit("UV", Vec2, DefaultInput::Uv),
                InputPin::float("Stärke", 1.0),
            ] },
            NodeKind::Math(op) => op.inputs(),
            NodeKind::Split => &const { [InputPin::typed("Vektor", Vec4, [0.0; 4])] },
            NodeKind::Combine => &const { [
                InputPin::float("X", 0.0),
                InputPin::float("Y", 0.0),
                InputPin::float("Z", 0.0),
                InputPin::float("W", 1.0),
            ] },
            NodeKind::Remap => &const { [
                InputPin::any("X", [0.5; 4]),
                InputPin::any("Von min", [0.0; 4]),
                InputPin::any("Von max", [1.0; 4]),
                InputPin::any("Nach min", [0.0; 4]),
                InputPin::any("Nach max", [1.0; 4]),
            ] },
            NodeKind::Fresnel => &const { [
                InputPin::implicit("Normale", Vec3, DefaultInput::Normal),
                InputPin::float("Exponent", 5.0),
            ] },
            NodeKind::Noise => &const { [
                InputPin::implicit("Position", Vec3, DefaultInput::WorldPosition),
                InputPin::float("Maßstab", 1.0),
            ] },
            NodeKind::BendNormalUp => &const { [
                InputPin::implicit("Normale", Vec3, DefaultInput::Normal),
                InputPin::float("Stärke", 0.5),
            ] },
            NodeKind::GraphicsLevel(_) => &const { [
                InputPin::any("Niedrig", [0.0; 4]),
                InputPin::any("Mittel", [0.0; 4]),
                InputPin::any("Hoch", [1.0; 4]),
                InputPin::any("Ultra", [1.0; 4]),
            ] },
            NodeKind::WindSway => &const { [
                InputPin::float("Stärke", 1.0),
                InputPin::float("Maske", 1.0),
                InputPin::float("Flattern", 0.0),
                InputPin::float("Flattermaske", 1.0),
                InputPin::float("Höhe", 1.0),
            ] },
            NodeKind::Output => &const { [
                InputPin::typed("Grundfarbe", Vec3, [1.0; 4]),
                InputPin::float("Deckkraft", 1.0),
                InputPin::implicit("Normale", Vec3, DefaultInput::Normal),
                InputPin::float("Rauheit", 0.8),
                InputPin::float("Metallisch", 0.0),
                InputPin::float("Okklusion", 1.0),
                InputPin::typed("Leuchten", Vec3, [0.0; 4]),
                InputPin::typed("World Offset", Vec3, [0.0; 4]),
            ] },
        }
    }

    /// Output pin labels.
    pub fn outputs(&self) -> &'static [&'static str] {
        match self {
            NodeKind::Output => &const { [] },
            NodeKind::SampleTexture => &const { ["RGBA", "RGB", "R", "G", "B", "A"] },
            NodeKind::Split => &const { ["X", "Y", "Z", "W"] },
            NodeKind::Combine => &const { ["Vec4", "Vec3", "Vec2"] },
            NodeKind::Parameter(_) => &const { ["Wert"] },
            _ => &const { ["Wert"] },
        }
    }

    /// Kinds that only work per fragment (screen-space derivatives).
    pub fn fragment_only(&self) -> bool {
        matches!(self, NodeKind::NormalMap)
    }

    /// Kinds that only make sense on the vertex side.
    pub fn vertex_only(&self) -> bool {
        matches!(self, NodeKind::WindSway)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Where the node editor shows it (editor-only).
    #[serde(default)]
    pub position: [f32; 2],
    /// Values of unwired inputs that differ from the pin's default, by
    /// input pin.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<u8, [f32; 4]>,
}

impl GraphNode {
    /// The value an unwired input `pin` takes.
    pub fn input_value(&self, pin: u8) -> [f32; 4] {
        self.values
            .get(&pin)
            .copied()
            .or_else(|| self.kind.inputs().get(pin as usize).map(|p| p.default))
            .unwrap_or([0.0; 4])
    }
}

/// A material graph.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MaterialGraph {
    #[serde(default)]
    pub params: Vec<ParamDecl>,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub wires: Vec<Wire>,
}

impl MaterialGraph {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_ron(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default().struct_names(false))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        fs::write(path, self.to_ron()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A graph with just an output node.
    pub fn empty() -> Self {
        let mut graph = MaterialGraph::default();
        graph.add(NodeKind::Output);
        graph
    }

    pub fn node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn next_id(&self) -> NodeId {
        NodeId(self.nodes.iter().map(|n| n.id.0 + 1).max().unwrap_or(0))
    }

    pub fn add(&mut self, kind: NodeKind) -> NodeId {
        let id = self.next_id();
        self.nodes.push(GraphNode { id, kind, position: [0.0; 2], values: BTreeMap::new() });
        id
    }

    pub fn add_at(&mut self, kind: NodeKind, position: [f32; 2]) -> NodeId {
        let id = self.add(kind);
        if let Some(node) = self.node_mut(id) {
            node.position = position;
        }
        id
    }

    /// Wire `from`'s output to `to`'s input, replacing what was wired
    /// there (an input takes one wire).
    pub fn connect(&mut self, from: NodeId, from_pin: u8, to: NodeId, to_pin: u8) {
        self.wires.retain(|w| !(w.to.node == to && w.to.pin == to_pin));
        self.wires.push(Wire {
            from: PinRef { node: from, pin: from_pin },
            to: PinRef { node: to, pin: to_pin },
        });
    }

    pub fn set_value(&mut self, node: NodeId, pin: u8, value: [f32; 4]) {
        if let Some(node) = self.node_mut(node) {
            node.values.insert(pin, value);
        }
    }

    pub fn remove(&mut self, id: NodeId) {
        self.nodes.retain(|n| n.id != id);
        self.wires.retain(|w| w.from.node != id && w.to.node != id);
    }

    /// The wire into `node`'s input `pin`, if any.
    pub fn wire_into(&self, node: NodeId, pin: u8) -> Option<&Wire> {
        self.wires.iter().find(|w| w.to.node == node && w.to.pin == pin)
    }

    pub fn output_node(&self) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Output)
    }

    pub fn param(&self, name: &str) -> Option<&ParamDecl> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Graphics settings the graph's level nodes read.
    pub fn settings_used(&self) -> Vec<String> {
        let mut settings: Vec<String> = self
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::GraphicsLevel(setting) => Some(setting.clone()),
                _ => None,
            })
            .collect();
        settings.sort();
        settings.dedup();
        settings
    }

    /// What is wrong with the graph, if anything; the code generator
    /// refuses a graph with problems.
    pub fn problems(&self) -> Vec<GraphProblem> {
        super::codegen::check(self)
    }
}

/// Something wrong with a graph, tied to the node it is on where there is
/// one.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphProblem {
    pub node: Option<NodeId>,
    pub message: String,
}

impl std::fmt::Display for GraphProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.node {
            Some(node) => write!(f, "node {}: {}", node.0, self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Whether `name` is usable as a parameter name (it becomes part of a
/// GLSL identifier).
pub fn valid_param_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}
