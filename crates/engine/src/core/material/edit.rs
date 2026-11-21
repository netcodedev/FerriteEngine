//! Editing materials and material graphs as lists of named properties and
//! small operations, shared by the editor's panels and its MCP tools.
//!
//! Values are `PropertyValue`s: numbers as floats, switches as bools,
//! vectors as text (`"1 0.5 0"`), colours as sRGB hex (`"#ff8800"`,
//! `"#ff880080"` with alpha), textures as `auto`, `none` or a texture
//! asset's name, a float that follows a graphics level as
//! `"level vegetation 0 0.1 0.2 0.3"`.

use std::borrow::Cow;

use serde_json::{json, Value};

use crate::core::{
    assets::{self, AssetId, AssetKind},
    entity::component::{Property, PropertyValue},
    graphics::{Param, PerLevel},
};

use super::{
    asset::{GraphRef, MaterialFile, ParamSetting, TextureSetting},
    graph::*,
    standard, AlphaMode,
};

fn prop(name: impl Into<Cow<'static, str>>, value: PropertyValue) -> Property {
    Property {
        name: name.into(),
        value,
        read_only: false,
        range: None,
        options: None,
        action: false,
    }
}

fn with_range(mut p: Property, min: f32, max: f32) -> Property {
    p.range = Some((min, max));
    p
}

fn with_options(mut p: Property, options: Vec<String>) -> Property {
    p.options = Some(options);
    p
}

fn num(v: f32) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() || s == "-" { "0".into() } else { s }
}

/// `width` numbers as text.
pub fn vector_text(v: [f32; 4], width: usize) -> String {
    v[..width.clamp(1, 4)].iter().map(|x| num(*x)).collect::<Vec<_>>().join(" ")
}

/// Numbers separated by spaces or commas; missing ones from `fallback`.
pub fn parse_vector(text: &str, fallback: [f32; 4]) -> Result<[f32; 4], String> {
    let mut out = fallback;
    let parts: Vec<&str> = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() || parts.len() > 4 {
        return Err(format!("\"{text}\": 1 to 4 numbers expected"));
    }
    for (i, part) in parts.iter().enumerate() {
        out[i] = part.parse().map_err(|_| format!("\"{part}\" is not a number"))?;
    }
    if parts.len() == 1 {
        out = [out[0]; 4];
    }
    Ok(out)
}

fn linear_to_srgb8(c: f32) -> u8 {
    (crate::core::model::model_materials::linear_to_srgb(c) * 255.0).round() as u8
}

/// A linear colour as sRGB hex (alpha only when not 1).
pub fn color_text(c: [f32; 4]) -> String {
    let [r, g, b] = [c[0], c[1], c[2]].map(linear_to_srgb8);
    let a = (c[3].clamp(0.0, 1.0) * 255.0).round() as u8;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// sRGB hex (`#rgb`, `#rrggbb`, `#rrggbbaa`) or linear numbers.
pub fn parse_color(text: &str, fallback: [f32; 4]) -> Result<[f32; 4], String> {
    let t = text.trim();
    let Some(hex) = t.strip_prefix('#') else {
        let mut v = parse_vector(t, fallback)?;
        if t.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).count() < 4 {
            v[3] = 1.0;
        }
        return Ok(v);
    };
    let hex: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_string() };
    if hex.len() != 6 && hex.len() != 8 {
        return Err(format!("\"{text}\": #rrggbb or #rrggbbaa expected"));
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| format!("\"{text}\" is not hex"));
    let srgb = |b: u8| super::srgb8_to_linear(b as f32 / 255.0);
    let a = if hex.len() == 8 { byte(6)? as f32 / 255.0 } else { 1.0 };
    Ok([srgb(byte(0)?), srgb(byte(2)?), srgb(byte(4)?), a])
}

fn level_text(setting: &str, v: &PerLevel<f32>) -> String {
    format!("level {setting} {} {} {} {}", num(v.low), num(v.medium), num(v.high), num(v.ultra))
}

fn parse_level(text: &str) -> Option<Result<Param<f32>, String>> {
    let rest = text.trim().strip_prefix("level")?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 5 {
        return Some(Err("level <setting> <low> <medium> <high> <ultra> expected".into()));
    }
    let n = |s: &str| s.parse::<f32>().map_err(|_| format!("\"{s}\" is not a number"));
    Some((|| {
        Ok(Param::ByLevel {
            setting: parts[0].to_string(),
            values: PerLevel { low: n(parts[1])?, medium: n(parts[2])?, high: n(parts[3])?, ultra: n(parts[4])? },
        })
    })())
}

