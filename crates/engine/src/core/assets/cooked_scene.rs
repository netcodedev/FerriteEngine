//! An Assimp scene as bytes of the engine's own, and back: what an asset
//! pack keeps of a model file (see docs/asset-pack.md). Reading it is a
//! walk over the bytes; Assimp is only needed when cooking.
//!
//! Everything of `russimp_ng::scene::Scene` is kept, so a model read from
//! a pack behaves exactly as one read from its file: meshes (with bones,
//! colours, every UV set, morph targets), materials (properties and
//! textures, embedded ones shared between materials as in the file), the
//! node tree, animations, cameras, lights and metadata.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use russimp_ng::{
    animation::{Animation, MeshAnim, MeshKey, MeshMorphAnim, MeshMorphKey, NodeAnim, QuatKey, Quaternion, VectorKey},
    bone::{Bone, VertexWeight},
    camera::Camera,
    face::Face,
    light::{Light, LightSourceType},
    material::{DataContent, Material, MaterialProperty, PropertyTypeInfo, Texel, Texture, TextureType},
    mesh::{AnimMesh, Mesh},
    metadata::{MetaData, MetaDataEntry, MetadataType},
    node::Node,
    scene::Scene,
    Color3D, Color4D, Matrix4x4, RussimpError, Vector2D, Vector3D, AABB,
};

/// Changes when the layout below changes: a pack of another version is
/// not read.
pub const VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"FSCN";

/// `scene` as bytes.
pub fn write(scene: &Scene) -> Vec<u8> {
    let mut w = Writer::default();
    w.bytes(MAGIC);
    w.u32(VERSION);
    w.u32(scene.flags);
    // Textures shared by materials are written once and referred to.
    let mut textures: Vec<Rc<RefCell<Texture>>> = Vec::new();
    for material in &scene.materials {
        for texture in material.textures.values() {
            if !textures.iter().any(|t| Rc::ptr_eq(t, texture)) {
                textures.push(texture.clone());
            }
        }
    }
    w.len(textures.len());
    for texture in &textures {
        write_texture(&mut w, &texture.borrow());
    }
    w.len(scene.materials.len());
    for material in &scene.materials {
        let mut slots: Vec<_> = material.textures.iter().collect();
        slots.sort_by_key(|(slot, _)| **slot as u32);
        w.len(slots.len());
        for (slot, texture) in slots {
            w.u32(*slot as u32);
            let index = textures.iter().position(|t| Rc::ptr_eq(t, texture)).unwrap_or(0);
            w.len(index);
        }
        w.len(material.properties.len());
        for property in &material.properties {
            w.string(&property.key);
            w.len(property.index);
            w.u32(property.semantic as u32);
            match &property.data {
                PropertyTypeInfo::Buffer(bytes) => {
                    w.u8(0);
                    w.len(bytes.len());
                    w.bytes(bytes);
                }
                PropertyTypeInfo::IntegerArray(values) => {
                    w.u8(1);
                    w.len(values.len());
                    values.iter().for_each(|v| w.i32(*v));
                }
                PropertyTypeInfo::FloatArray(values) => {
                    w.u8(2);
                    w.len(values.len());
                    values.iter().for_each(|v| w.f32(*v));
                }
                PropertyTypeInfo::String(text) => {
                    w.u8(3);
                    w.string(text);
                }
            }
        }
    }
    w.len(scene.meshes.len());
    for mesh in &scene.meshes {
        write_mesh(&mut w, mesh);
    }
    w.option(scene.metadata.as_ref(), write_metadata);
    w.len(scene.animations.len());
    for animation in &scene.animations {
        write_animation(&mut w, animation);
    }
    w.len(scene.cameras.len());
    for camera in &scene.cameras {
        w.string(&camera.name);
        w.f32(camera.aspect);
        w.f32(camera.clip_plane_far);
        w.f32(camera.clip_plane_near);
        w.f32(camera.horizontal_fov);
        w.vec3(&camera.look_at);
        w.vec3(&camera.position);
        w.vec3(&camera.up);
    }
    w.len(scene.lights.len());
    for light in &scene.lights {
        w.vec3(&light.up);
        w.vec3(&light.pos);
        w.string(&light.name);
        w.f32(light.angle_inner_cone);
        w.f32(light.angle_outer_cone);
        w.f32(light.attenuation_linear);
        w.f32(light.attenuation_quadratic);
        w.f32(light.attenuation_constant);
        w.color3(&light.color_ambient);
        w.color3(&light.color_specular);
        w.color3(&light.color_diffuse);
        w.vec3(&light.direction);
        w.f32(light.size.x);
        w.f32(light.size.y);
        w.u32(light_type(&light.light_source_type));
    }
    w.option(scene.root.as_ref(), |w, root| write_node(w, root));
    w.out
}

