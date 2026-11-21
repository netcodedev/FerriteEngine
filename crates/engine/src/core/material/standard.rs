//! The builtin graph "Standard PBR": today's model shading as a graph.
//! Every imported material starts from it. Drawn with the maps a model
//! brings along, it looks as models looked before materials (rule R1 in
//! docs/materials.md); its wind is off until a material turns it on.

use super::graph::*;

/// Parameter names of Standard PBR.
pub mod params {
    pub const BASE_COLOR_MAP: &str = "base_color_map";
    pub const BASE_COLOR: &str = "base_color";
    pub const NORMAL_MAP: &str = "normal_map";
    pub const NORMAL_STRENGTH: &str = "normal_strength";
    pub const ROUGHNESS_MAP: &str = "roughness_map";
    pub const ROUGHNESS: &str = "roughness";
    pub const METALLIC_MAP: &str = "metallic_map";
    pub const METALLIC: &str = "metallic";
    pub const OCCLUSION_MAP: &str = "occlusion_map";
    pub const EMISSIVE: &str = "emissive";
    pub const EMISSIVE_STRENGTH: &str = "emissive_strength";
    pub const UV_SCALE: &str = "uv_scale";
    pub const UV_OFFSET: &str = "uv_offset";
    pub const WIND: &str = "wind";
    pub const WIND_FLUTTER: &str = "wind_flutter";
    pub const WIND_MASK_VERTEX_COLOR: &str = "wind_mask_vertex_color";
    pub const WIND_HEIGHT: &str = "wind_height";
}

/// What the inspector calls Standard PBR's parameters.
fn label_of(name: &str) -> &'static str {
    use params::*;
    match name {
        BASE_COLOR_MAP => "Color Map",
        BASE_COLOR => "Color",
        NORMAL_MAP => "Normal Map",
        NORMAL_STRENGTH => "Normal Strength",
        ROUGHNESS_MAP => "Roughness Map",
        ROUGHNESS => "Roughness",
        METALLIC_MAP => "Metallic Map",
        METALLIC => "Metallic",
        OCCLUSION_MAP => "Occlusion Map",
        EMISSIVE => "Glow Color",
        EMISSIVE_STRENGTH => "Glow Strength",
        UV_SCALE => "UV Scale",
        UV_OFFSET => "UV Offset",
        WIND => "Wind Strength",
        WIND_FLUTTER => "Leaf Flutter",
        WIND_MASK_VERTEX_COLOR => "Mask from Vertex Color (R)",
        WIND_HEIGHT => "Bend Height (m)",
        _ => "",
    }
}

/// The name the builtin graph is shown and referred to by.
pub const STANDARD_PBR: &str = "Standard PBR";