fn texture_text(setting: TextureSetting) -> String {
    match setting {
        TextureSetting::Auto => "auto".into(),
        TextureSetting::None => "none".into(),
        TextureSetting::Asset(id) => assets::database()
            .get(id)
            .map(|e| e.meta.name.clone())
            .unwrap_or_else(|| format!("missing {id}")),
    }
}

/// A texture by `auto`, `none`, a texture asset's name or its id.
pub fn parse_texture(text: &str) -> Result<TextureSetting, String> {
    match text.trim() {
        "" | "auto" | "automatic" => return Ok(TextureSetting::Auto),
        "none" => return Ok(TextureSetting::None),
        _ => {}
    }
    let database = assets::database();
    let wanted = text.trim();
    let found = database
        .iter()
        .filter(|e| matches!(e.meta.kind, AssetKind::Texture(_)))
        .find(|e| e.meta.name == wanted || e.meta.id.to_string() == wanted)
        .map(|e| TextureSetting::Asset(e.meta.id));
    found.ok_or_else(|| format!("no texture asset \"{wanted}\""))
}

/// The graphs a material can use, by the name `graph` shows: the
/// builtins, then the graph assets.
pub fn graph_choices() -> Vec<(String, GraphRef)> {
    let mut out = vec![(standard::STANDARD_PBR.to_string(), GraphRef::default())];
    let database = assets::database();
    let mut graphs: Vec<(String, GraphRef)> = database
        .iter()
        .filter(|e| matches!(e.meta.kind, AssetKind::MaterialGraph(_)))
        .map(|e| (e.meta.name.clone(), GraphRef::Asset(e.meta.id)))
        .collect();
    graphs.sort_by_key(|(n, _)| n.to_lowercase());
    out.extend(graphs);
    out
}

fn graph_name(graph: &GraphRef) -> String {
    graph_choices()
        .into_iter()
        .find(|(_, g)| g == graph)
        .map(|(n, _)| n)
        .unwrap_or_else(|| match graph {
            GraphRef::Builtin(name) => name.clone(),
            GraphRef::Asset(id) => format!("missing {id}"),
        })
}

/// A material's properties, each with the section it belongs in.
pub fn material_properties(file: &MaterialFile, graph: &MaterialGraph) -> Vec<(String, Property)> {
    let mut out = Vec::new();
    let general = "Material".to_string();
    out.push((
        general.clone(),
        with_options(
            prop("graph", PropertyValue::Text(graph_name(&file.graph))),
            graph_choices().into_iter().map(|(n, _)| n).collect(),
        ),
    ));
    let (alpha, cutoff) = match file.alpha {
        AlphaMode::Opaque => ("opaque", 0.5),
        AlphaMode::Cutout(c) => ("cutout", c),
    };
    out.push((general.clone(), with_options(prop("alpha", PropertyValue::Text(alpha.into())), vec!["opaque".into(), "cutout".into()])));
    out.push((general.clone(), with_range(prop("cutoff", PropertyValue::Float(cutoff)), 0.0, 1.0)));
    out.push((general.clone(), prop("two_sided", PropertyValue::Bool(file.two_sided))));
    out.push((general.clone(), prop("foliage", PropertyValue::Bool(file.foliage))));
    out.push((general, prop("shadows", PropertyValue::Bool(file.shadows))));
    for decl in &graph.params {
        let section = if decl.group.is_empty() { "Parameter".to_string() } else { decl.group.clone() };
        let setting = file.params.get(&decl.name);
        let value = match (decl.ty, setting) {
            (ParamType::Texture { .. }, Some(ParamSetting::Texture(t))) => PropertyValue::Text(texture_text(*t)),
            (ParamType::Texture { .. }, _) => PropertyValue::Text("auto".into()),
            (_, Some(ParamSetting::Float(Param::ByLevel { setting, values }))) => PropertyValue::Text(level_text(setting, values)),
            (ParamType::Float, Some(ParamSetting::Float(p))) => PropertyValue::Float(p.high()),
            (ParamType::Float, Some(ParamSetting::Vector(v))) => PropertyValue::Float(v[0]),
            (ParamType::Float, _) => PropertyValue::Float(decl.default[0]),
            (ParamType::Bool, Some(ParamSetting::Vector(v))) => PropertyValue::Bool(v[0] > 0.5),
            (ParamType::Bool, Some(ParamSetting::Float(p))) => PropertyValue::Bool(p.high() > 0.5),
            (ParamType::Bool, _) => PropertyValue::Bool(decl.default[0] > 0.5),
            (ty, setting) => {
                let v = match setting {
                    Some(ParamSetting::Vector(v)) => *v,
                    Some(ParamSetting::Float(p)) => [p.high(); 4],
                    _ => decl.default,
                };
                PropertyValue::Text(match ty {
                    ParamType::Color => color_text(v),
                    ParamType::Vec2 => vector_text(v, 2),
                    _ => vector_text(v, 3),
                })
            }
        };
        out.push((section, prop(decl.name.clone(), value)));
    }
    out
}