/// The scene `bytes` hold (made by `write`).
pub fn read(bytes: &[u8]) -> Result<Scene, String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4)? != MAGIC {
        return Err("not a cooked scene".into());
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(format!("cooked scene version {version}, expected {VERSION}"));
    }
    let flags = r.u32()?;
    let texture_count = r.len()?;
    let mut textures = Vec::with_capacity(texture_count);
    for _ in 0..texture_count {
        textures.push(Rc::new(RefCell::new(read_texture(&mut r)?)));
    }
    let mut materials = Vec::new();
    for _ in 0..r.len()? {
        let mut material_textures = HashMap::new();
        for _ in 0..r.len()? {
            let slot = texture_type(r.u32()?)?;
            let index = r.len()?;
            let texture = textures.get(index).ok_or("texture index out of range")?;
            material_textures.insert(slot, texture.clone());
        }
        let mut properties = Vec::new();
        for _ in 0..r.len()? {
            let key = r.string()?;
            let index = r.len()?;
            let semantic = texture_type(r.u32()?)?;
            let data = match r.u8()? {
                0 => {
                    let n = r.len()?;
                    PropertyTypeInfo::Buffer(r.take(n)?.to_vec())
                }
                1 => PropertyTypeInfo::IntegerArray(r.many(|r| r.i32())?),
                2 => PropertyTypeInfo::FloatArray(r.many(|r| r.f32())?),
                3 => PropertyTypeInfo::String(r.string()?),
                other => return Err(format!("unknown property kind {other}")),
            };
            properties.push(MaterialProperty { key, data, index, semantic });
        }
        materials.push(Material { properties, textures: material_textures });
    }
    let meshes = r.many(read_mesh)?;
    let metadata = r.option(read_metadata)?;
    let animations = r.many(read_animation)?;
    let cameras = r.many(|r| {
        Ok(Camera {
            name: r.string()?,
            aspect: r.f32()?,
            clip_plane_far: r.f32()?,
            clip_plane_near: r.f32()?,
            horizontal_fov: r.f32()?,
            look_at: r.vec3()?,
            position: r.vec3()?,
            up: r.vec3()?,
        })
    })?;
    let lights = r.many(|r| {
        Ok(Light {
            up: r.vec3()?,
            pos: r.vec3()?,
            name: r.string()?,
            angle_inner_cone: r.f32()?,
            angle_outer_cone: r.f32()?,
            attenuation_linear: r.f32()?,
            attenuation_quadratic: r.f32()?,
            attenuation_constant: r.f32()?,
            color_ambient: r.color3()?,
            color_specular: r.color3()?,
            color_diffuse: r.color3()?,
            direction: r.vec3()?,
            size: Vector2D { x: r.f32()?, y: r.f32()? },
            light_source_type: LightSourceType::from(r.u32()?),
        })
    })?;
    let root = r.option(|r| read_node(r, std::rc::Weak::new()))?;
    Ok(Scene { materials, meshes, metadata, animations, cameras, lights, root, flags })
}

fn light_type(kind: &LightSourceType) -> u32 {
    match kind {
        LightSourceType::Ambient => LightSourceType::Ambient.into(),
        LightSourceType::Area => LightSourceType::Area.into(),
        LightSourceType::Directional => LightSourceType::Directional.into(),
        LightSourceType::Point => LightSourceType::Point.into(),
        LightSourceType::Spot => LightSourceType::Spot.into(),
        _ => LightSourceType::Undefined.into(),
    }
}

