//! The bridges of the road plan near the camera (see
//! `generator::bridges`): each a model of its own with a collider, made
//! when the camera comes near and dropped when it leaves.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use cgmath::{Matrix4, Point3, Vector3};
use image::RgbaImage;
use rapier3d::prelude::*;
use russimp_ng::material::TextureType;

use crate::core::{
    entity::{
        component::{Component, Property},
        Entity,
    },
    headless,
    model::{generated, Model},
    physics::{physics_engine::collider_tag, rigidbody::RigidBody},
    renderer::{pbr::Lighting, texture::Texture, weather::exposure},
    scene::Scene,
};

use super::{
    generator::{
        bridges::{self, Bridge},
        WorldGenerator,
    },
    materials,
};

/// Bridges within this many metres of the camera are shown, and those
/// further than `DROP` dropped.
const SHOW: f64 = 2500.0;
const DROP: f64 = 3000.0;
/// How many bridges are made in one frame at most.
const PER_FRAME: usize = 2;

/// One bridge on its entity: its model (none without a GPU).
pub struct BridgeComponent {
    index: usize,
    model: Option<Model>,
    triangles: usize,
}

impl BridgeComponent {
    pub fn index(&self) -> usize {
        self.index
    }
}

impl Component for BridgeComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(&self, scene: &Scene, _: &Entity, view_projection: &Matrix4<f32>, parent_transform: &Matrix4<f32>) {
        // The ground map holds the terrain alone.
        if exposure::in_ground_pass() {
            return;
        }
        if let (Some(model), Some(lighting)) = (&self.model, Lighting::from_scene(scene)) {
            model.render(&lighting, parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}

    fn raycast(&self, transform: &Matrix4<f32>, origin: Point3<f32>, dir: Vector3<f32>) -> Option<f32> {
        self.model.as_ref()?.raycast(transform, origin, dir)
    }

    fn display_name(&self) -> String {
        "Bridge".into()
    }

    fn properties(&self) -> Vec<Property> {
        vec![Property::text("triangles", self.triangles.to_string()).read_only()]
    }
}

/// The textures the bridges are drawn with, uploaded once and shared:
/// stone (colour, normal) and road (colour, normal).
struct Textures {
    /// Colour, normal, roughness and occlusion; drawn in code (colour and
    /// normal only) without a texture of its own in the project.
    stone: Vec<(TextureType, Texture)>,
    road: (Texture, Option<Texture>),
}

thread_local! {
    static TEXTURES: RefCell<Option<Rc<Textures>>> = const { RefCell::new(None) };
}

/// Free the shared bridge textures while the GL context is still current
/// (see `Application`'s `Drop`): a thread-local's destructor would run after
/// the GL driver is gone, and crash in it.
pub fn release_gl_resources() {
    TEXTURES.with(|cell| cell.borrow_mut().take());
}

fn textures() -> Rc<Textures> {
    TEXTURES.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(|| {
                let stone = match materials::bridge_images() {
                    Some(maps) => maps
                        .into_iter()
                        .zip([TextureType::Diffuse, TextureType::Normals, TextureType::Roughness, TextureType::AmbientOcclusion])
                        .filter_map(|(image, slot)| Some((slot, upload(&image?, slot == TextureType::Diffuse))))
                        .collect(),
                    None => {
                        let (color, normal) = masonry();
                        vec![(TextureType::Diffuse, upload(&color, true)), (TextureType::Normals, upload(&normal, false))]
                    }
                };
                let (road, road_normal) = match materials::road_images() {
                    Some((color, normal)) => bake_road(&color, normal.as_ref()),
                    None => (plain([92, 88, 82]), None),
                };
                Rc::new(Textures {
                    stone,
                    road: (upload(&road, true), road_normal.map(|n| upload(&n, false))),
                })
            })
            .clone()
    })
}

/// Metres one repeat of the terrain's road texture covers (its layer's
/// tile size), and pixels per metre of the baked road.
const ROAD_SOURCE_TILE: f32 = 2.0;
const ROAD_PIXELS_PER_METRE: u32 = 384;