/// Set one of `material_properties` on `file` (whose graph is `graph`).
pub fn set_material_property(file: &mut MaterialFile, graph: &MaterialGraph, name: &str, value: &PropertyValue) -> Result<(), String> {
    let text = || -> Result<String, String> { match value {
        PropertyValue::Text(t) => Ok(t.clone()),
        PropertyValue::Float(f) => Ok(num(*f)),
        PropertyValue::Int(i) => Ok(i.to_string()),
        PropertyValue::Bool(b) => Ok(if *b { "1".into() } else { "0".into() }),
    } };
    let float = || -> Result<f32, String> { match value {
        PropertyValue::Float(f) => Ok(*f),
        PropertyValue::Int(i) => Ok(*i as f32),
        PropertyValue::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        PropertyValue::Text(t) => t.trim().parse::<f32>().map_err(|_| format!("\"{t}\" is not a number")),
    } };
    let boolean = || -> Result<bool, String> { match value {
        PropertyValue::Bool(b) => Ok(*b),
        PropertyValue::Float(f) => Ok(*f > 0.5),
        PropertyValue::Int(i) => Ok(*i != 0),
        PropertyValue::Text(t) => match t.trim() {
            "true" | "yes" | "1" | "on" => Ok(true),
            "false" | "no" | "0" | "off" => Ok(false),
            other => Err(format!("\"{other}\" is not a switch")),
        },
    } };
    match name {
        "graph" => {
            let wanted = text()?;
            let (_, graph) = graph_choices()
                .into_iter()
                .find(|(n, _)| *n == wanted)
                .ok_or_else(|| format!("no material graph \"{wanted}\""))?;
            file.graph = graph;
        }
        "alpha" => {
            let cutoff = match file.alpha {
                AlphaMode::Cutout(c) => c,
                AlphaMode::Opaque => 0.5,
            };
            file.alpha = match text()?.as_str() {
                "opaque" => AlphaMode::Opaque,
                "cutout" => AlphaMode::Cutout(cutoff),
                other => return Err(format!("alpha: \"{other}\" (opaque or cutout)")),
            };
        }
        "cutoff" => {
            let c = float()?.clamp(0.0, 1.0);
            if let AlphaMode::Cutout(_) = file.alpha {
                file.alpha = AlphaMode::Cutout(c);
            }
        }
        "two_sided" => file.two_sided = boolean()?,
        "foliage" => file.foliage = boolean()?,
        "shadows" => file.shadows = boolean()?,
        _ => {
            let decl = graph.param(name).ok_or_else(|| format!("the graph has no parameter \"{name}\""))?;
            let setting = match decl.ty {
                ParamType::Texture { .. } => ParamSetting::Texture(parse_texture(&text()?)?),
                ParamType::Float => match parse_level(&text()?) {
                    Some(level) => ParamSetting::Float(level?),
                    None => ParamSetting::Float(Param::Fixed(float()?)),
                },
                ParamType::Bool => ParamSetting::Vector([if boolean()? { 1.0 } else { 0.0 }; 4]),
                ParamType::Color => ParamSetting::Vector(parse_color(&text()?, decl.default)?),
                ParamType::Vec2 | ParamType::Vec3 => ParamSetting::Vector(parse_vector(&text()?, decl.default)?),
            };
            file.params.insert(name.to_string(), setting);
        }
    }
    Ok(())
}