fn texture_type(value: u32) -> Result<TextureType, String> {
    TextureType::try_from(value).map_err(|_| format!("unknown texture type {value}"))
}

fn write_texture(w: &mut Writer, texture: &Texture) {
    w.u32(texture.height);
    w.u32(texture.width);
    w.string(&texture.filename);
    w.string(&texture.ach_format_hint);
    match &texture.data {
        DataContent::Texel(texels) => {
            w.u8(0);
            w.len(texels.len());
            for t in texels {
                let (b, g, r, a) = (t.b, t.g, t.r, t.a);
                w.bytes(&[b, g, r, a]);
            }
        }
        DataContent::Bytes(bytes) => {
            w.u8(1);
            w.len(bytes.len());
            w.bytes(bytes);
        }
    }
}

fn read_texture(r: &mut Reader) -> Result<Texture, String> {
    let height = r.u32()?;
    let width = r.u32()?;
    let filename = r.string()?;
    let ach_format_hint = r.string()?;
    let data = match r.u8()? {
        0 => DataContent::Texel(r.many(|r| {
            let t = r.take(4)?;
            Ok(Texel { b: t[0], g: t[1], r: t[2], a: t[3] })
        })?),
        1 => {
            let n = r.len()?;
            DataContent::Bytes(r.take(n)?.to_vec())
        }
        other => return Err(format!("unknown texture data kind {other}")),
    };
    Ok(Texture { height, width, filename, ach_format_hint, data })
}

fn write_mesh(w: &mut Writer, mesh: &Mesh) {
    w.string(&mesh.name);
    w.vec3s(&mesh.normals);
    w.vec3s(&mesh.vertices);
    w.len(mesh.texture_coords.len());
    for set in &mesh.texture_coords {
        w.option(set.as_ref(), |w, set| w.vec3s(set));
    }
    w.vec3s(&mesh.tangents);
    w.vec3s(&mesh.bitangents);
    w.len(mesh.uv_components.len());
    mesh.uv_components.iter().for_each(|c| w.u32(*c));
    w.u32(mesh.primitive_types);
    w.len(mesh.bones.len());
    for bone in &mesh.bones {
        w.string(&bone.name);
        w.matrix(&bone.offset_matrix);
        w.len(bone.weights.len());
        for weight in &bone.weights {
            w.u32(weight.vertex_id);
            w.f32(weight.weight);
        }
    }
    w.u32(mesh.material_index);
    w.u32(mesh.method);
    w.len(mesh.anim_meshes.len());
    for anim in &mesh.anim_meshes {
        w.vec3s(&anim.0);
    }
    w.len(mesh.faces.len());
    for face in &mesh.faces {
        w.len(face.0.len());
        face.0.iter().for_each(|i| w.u32(*i));
    }
    w.len(mesh.colors.len());
    for set in &mesh.colors {
        w.option(set.as_ref(), |w, set| {
            w.len(set.len());
            for c in set {
                w.f32(c.r);
                w.f32(c.g);
                w.f32(c.b);
                w.f32(c.a);
            }
        });
    }
    w.vec3(&mesh.aabb.min);
    w.vec3(&mesh.aabb.max);
}

fn read_mesh(r: &mut Reader) -> Result<Mesh, String> {
    let name = r.string()?;
    let normals = r.vec3s()?;
    let vertices = r.vec3s()?;
    let texture_coords = r.many(|r| r.option(|r| r.vec3s()))?;
    let tangents = r.vec3s()?;
    let bitangents = r.vec3s()?;
    let uv_components = r.many(|r| r.u32())?;
    let primitive_types = r.u32()?;
    let bones = r.many(|r| {
        let name = r.string()?;
        let offset_matrix = r.matrix()?;
        let weights = r.many(|r| Ok(VertexWeight { vertex_id: r.u32()?, weight: r.f32()? }))?;
        Ok(Bone { weights, name, offset_matrix })
    })?;
    let material_index = r.u32()?;
    let method = r.u32()?;
    let anim_meshes = r.many(|r| Ok(AnimMesh(r.vec3s()?)))?;
    let faces = r.many(|r| Ok(Face(r.many(|r| r.u32())?)))?;
    let colors = r.many(|r| {
        r.option(|r| r.many(|r| Ok(Color4D { r: r.f32()?, g: r.f32()?, b: r.f32()?, a: r.f32()? })))
    })?;
    let aabb = AABB { min: r.vec3()?, max: r.vec3()? };
    Ok(Mesh {
        normals,
        name,
        vertices,
        texture_coords,
        tangents,
        bitangents,
        uv_components,
        primitive_types,
        bones,
        material_index,
        method,
        anim_meshes,
        faces,
        colors,
        aabb,
    })
}