pub fn standard_pbr() -> MaterialGraph {
    use params::*;
    let decl = |name: &str, ty: ParamType, default: [f32; 4], group: &str| ParamDecl {
        name: name.into(),
        ty,
        default,
        group: group.into(),
        label: label_of(name).into(),
    };
    let color = ParamType::Texture { color: true };
    let data = ParamType::Texture { color: false };
    let mut g = MaterialGraph {
        params: vec![
            decl(BASE_COLOR_MAP, color, [0.0; 4], "Color"),
            decl(BASE_COLOR, ParamType::Color, [1.0; 4], "Color"),
            decl(NORMAL_MAP, data, [0.0; 4], "Surface"),
            decl(NORMAL_STRENGTH, ParamType::Float, [1.0; 4], "Surface"),
            decl(ROUGHNESS_MAP, data, [0.0; 4], "Surface"),
            decl(ROUGHNESS, ParamType::Float, [0.8; 4], "Surface"),
            decl(METALLIC_MAP, data, [0.0; 4], "Surface"),
            decl(METALLIC, ParamType::Float, [0.0; 4], "Surface"),
            decl(OCCLUSION_MAP, data, [0.0; 4], "Surface"),
            decl(EMISSIVE, ParamType::Color, [0.0, 0.0, 0.0, 1.0], "Glow"),
            decl(EMISSIVE_STRENGTH, ParamType::Float, [1.0; 4], "Glow"),
            decl(UV_SCALE, ParamType::Vec2, [1.0; 4], "UV"),
            decl(UV_OFFSET, ParamType::Vec2, [0.0; 4], "UV"),
            decl(WIND, ParamType::Float, [0.0; 4], "Wind"),
            decl(WIND_FLUTTER, ParamType::Float, [0.0; 4], "Wind"),
            decl(WIND_MASK_VERTEX_COLOR, ParamType::Bool, [0.0; 4], "Wind"),
            decl(WIND_HEIGHT, ParamType::Float, [1.0; 4], "Wind"),
        ],
        nodes: Vec::new(),
        wires: Vec::new(),
    };
    let out = g.add_at(NodeKind::Output, [900.0, 200.0]);
    let param = |g: &mut MaterialGraph, name: &str, pos: [f32; 2]| g.add_at(NodeKind::Parameter(name.into()), pos);

    // UV = uv * scale + offset.
    let uv = g.add_at(NodeKind::Uv, [0.0, 0.0]);
    let scale = param(&mut g, UV_SCALE, [0.0, 60.0]);
    let offset = param(&mut g, UV_OFFSET, [0.0, 120.0]);
    let scaled = g.add_at(NodeKind::Math(MathOp::Multiply), [180.0, 20.0]);
    g.connect(uv, 0, scaled, 0);
    g.connect(scale, 0, scaled, 1);
    let uv_final = g.add_at(NodeKind::Math(MathOp::Add), [340.0, 40.0]);
    g.connect(scaled, 0, uv_final, 0);
    g.connect(offset, 0, uv_final, 1);

    // Colour and cut-out.
    let base_map = param(&mut g, BASE_COLOR_MAP, [340.0, -120.0]);
    let base = g.add_at(NodeKind::SampleTexture, [520.0, -120.0]);
    g.connect(base_map, 0, base, 0);
    g.connect(uv_final, 0, base, 1);
    let base_factor = param(&mut g, BASE_COLOR, [520.0, -10.0]);
    let tinted = g.add_at(NodeKind::Math(MathOp::Multiply), [680.0, -100.0]);
    g.connect(base, 0, tinted, 0);
    g.connect(base_factor, 0, tinted, 1);
    let split = g.add_at(NodeKind::Split, [760.0, 0.0]);
    g.connect(tinted, 0, split, 0);
    g.connect(tinted, 0, out, output::BASE_COLOR);
    g.connect(split, 3, out, output::OPACITY);

    // Normal.
    let normal_map = param(&mut g, NORMAL_MAP, [340.0, 160.0]);
    let strength = param(&mut g, NORMAL_STRENGTH, [340.0, 220.0]);
    let normal = g.add_at(NodeKind::NormalMap, [560.0, 170.0]);
    g.connect(normal_map, 0, normal, 0);
    g.connect(uv_final, 0, normal, 1);
    g.connect(strength, 0, normal, 2);
    g.connect(normal, 0, out, output::NORMAL);

    // Roughness, metallic, occlusion: the map's red channel, or the value.
    let data_map = |g: &mut MaterialGraph, map: &str, value: Option<&str>, pin: u8, y: f32| {
        let map = param(g, map, [340.0, y]);
        let sample = g.add_at(NodeKind::SampleTexture, [560.0, y]);
        g.connect(map, 0, sample, 0);
        g.connect(uv_final, 0, sample, 1);
        if let Some(value) = value {
            let value = param(g, value, [340.0, y + 50.0]);
            g.connect(value, 0, sample, 2);
        }
        g.connect(sample, 2, out, pin);
    };
    data_map(&mut g, ROUGHNESS_MAP, Some(ROUGHNESS), output::ROUGHNESS, 280.0);
    data_map(&mut g, METALLIC_MAP, Some(METALLIC), output::METALLIC, 400.0);
    data_map(&mut g, OCCLUSION_MAP, None, output::OCCLUSION, 520.0);

    // Glow: a colour times a strength (above 1 for lamps).
    let emissive = param(&mut g, EMISSIVE, [380.0, 620.0]);
    let strength = param(&mut g, EMISSIVE_STRENGTH, [380.0, 670.0]);
    let glow = g.add_at(NodeKind::Math(MathOp::Multiply), [560.0, 630.0]);
    g.connect(emissive, 0, glow, 0);
    g.connect(strength, 0, glow, 1);
    g.connect(glow, 0, out, output::EMISSIVE);

    // Wind: mask 1, or the vertex colour's red where the switch is on.
    let wind = param(&mut g, WIND, [340.0, 700.0]);
    let flutter = param(&mut g, WIND_FLUTTER, [340.0, 760.0]);
    let use_color = param(&mut g, WIND_MASK_VERTEX_COLOR, [340.0, 820.0]);
    let color = g.add_at(NodeKind::VertexColor, [180.0, 880.0]);
    let color_split = g.add_at(NodeKind::Split, [340.0, 880.0]);
    g.connect(color, 0, color_split, 0);
    let mask = g.add_at(NodeKind::Math(MathOp::Lerp), [520.0, 820.0]);
    g.set_value(mask, 0, [1.0; 4]);
    g.connect(color_split, 0, mask, 1);
    g.connect(use_color, 0, mask, 2);
    let sway = g.add_at(NodeKind::WindSway, [700.0, 740.0]);
    g.connect(wind, 0, sway, 0);
    g.connect(mask, 0, sway, 1);
    g.connect(flutter, 0, sway, 2);
    g.connect(mask, 0, sway, 3);
    let height = param(&mut g, WIND_HEIGHT, [520.0, 940.0]);
    g.connect(height, 0, sway, 4);
    g.connect(sway, 0, out, output::WORLD_OFFSET);
    // Nodes are taller than the positions above were laid out for.
    for node in &mut g.nodes {
        node.position = [node.position[0] * 1.35, node.position[1] * 1.9];
    }
    g
}