/// What the inspector calls a property of `material_properties`,
/// `node_properties` or a graph parameter's row.
pub fn property_label(name: &str, graph: &MaterialGraph) -> String {
    match name {
        "graph" => return "Graph".into(),
        "alpha" => return "Alpha Mode".into(),
        "cutoff" => return "Alpha Cutoff".into(),
        "two_sided" => return "Two-Sided".into(),
        "foliage" => return "Foliage (No Collision)".into(),
        "shadows" => return "Casts Shadows".into(),
        "parameter" => return "Parameter".into(),
        "width" => return "Components".into(),
        "value" => return "Value".into(),
        "space" => return "Space".into(),
        "operation" => return "Operation".into(),
        "setting" => return "Graphics Setting".into(),
        _ => {}
    }
    if let Some(pin) = name.strip_prefix("in:") {
        return format!("{pin} (unwired)");
    }
    if let Some(rest) = name.strip_prefix("param:") {
        let field = rest.rsplit_once(':').map_or(rest, |(_, f)| f);
        return display_name(field);
    }
    graph.param(name).map(|p| p.display()).unwrap_or_else(|| display_name(name))
}

/// Labels of the parameter types, as the editor and MCP name them.
pub fn param_type_name(ty: ParamType) -> &'static str {
    match ty {
        ParamType::Float => "float",
        ParamType::Vec2 => "vec2",
        ParamType::Vec3 => "vec3",
        ParamType::Color => "color",
        ParamType::Bool => "bool",
        ParamType::Texture { color: true } => "color_texture",
        ParamType::Texture { color: false } => "data_texture",
    }
}

pub fn parse_param_type(name: &str) -> Result<ParamType, String> {
    ParamType::ALL
        .into_iter()
        .find(|t| param_type_name(*t) == name)
        .ok_or_else(|| format!("unknown parameter type \"{name}\" (float, vec2, vec3, color, bool, color_texture, data_texture)"))
}

fn space_name(space: Space) -> &'static str {
    match space {
        Space::World => "world",
        Space::Object => "object",
    }
}

/// The settings of a node: what its kind holds, and the values of its
/// unwired inputs (`in:<label>`).
pub fn node_properties(graph: &MaterialGraph, id: NodeId) -> Vec<Property> {
    let Some(node) = graph.node(id) else { return Vec::new() };
    let mut out = Vec::new();
    match &node.kind {
        NodeKind::Parameter(name) => out.push(with_options(
            prop("parameter", PropertyValue::Text(name.clone())),
            graph.params.iter().map(|p| p.name.clone()).collect(),
        )),
        NodeKind::Constant { value, width } => {
            out.push(with_range(prop("width", PropertyValue::Int(*width as i64)), 1.0, 4.0));
            out.push(prop("value", PropertyValue::Text(vector_text(*value, *width as usize))));
        }
        NodeKind::Position(space) | NodeKind::Normal(space) => out.push(with_options(
            prop("space", PropertyValue::Text(space_name(*space).into())),
            vec!["world".into(), "object".into()],
        )),
        NodeKind::Math(op) => out.push(with_options(
            prop("operation", PropertyValue::Text(op.label().into())),
            MathOp::ALL.iter().map(|o| o.label().to_string()).collect(),
        )),
        NodeKind::GraphicsLevel(setting) => out.push(prop("setting", PropertyValue::Text(setting.clone()))),
        _ => {}
    }
    for (i, pin) in node.kind.inputs().iter().enumerate() {
        if pin.implicit.is_some() || pin.ty == Some(ValueType::Texture) || graph.wire_into(id, i as u8).is_some() {
            continue;
        }
        let v = node.input_value(i as u8);
        let name = format!("in:{}", pin.label);
        out.push(match pin.ty {
            Some(ValueType::Float) => prop(name, PropertyValue::Float(v[0])),
            Some(ty) => prop(name, PropertyValue::Text(vector_text(v, ty.width() as usize))),
            None => prop(name, PropertyValue::Text(vector_text(v, if v.iter().all(|x| *x == v[0]) { 1 } else { 4 }))),
        });
    }
    out
}