/// The road's texture laid out over `bridges::ROAD_TILE` metres without
/// repeats, as the terrain lays it (see `sampleNoTile` in its shader): a
/// smooth noise picks one of many shifted placements of the texture, and
/// where it moves from one to the next the two are blended. The noise
/// wraps, so the result tiles.
fn bake_road(color: &RgbaImage, normal: Option<&RgbaImage>) -> (RgbaImage, Option<RgbaImage>) {
    use image::imageops::{resize, FilterType};
    let tile_pixels = (ROAD_SOURCE_TILE * ROAD_PIXELS_PER_METRE as f32) as u32;
    let size = (bridges::ROAD_TILE * ROAD_PIXELS_PER_METRE as f32) as u32;
    let tiles = bridges::ROAD_TILE / ROAD_SOURCE_TILE;
    let sources: Vec<RgbaImage> = std::iter::once(color)
        .chain(normal)
        .map(|image| resize(image, tile_pixels, tile_pixels, FilterType::Triangle))
        .collect();
    // Noise cells per tile and placements across its range, as the
    // terrain's; the noise wraps every `period` cells.
    const SCALE: f32 = 0.5;
    const STEPS: f32 = 12.0;
    let period = (tiles * SCALE).round() as i32;
    let hash = |x: f32, y: f32| {
        let v = (x * 127.1 + y * 311.7).sin() * 43758.547;
        v - v.floor()
    };
    let noise = |x: f32, y: f32| {
        let (i, j) = (x.floor(), y.floor());
        let (fx, fy) = (x - i, y - j);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let at = |a: f32, b: f32| hash((a as i32).rem_euclid(period) as f32, (b as i32).rem_euclid(period) as f32);
        let top = at(i, j) + (at(i + 1.0, j) - at(i, j)) * sx;
        let bottom = at(i, j + 1.0) + (at(i + 1.0, j + 1.0) - at(i, j + 1.0)) * sx;
        top + (bottom - top) * sy
    };
    // A source's texel at `(u, v)` tiles, bilinear, wrapping.
    let sample = |image: &RgbaImage, u: f32, v: f32| {
        let (x, y) = (u.rem_euclid(1.0) * tile_pixels as f32 - 0.5, v.rem_euclid(1.0) * tile_pixels as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |dx: f32, dy: f32| {
            let px = ((x0 + dx) as i32).rem_euclid(tile_pixels as i32) as u32;
            let py = ((y0 + dy) as i32).rem_euclid(tile_pixels as i32) as u32;
            image.get_pixel(px, py).0.map(|c| c as f32)
        };
        let (a, b, c, d) = (texel(0.0, 0.0), texel(1.0, 0.0), texel(0.0, 1.0), texel(1.0, 1.0));
        std::array::from_fn::<f32, 4, _>(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    };
    let placed = |image: &RgbaImage, u: f32, v: f32, i: f32| {
        let (h2, h3) = (hash(i + 19.1, 3.17), hash(i * 1.37 + 5.3, 11.0));
        sample(image, u + h2 * 7.0, v + h3 * 7.0)
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = (size as usize).div_ceil(threads);
    let mut pixels: Vec<Vec<u8>> = sources.iter().map(|_| vec![0u8; (size * size * 4) as usize]).collect();
    std::thread::scope(|scope| {
        let mut chunks: Vec<Vec<&mut [u8]>> = (0..threads).map(|_| Vec::new()).collect();
        for buffer in pixels.iter_mut() {
            for (k, chunk) in buffer.chunks_mut(rows * size as usize * 4).enumerate() {
                chunks[k].push(chunk);
            }
        }
        for (k, mut chunk) in chunks.into_iter().enumerate() {
            let (sources, noise, placed) = (&sources, &noise, &placed);
            scope.spawn(move || {
                let first = k * rows;
                let count = chunk.first().map_or(0, |c| c.len() / (size as usize * 4));
                for row in 0..count {
                    for x in 0..size as usize {
                        let (u, v) = (
                            x as f32 / size as f32 * tiles,
                            (first + row) as f32 / size as f32 * tiles,
                        );
                        let l = noise(u * SCALE, v * SCALE) * STEPS;
                        let (i, f) = (l.floor(), l - l.floor());
                        let w = ((f - 0.42) / 0.16).clamp(0.0, 1.0);
                        let w = w * w * (3.0 - 2.0 * w);
                        for (source, out) in sources.iter().zip(chunk.iter_mut()) {
                            let a = placed(source, u, v, i);
                            let c = if w > 0.0 {
                                let b = placed(source, u, v, i + 1.0);
                                std::array::from_fn::<f32, 4, _>(|k| a[k] + (b[k] - a[k]) * w)
                            } else {
                                a
                            };
                            let at = (row * size as usize + x) * 4;
                            for k in 0..4 {
                                out[at + k] = c[k].round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                }
            });
        }
    });
    let mut images = pixels
        .into_iter()
        .map(|raw| RgbaImage::from_raw(size, size, raw).expect("a square image"));
    let color = images.next().expect("the colour");
    (color, images.next())
}

fn plain(color: [u8; 3]) -> RgbaImage {
    RgbaImage::from_pixel(4, 4, image::Rgba([color[0], color[1], color[2], 255]))
}

/// Dressed stone in courses, each block a little different, the joints
/// between them sunk in (its normal map too). One repeat is a bridge's
/// stone tile (see `bridges::STONE_TILE`): four courses high, two to three
/// blocks across.
fn masonry() -> (RgbaImage, RgbaImage) {
    const SIZE: u32 = 512;
    const COURSES: u32 = 4;
    let hash = |a: u32, b: u32| {
        let mut h = a.wrapping_mul(374761393) ^ b.wrapping_mul(668265263) ^ 0x9E37_79B9;
        h = (h ^ (h >> 13)).wrapping_mul(1274126177);
        ((h ^ (h >> 16)) & 0xFFFF) as f32 / 65535.0
    };
    // How far into a block a point lies (0 at a joint, 1 well inside), and
    // which block it is.
    let block = |x: f32, y: f32| {
        let course = (y * COURSES as f32).floor();
        let row = course as u32 % COURSES;
        let v = y * COURSES as f32 - course;
        // Each course its own joints, shifted.
        let per = 2.0 + (hash(row, 7) * 1.99).floor();
        let shifted = x * per + hash(row, 3);
        let column = shifted.floor();
        let u = shifted - column;
        let edge = (u.min(1.0 - u) / per * 4.0).min(v.min(1.0 - v) / COURSES as f32 * 4.0);
        ((edge * 60.0).min(1.0), (row, column as u32 % 8))
    };
    let mut color = RgbaImage::new(SIZE, SIZE);
    let mut height = vec![0f32; (SIZE * SIZE) as usize];
    for py in 0..SIZE {
        for px in 0..SIZE {
            let (x, y) = (px as f32 / SIZE as f32, py as f32 / SIZE as f32);
            let (inside, (row, column)) = block(x, y);
            let tone = hash(row * 31 + column, 11);
            let grain = hash(px, py);
            let base = [0.47 + 0.12 * tone, 0.43 + 0.1 * tone, 0.37 + 0.08 * tone];
            let mortar = [0.30, 0.28, 0.25];
            let shade = 0.88 + 0.12 * grain;
            let c: Vec<u8> = (0..3)
                .map(|i| ((mortar[i] + (base[i] - mortar[i]) * inside) * shade * 255.0).round() as u8)
                .collect();
            color.put_pixel(px, py, image::Rgba([c[0], c[1], c[2], 255]));
            height[(py * SIZE + px) as usize] = inside + 0.08 * grain;
        }
    }
    // The normal from the height's slope (wrapping, so it tiles).
    let mut normal = RgbaImage::new(SIZE, SIZE);
    let h = |x: i32, y: i32| height[(y.rem_euclid(SIZE as i32) as u32 * SIZE + x.rem_euclid(SIZE as i32) as u32) as usize];
    for py in 0..SIZE as i32 {
        for px in 0..SIZE as i32 {
            let (dx, dy) = (h(px + 1, py) - h(px - 1, py), h(px, py + 1) - h(px, py - 1));
            let n = Vector3::new(-dx * 2.0, dy * 2.0, 1.0);
            let n = n / (n.x * n.x + n.y * n.y + n.z * n.z).sqrt();
            let byte = |v: f32| ((v * 0.5 + 0.5) * 255.0).round() as u8;
            normal.put_pixel(px as u32, py as u32, image::Rgba([byte(n.x), byte(n.y), byte(n.z), 255]));
        }
    }
    (color, normal)
}

fn upload(image: &RgbaImage, srgb: bool) -> Texture {
    let texture = Texture::new();
    texture.load_material_map(image.width(), image.height(), image.as_raw().clone(), srgb);
    texture
}

/// The bridge `index` of `bridge` as an entity: its model and a fixed body
/// with its collider.
fn make(scene: &mut Scene, bridge: &Bridge, index: usize) -> Entity {
    let mesh = bridge.mesh();
    let triangles = mesh.stone.triangles.len() + mesh.road.triangles.len();
    let model = (!headless::is_headless())
        .then(|| match generated::build_model(&[("stone", &mesh.stone), ("road", &mesh.road)]) {
            Ok(mut model) => {
                let textures = textures();
                for material in 0..model.material_count() {
                    let name = model.material_name_at(material).unwrap_or_default();
                    if name.contains("road") {
                        model.set_material_texture(material, TextureType::Diffuse, textures.road.0.share());
                        if let Some(normal) = &textures.road.1 {
                            model.set_material_texture(material, TextureType::Normals, normal.share());
                        }
                    } else if name.contains("stone") {
                        for (slot, texture) in &textures.stone {
                            model.set_material_texture(material, *slot, texture.share());
                        }
                    }
                }
                Some(model)
            }
            Err(e) => {
                log::warn!("bridge {index}: {e}");
                None
            }
        })
        .flatten();
    let mut entity = Entity::new(&format!("bridge#{index}"));
    let body = RigidBody::new(RigidBodyType::Fixed, scene, &entity, None);
    // Both parts in one collider, in world metres like the chunks'.
    let mut vertices: Vec<Vector> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    for part in [&mesh.stone, &mesh.road] {
        let base = vertices.len() as u32;
        vertices.extend(part.positions.iter().map(|p| Vector::new(p[0], p[1], p[2])));
        indices.extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
    }
    match ColliderBuilder::trimesh_with_flags(vertices, indices, TriMeshFlags::FIX_INTERNAL_EDGES) {
        Ok(builder) => {
            let collider = builder.user_data(collider_tag::TERRAIN).build();
            scene.physics_engine.add_collider(collider, Some(body.rigid_body_handle));
        }
        Err(e) => log::warn!("bridge {index} has no collider: {e:?}"),
    }
    entity.add_component(BridgeComponent { index, model, triangles });
    entity.add_component(body);
    entity.set_transient(true);
    entity
}

/// Bring the bridges among `entity`'s children in line with the camera at
/// `eye`: make the near ones that are missing (a few a frame), drop the
/// far ones.
pub fn sync(generator: &WorldGenerator, scene: &mut Scene, entity: &mut Entity, eye: Point3<f32>) {
    // (Not before the roads are planned: that is not waited for here.)
    let Some(roads) = generator.planned_roads() else {
        return;
    };
    let distance = |bridge: &Bridge| {
        let (lo, hi) = (bridge.lo(), bridge.hi());
        let dx = (lo[0] - eye.x as f64).max(eye.x as f64 - hi[0]).max(0.0);
        let dz = (lo[1] - eye.z as f64).max(eye.z as f64 - hi[1]).max(0.0);
        dx.hypot(dz)
    };
    let bridges = roads.bridges();
    let children = entity.get_children_mut();
    let mut shown = HashSet::new();
    let mut index = 0;
    while index < children.len() {
        let Some(at) = children[index].get_own_component::<BridgeComponent>().map(|b| b.index) else {
            index += 1;
            continue;
        };
        if bridges.get(at).is_none_or(|b| distance(b) > DROP) {
            let mut gone = children.remove(index);
            gone.release_resources(&mut scene.physics_engine);
        } else {
            shown.insert(at);
            index += 1;
        }
    }
    let mut wanted: Vec<(f64, usize)> = bridges
        .iter()
        .enumerate()
        .filter(|(i, b)| !shown.contains(i) && distance(b) < SHOW)
        .map(|(i, b)| (distance(b), i))
        .collect();
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, i) in wanted.iter().take(PER_FRAME) {
        let child = make(scene, &bridges[i], i);
        entity.add_child(child);
    }
}

/// Take every bridge out of `entity`'s children (the world changed).
pub fn clear(scene: &mut Scene, entity: &mut Entity) {
    let children = entity.get_children_mut();
    let mut index = 0;
    while index < children.len() {
        if children[index].get_own_component::<BridgeComponent>().is_some() {
            let mut gone = children.remove(index);
            gone.release_resources(&mut scene.physics_engine);
        } else {
            index += 1;
        }
    }
}