fn write_metadata(w: &mut Writer, metadata: &MetaData) {
    w.len(metadata.keys.len());
    for key in &metadata.keys {
        w.string(key);
    }
    w.len(metadata.values.len());
    for value in &metadata.values {
        match &value.0 {
            Ok(MetadataType::String(s)) => {
                w.u8(0);
                w.string(s);
            }
            Ok(MetadataType::Vector3d(v)) => {
                w.u8(1);
                w.vec3(v);
            }
            Ok(MetadataType::Bool(b)) => {
                w.u8(2);
                w.u8(*b as u8);
            }
            Ok(MetadataType::Float(f)) => {
                w.u8(3);
                w.f32(*f);
            }
            Ok(MetadataType::Double(d)) => {
                w.u8(4);
                w.f64(*d);
            }
            Ok(MetadataType::Int(i)) => {
                w.u8(5);
                w.i32(*i);
            }
            Ok(MetadataType::ULong(u)) => {
                w.u8(6);
                w.u64(*u);
            }
            Err(e) => {
                w.u8(7);
                w.string(&e.to_string());
            }
        }
    }
}

fn read_metadata(r: &mut Reader) -> Result<MetaData, String> {
    let keys = r.many(|r| r.string())?;
    let values = r.many(|r| {
        Ok(MetaDataEntry(match r.u8()? {
            0 => Ok(MetadataType::String(r.string()?)),
            1 => Ok(MetadataType::Vector3d(r.vec3()?)),
            2 => Ok(MetadataType::Bool(r.u8()? != 0)),
            3 => Ok(MetadataType::Float(r.f32()?)),
            4 => Ok(MetadataType::Double(r.f64()?)),
            5 => Ok(MetadataType::Int(r.i32()?)),
            6 => Ok(MetadataType::ULong(r.u64()?)),
            7 => Err(RussimpError::MetadataError(r.string()?)),
            other => return Err(format!("unknown metadata kind {other}")),
        }))
    })?;
    Ok(MetaData { keys, values })
}

fn write_animation(w: &mut Writer, animation: &Animation) {
    w.string(&animation.name);
    w.f64(animation.duration);
    w.f64(animation.ticks_per_second);
    w.len(animation.channels.len());
    for channel in &animation.channels {
        w.string(&channel.name);
        let vector_keys = |w: &mut Writer, keys: &[VectorKey]| {
            w.len(keys.len());
            for key in keys {
                w.f64(key.time);
                w.vec3(&key.value);
            }
        };
        vector_keys(w, &channel.position_keys);
        w.len(channel.rotation_keys.len());
        for key in &channel.rotation_keys {
            w.f64(key.time);
            w.f32(key.value.w);
            w.f32(key.value.x);
            w.f32(key.value.y);
            w.f32(key.value.z);
        }
        vector_keys(w, &channel.scaling_keys);
        w.u32(channel.post_state);
        w.u32(channel.pre_state);
    }
    w.len(animation.morph_mesh_channels.len());
    for channel in &animation.morph_mesh_channels {
        w.string(&channel.name);
        w.len(channel.keys.len());
        for key in &channel.keys {
            w.f64(key.time);
            w.len(key.values.len());
            key.values.iter().for_each(|v| w.u32(*v));
            w.len(key.weights.len());
            key.weights.iter().for_each(|v| w.f64(*v));
        }
    }
    w.len(animation.mesh_channels.len());
    for channel in &animation.mesh_channels {
        w.string(&channel.name);
        w.len(channel.keys.len());
        for key in &channel.keys {
            w.f64(key.time);
            w.u32(key.value);
        }
    }
}