/// Set one of `node_properties`.
pub fn set_node_property(graph: &mut MaterialGraph, id: NodeId, name: &str, value: &PropertyValue) -> Result<(), String> {
    let text = match value {
        PropertyValue::Text(t) => t.clone(),
        PropertyValue::Float(f) => num(*f),
        PropertyValue::Int(i) => i.to_string(),
        PropertyValue::Bool(b) => (*b as i32).to_string(),
    };
    let param_names: Vec<String> = graph.params.iter().map(|p| p.name.clone()).collect();
    let node = graph.node_mut(id).ok_or_else(|| format!("no node {}", id.0))?;
    if let Some(label) = name.strip_prefix("in:") {
        let pin = node
            .kind
            .inputs()
            .iter()
            .position(|p| p.label == label)
            .ok_or_else(|| format!("the node has no input \"{label}\""))?;
        let v = parse_vector(&text, node.input_value(pin as u8))?;
        node.values.insert(pin as u8, v);
        return Ok(());
    }
    match (&mut node.kind, name) {
        (NodeKind::Parameter(p), "parameter") => {
            if !param_names.contains(&text) {
                return Err(format!("the graph has no parameter \"{text}\""));
            }
            *p = text;
        }
        (NodeKind::Constant { width, .. }, "width") => {
            *width = text.trim().parse::<f32>().map_err(|_| "a width of 1 to 4".to_string())?.round().clamp(1.0, 4.0) as u8;
        }
        (NodeKind::Constant { value, .. }, "value") => *value = parse_vector(&text, *value)?,
        (NodeKind::Position(space) | NodeKind::Normal(space), "space") => {
            *space = match text.as_str() {
                "world" => Space::World,
                "object" => Space::Object,
                other => return Err(format!("space \"{other}\" (world or object)")),
            }
        }
        (NodeKind::Math(op), "operation") => {
            *op = MathOp::ALL
                .into_iter()
                .find(|o| o.label().eq_ignore_ascii_case(&text))
                .ok_or_else(|| format!("unknown operation \"{text}\""))?;
        }
        (NodeKind::GraphicsLevel(setting), "setting") => *setting = text,
        _ => return Err(format!("the node has no setting \"{name}\"")),
    }
    Ok(())
}

/// A node kind by the name `NodeKind::label` gives it (case does not
/// matter); `Param: <name>` names a parameter node.
pub fn kind_by_name(graph: &MaterialGraph, name: &str) -> Result<NodeKind, String> {
    let lower = name.trim().to_lowercase();
    if let Some(param) = lower.strip_prefix("param:").or_else(|| lower.strip_prefix("parameter:")) {
        let param = param.trim();
        let found = graph
            .params
            .iter()
            .find(|p| p.name.to_lowercase() == param)
            .ok_or_else(|| format!("the graph has no parameter \"{param}\""))?;
        return Ok(NodeKind::Parameter(found.name.clone()));
    }
    NodeKind::palette()
        .into_iter()
        .chain([NodeKind::Normal(Space::Object)])
        .find(|k| k.label().to_lowercase() == lower)
        .ok_or_else(|| format!("unknown node kind \"{name}\""))
}

/// The graph as JSON for tools: parameters, nodes (with their pins and
/// settings) and wires, and its problems.
pub fn graph_json(graph: &MaterialGraph) -> Value {
    let props_json = |props: Vec<Property>| -> Value {
        Value::Object(
            props
                .into_iter()
                .map(|p| {
                    let v = match p.value {
                        PropertyValue::Float(f) => json!(f),
                        PropertyValue::Int(i) => json!(i),
                        PropertyValue::Bool(b) => json!(b),
                        PropertyValue::Text(t) => json!(t),
                    };
                    (p.name.into_owned(), v)
                })
                .collect(),
        )
    };
    json!({
        "params": graph.params.iter().map(|p| json!({
            "name": p.name,
            "type": param_type_name(p.ty),
            "default": match p.ty {
                ParamType::Color => json!(color_text(p.default)),
                ParamType::Texture { .. } => Value::Null,
                ParamType::Float | ParamType::Bool => json!(p.default[0]),
                ParamType::Vec2 => json!(vector_text(p.default, 2)),
                ParamType::Vec3 => json!(vector_text(p.default, 3)),
            },
            "group": p.group,
            "label": p.display(),
        })).collect::<Vec<_>>(),
        "nodes": graph.nodes.iter().map(|n| json!({
            "id": n.id.0,
            "kind": n.kind.label(),
            "position": n.position,
            "inputs": n.kind.inputs().iter().map(|p| p.label).collect::<Vec<_>>(),
            "outputs": n.kind.outputs(),
            "settings": props_json(node_properties(graph, n.id)),
        })).collect::<Vec<_>>(),
        "wires": graph.wires.iter().map(|w| json!({
            "from": w.from.node.0, "from_pin": w.from.pin, "to": w.to.node.0, "to_pin": w.to.pin,
        })).collect::<Vec<_>>(),
        "problems": graph.problems().iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        "kinds": NodeKind::palette().iter().map(|k| k.label()).collect::<Vec<_>>(),
    })
}