fn read_animation(r: &mut Reader) -> Result<Animation, String> {
    let name = r.string()?;
    let duration = r.f64()?;
    let ticks_per_second = r.f64()?;
    let channels = r.many(|r| {
        let name = r.string()?;
        let position_keys = r.many(|r| Ok(VectorKey { time: r.f64()?, value: r.vec3()? }))?;
        let rotation_keys = r.many(|r| {
            let time = r.f64()?;
            let value = Quaternion { w: r.f32()?, x: r.f32()?, y: r.f32()?, z: r.f32()? };
            Ok(QuatKey { time, value })
        })?;
        let scaling_keys = r.many(|r| Ok(VectorKey { time: r.f64()?, value: r.vec3()? }))?;
        let post_state = r.u32()?;
        let pre_state = r.u32()?;
        Ok(NodeAnim { name, position_keys, rotation_keys, scaling_keys, post_state, pre_state })
    })?;
    let morph_mesh_channels = r.many(|r| {
        let name = r.string()?;
        let keys = r.many(|r| {
            Ok(MeshMorphKey { time: r.f64()?, values: r.many(|r| r.u32())?, weights: r.many(|r| r.f64())? })
        })?;
        Ok(MeshMorphAnim { keys, name })
    })?;
    let mesh_channels = r.many(|r| {
        let name = r.string()?;
        let keys = r.many(|r| Ok(MeshKey { time: r.f64()?, value: r.u32()? }))?;
        Ok(MeshAnim { name, keys })
    })?;
    Ok(Animation { name, channels, duration, morph_mesh_channels, mesh_channels, ticks_per_second })
}

fn write_node(w: &mut Writer, node: &Node) {
    w.string(&node.name);
    w.matrix(&node.transformation);
    w.len(node.meshes.len());
    node.meshes.iter().for_each(|m| w.u32(*m));
    w.option(node.metadata.as_ref(), write_metadata);
    let children = node.children.borrow();
    w.len(children.len());
    for child in children.iter() {
        write_node(w, child);
    }
}

fn read_node(r: &mut Reader, parent: std::rc::Weak<Node>) -> Result<Rc<Node>, String> {
    let name = r.string()?;
    let transformation = r.matrix()?;
    let meshes = r.many(|r| r.u32())?;
    let metadata = r.option(read_metadata)?;
    let count = r.len()?;
    // The children are read inside so they can point at this node.
    let mut failed = None;
    let node = Rc::new_cyclic(|me| {
        let mut children = Vec::with_capacity(count);
        for _ in 0..count {
            match read_node(r, me.clone()) {
                Ok(child) => children.push(child),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        Node { name, children: RefCell::new(children), meshes, metadata, transformation, parent }
    });
    match failed {
        Some(e) => Err(e),
        None => Ok(node),
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u64(n as u64);
    }
    fn string(&mut self, s: &str) {
        self.len(s.len());
        self.bytes(s.as_bytes());
    }
    fn vec3(&mut self, v: &Vector3D) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    fn vec3s(&mut self, vs: &[Vector3D]) {
        self.len(vs.len());
        vs.iter().for_each(|v| self.vec3(v));
    }
    fn color3(&mut self, c: &Color3D) {
        self.f32(c.r);
        self.f32(c.g);
        self.f32(c.b);
    }
    fn matrix(&mut self, m: &Matrix4x4) {
        for v in [m.a1, m.a2, m.a3, m.a4, m.b1, m.b2, m.b3, m.b4, m.c1, m.c2, m.c3, m.c4, m.d1, m.d2, m.d3, m.d4] {
            self.f32(v);
        }
    }
    fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Writer, &T)) {
        match value {
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
            None => self.u8(0),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len()).ok_or("cooked scene cut short")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.array()?))
    }
    fn len(&mut self) -> Result<usize, String> {
        let n = self.u64()?;
        // Never more items than bytes left: a broken file fails, it does
        // not allocate without end.
        if n > (self.bytes.len() - self.at) as u64 * 8 + 1024 {
            return Err("cooked scene: a count out of range".into());
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
    }
    fn vec3(&mut self) -> Result<Vector3D, String> {
        Ok(Vector3D { x: self.f32()?, y: self.f32()?, z: self.f32()? })
    }
    fn vec3s(&mut self) -> Result<Vec<Vector3D>, String> {
        self.many(|r| r.vec3())
    }
    fn color3(&mut self) -> Result<Color3D, String> {
        Ok(Color3D { r: self.f32()?, g: self.f32()?, b: self.f32()? })
    }
    fn matrix(&mut self) -> Result<Matrix4x4, String> {
        let mut v = [0.0f32; 16];
        for x in &mut v {
            *x = self.f32()?;
        }
        Ok(Matrix4x4 {
            a1: v[0],
            a2: v[1],
            a3: v[2],
            a4: v[3],
            b1: v[4],
            b2: v[5],
            b3: v[6],
            b4: v[7],
            c1: v[8],
            c2: v[9],
            c3: v[10],
            c4: v[11],
            d1: v[12],
            d2: v[13],
            d3: v[14],
            d4: v[15],
        })
    }
    fn many<T>(&mut self, mut read: impl FnMut(&mut Self) -> Result<T, String>) -> Result<Vec<T>, String> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            out.push(read(self)?);
        }
        Ok(out)
    }
    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<Option<T>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(read(self)?)),
            other => Err(format!("cooked scene: option tag {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Models written as the engine's tests write them (meshes, a
    /// textured material, a node tree, an animation), written and read
    /// back: the same in every field (compared as Assimp's own debug
    /// print).
    #[test]
    fn scenes_come_back_as_they_were_written() {
        use super::super::test_models;
        let dir = std::env::temp_dir().join(format!("ferrite-cooked-scene-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("thing.gltf");
        test_models::write_gltf(&model, &[("tree", &["tree_LOD0", "tree_LOD1"]), ("rock", &[])], Some("bark.png"));
        test_models::write_png(&dir.join("bark.png"));
        let clip = dir.join("clip.gltf");
        test_models::write_animation_gltf(&clip, "wave");
        for path in [model, clip] {
            let scene = crate::core::model::read_scene(&path.to_string_lossy()).unwrap();
            let bytes = write(&scene);
            let back = read(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(format!("{:?}", scene.meshes), format!("{:?}", back.meshes));
            assert_eq!(format!("{:?}", scene.animations), format!("{:?}", back.animations));
            assert_eq!(format!("{:?}", scene.root), format!("{:?}", back.root));
            assert_eq!(scene.materials.len(), back.materials.len());
            for (a, b) in scene.materials.iter().zip(&back.materials) {
                assert_eq!(format!("{:?}", a.properties), format!("{:?}", b.properties));
                let mut keys_a: Vec<u32> = a.textures.keys().map(|k| *k as u32).collect();
                let mut keys_b: Vec<u32> = b.textures.keys().map(|k| *k as u32).collect();
                keys_a.sort();
                keys_b.sort();
                assert_eq!(keys_a, keys_b);
            }
            // Children point back at their parent.
            if let Some(root) = &back.root {
                for child in root.children.borrow().iter() {
                    assert!(child.parent.upgrade().is_some_and(|p| std::rc::Rc::ptr_eq(&p, root)));
                }
            }
            assert_eq!(write(&back), bytes);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cut_file_is_an_error() {
        let scene = Scene {
            materials: Vec::new(),
            meshes: vec![Mesh { name: "m".into(), vertices: vec![Vector3D::default(); 3], ..Default::default() }],
            metadata: None,
            animations: Vec::new(),
            cameras: Vec::new(),
            lights: Vec::new(),
            root: None,
            flags: 0,
        };
        let bytes = write(&scene);
        assert!(read(&bytes).is_ok());
        assert!(read(&bytes[..bytes.len() - 5]).is_err());
        assert!(read(b"nope").is_err());
    }
}