fn pin_index(labels: &[&str], value: Option<&Value>, what: &str) -> Result<u8, String> {
    match value {
        None => Ok(0),
        Some(Value::Number(n)) => n.as_u64().map(|n| n as u8).ok_or_else(|| format!("{what}: a pin number")),
        Some(Value::String(s)) => labels
            .iter()
            .position(|l| l.eq_ignore_ascii_case(s))
            .map(|i| i as u8)
            .ok_or_else(|| format!("{what}: no pin \"{s}\" (pins: {})", labels.join(", "))),
        Some(_) => Err(format!("{what}: a pin number or name")),
    }
}

fn node_arg(op: &Value, key: &str) -> Result<NodeId, String> {
    op.get(key)
        .and_then(Value::as_u64)
        .map(|n| NodeId(n as u32))
        .ok_or_else(|| format!("\"{key}\": a node id"))
}

fn property_value(v: &Value) -> PropertyValue {
    match v {
        Value::Bool(b) => PropertyValue::Bool(*b),
        Value::Number(n) => PropertyValue::Float(n.as_f64().unwrap_or(0.0) as f32),
        Value::String(s) => PropertyValue::Text(s.clone()),
        other => PropertyValue::Text(other.to_string()),
    }
}

/// Apply one tool operation to a graph. Returns the id of a node it added.
///
/// Ops: `{"op":"add","kind":..,"position":[x,y]?}`, `remove {node}`,
/// `move {node, position}`, `connect {from, from_pin?, to, to_pin?}` (pins
/// by number or label), `disconnect {to, to_pin?}`, `set {node, name,
/// value}` (see `node_properties`), `add_param {name, type, default?,
/// group?}`, `remove_param {name}`, `set_param {name, default?, group?,
/// type?, new_name?}`.
pub fn apply_op(graph: &mut MaterialGraph, op: &Value) -> Result<Option<NodeId>, String> {
    let name = op.get("op").and_then(Value::as_str).ok_or("each op needs \"op\"")?;
    match name {
        "add" => {
            let kind = kind_by_name(graph, op.get("kind").and_then(Value::as_str).ok_or("add: \"kind\"")?)?;
            let position = op
                .get("position")
                .and_then(|p| serde_json::from_value::<[f32; 2]>(p.clone()).ok())
                .unwrap_or([0.0, 0.0]);
            Ok(Some(graph.add_at(kind, position)))
        }
        "remove" => {
            let id = node_arg(op, "node")?;
            if graph.node(id).is_some_and(|n| n.kind == NodeKind::Output) {
                return Err("the material output cannot be removed".into());
            }
            graph.remove(id);
            Ok(None)
        }
        "move" => {
            let id = node_arg(op, "node")?;
            let position: [f32; 2] = op
                .get("position")
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .ok_or("move: \"position\": [x, y]")?;
            graph.node_mut(id).ok_or_else(|| format!("no node {}", id.0))?.position = position;
            Ok(None)
        }
        "connect" => {
            let from = node_arg(op, "from")?;
            let to = node_arg(op, "to")?;
            let outputs = graph.node(from).ok_or_else(|| format!("no node {}", from.0))?.kind.outputs();
            let inputs: Vec<&str> = graph.node(to).ok_or_else(|| format!("no node {}", to.0))?.kind.inputs().iter().map(|p| p.label).collect();
            let from_pin = pin_index(outputs, op.get("from_pin"), "from_pin")?;
            let to_pin = pin_index(&inputs, op.get("to_pin"), "to_pin")?;
            if from_pin as usize >= outputs.len() || to_pin as usize >= inputs.len() {
                return Err("no such pin".into());
            }
            let before = graph.clone();
            graph.connect(from, from_pin, to, to_pin);
            let problems = graph.problems();
            if let Some(problem) = problems.iter().find(|p| !before.problems().contains(p)) {
                *graph = before;
                return Err(format!("cannot wire: {}", problem.message));
            }
            Ok(None)
        }
        "disconnect" => {
            let to = node_arg(op, "to")?;
            let inputs: Vec<&str> = graph.node(to).ok_or_else(|| format!("no node {}", to.0))?.kind.inputs().iter().map(|p| p.label).collect();
            let to_pin = pin_index(&inputs, op.get("to_pin"), "to_pin")?;
            graph.wires.retain(|w| !(w.to.node == to && w.to.pin == to_pin));
            Ok(None)
        }
        "set" => {
            let id = node_arg(op, "node")?;
            let setting = op.get("name").and_then(Value::as_str).ok_or("set: \"name\"")?;
            let value = property_value(op.get("value").ok_or("set: \"value\"")?);
            set_node_property(graph, id, setting, &value)?;
            Ok(None)
        }
        "add_param" => {
            let name = op.get("name").and_then(Value::as_str).ok_or("add_param: \"name\"")?.to_string();
            if !valid_param_name(&name) || graph.param(&name).is_some() {
                return Err(format!("\"{name}\": a new name of letters, digits and _"));
            }
            let ty = parse_param_type(op.get("type").and_then(Value::as_str).unwrap_or("float"))?;
            let mut decl = ParamDecl { name, ty, default: [0.0; 4], group: String::new(), label: String::new() };
            set_param_fields(&mut decl, op)?;
            graph.params.push(decl);
            Ok(None)
        }
        "remove_param" => {
            let name = op.get("name").and_then(Value::as_str).ok_or("remove_param: \"name\"")?;
            graph.params.retain(|p| p.name != name);
            Ok(None)
        }
        "set_param" => {
            let name = op.get("name").and_then(Value::as_str).ok_or("set_param: \"name\"")?.to_string();
            let index = graph.params.iter().position(|p| p.name == name).ok_or_else(|| format!("no parameter \"{name}\""))?;
            let mut decl = graph.params[index].clone();
            set_param_fields(&mut decl, op)?;
            if let Some(new_name) = op.get("new_name").and_then(Value::as_str) {
                if !valid_param_name(new_name) || (new_name != name && graph.param(new_name).is_some()) {
                    return Err(format!("\"{new_name}\": a new name of letters, digits and _"));
                }
                decl.name = new_name.to_string();
                for node in &mut graph.nodes {
                    if node.kind == NodeKind::Parameter(name.clone()) {
                        node.kind = NodeKind::Parameter(new_name.to_string());
                    }
                }
            }
            graph.params[index] = decl;
            Ok(None)
        }
        other => Err(format!("unknown op \"{other}\"")),
    }
}

fn set_param_fields(decl: &mut ParamDecl, op: &Value) -> Result<(), String> {
    if let Some(ty) = op.get("type").and_then(Value::as_str) {
        decl.ty = parse_param_type(ty)?;
    }
    if let Some(group) = op.get("group").and_then(Value::as_str) {
        decl.group = group.to_string();
    }
    if let Some(label) = op.get("label").and_then(Value::as_str) {
        decl.label = label.to_string();
    }
    if let Some(default) = op.get("default") {
        let text = match default {
            Value::String(s) => s.clone(),
            Value::Bool(b) => (*b as i32).to_string(),
            other => other.to_string(),
        };
        decl.default = match decl.ty {
            ParamType::Color => parse_color(&text, decl.default)?,
            ParamType::Texture { .. } => [0.0; 4],
            _ => parse_vector(&text, decl.default)?,
        };
    }
    Ok(())
}

/// A material file as JSON for tools: its graph, settings and parameters.
pub fn material_json(file: &MaterialFile, graph: &MaterialGraph) -> Value {
    let props: Vec<Value> = material_properties(file, graph)
        .into_iter()
        .map(|(section, p)| {
            let v = match p.value {
                PropertyValue::Float(f) => json!(f),
                PropertyValue::Int(i) => json!(i),
                PropertyValue::Bool(b) => json!(b),
                PropertyValue::Text(t) => json!(t),
            };
            let ty = graph.param(&p.name).map(|d| param_type_name(d.ty));
            let label = property_label(&p.name, graph);
            json!({"section": section, "name": p.name, "label": label, "value": v, "type": ty, "options": p.options})
        })
        .collect();
    json!({ "properties": props })
}

/// Material assets by name (case does not matter) or id.
pub fn find_asset(name: &str, kind: fn(&AssetKind) -> bool) -> Option<AssetId> {
    let database = assets::database();
    let wanted = name.trim();
    let found = database
        .iter()
        .filter(|e| kind(&e.meta.kind))
        .find(|e| e.meta.name.eq_ignore_ascii_case(wanted) || e.meta.id.to_string() == wanted)
        .map(|e| e.meta.id);
    found
}
