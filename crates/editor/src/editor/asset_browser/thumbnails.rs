//! The pictures on the browser's cards: textures shrunk, materials on a
//! sphere, models from the front. Each is made once and kept as a PNG in
//! `.ferrite-cache/thumbnails/` with a key file beside it; it is made
//! again only when the key (the asset's settings, its file's size and time,
//! and those of the assets it uses) no longer fits.
//!
//! Textures are shrunk on a worker thread. Materials and models are drawn
//! on the render thread into a picture of their own, at most one per frame.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver, Sender},
    time::{Instant, UNIX_EPOCH},
};

use cgmath::{Matrix4, Point3, SquareMatrix, Vector3};
use ferrite::{
    core::{
        assets::{self, AssetDatabase, AssetId, AssetKind, AssetRef},
        gfx::{self, Format, Load, TextureDesc},
        material::{
            asset::{self as material_asset, GraphRef, MaterialFile, TextureSetting, ParamSetting},
            preview::Shape,
            AlphaMode,
        },
        model::{Model, ModelBuilder},
        primitives::{Position, Size},
        renderer::{hdr, pbr::Lighting},
    },
    terrain::scatter::{graph::Placement, LodChoice, ScatterInstances, ScatterModel, ScatterSpec},
};
use ferrite_ui::ui::{elements::image::Image, UIElement};

/// Pixels of a thumbnail, both ways.
pub const SIZE: u32 = 128;
/// Folder below the project for the pictures.
const FOLDER: &str = ".ferrite-cache/thumbnails";
/// Keys of the pictures on screen are checked again this often, to notice
/// files saved elsewhere.
const RECHECK_SECONDS: f32 = 1.5;

/// Whether cards of this kind get a picture.
pub fn has_thumbnail(kind: &AssetKind) -> bool {
    matches!(kind, AssetKind::Texture(_) | AssetKind::Material(_) | AssetKind::MaterialGraph(_) | AssetKind::Model(_))
}

/// FNV-1a, for keys that stay the same from run to run.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }

    fn add(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
        // A separator, so "ab"+"c" and "a"+"bc" differ.
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }
}

/// Size and change time of a file, as text.
fn file_stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(meta) => {
            let time = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            format!("{}:{time}", meta.len())
        }
        Err(_) => "missing".into(),
    }
}

/// The assets an asset's picture depends on: a material's graph and
/// textures, a model's materials.
fn dependencies(database: &AssetDatabase, id: AssetId) -> Vec<AssetId> {
    let Some(entry) = database.get(id) else {
        return Vec::new();
    };
    match &entry.meta.kind {
        AssetKind::Material(_) => match MaterialFile::load(&entry.source_path()) {
            Ok(file) => {
                let mut ids = Vec::new();
                if let GraphRef::Asset(graph) = file.graph {
                    ids.push(graph);
                }
                for setting in file.params.values() {
                    if let ParamSetting::Texture(TextureSetting::Asset(texture)) = setting {
                        ids.push(*texture);
                    }
                }
                ids
            }
            Err(_) => Vec::new(),
        },
        AssetKind::Model(model) => model.materials.iter().filter_map(|slot| slot.material).collect(),
        _ => Vec::new(),
    }
}

fn add_asset(database: &AssetDatabase, id: AssetId, hash: &mut Fnv, depth: u32) {
    let Some(entry) = database.get(id) else {
        hash.add(b"gone");
        return;
    };
    hash.add(entry.meta.to_ron().unwrap_or_default().as_bytes());
    hash.add(file_stamp(&entry.source_path()).as_bytes());
    if depth < 3 {
        for dependency in dependencies(database, id) {
            add_asset(database, dependency, hash, depth + 1);
        }
    }
}

/// The key of an asset's (or part's) picture.
fn key_of(database: &AssetDatabase, asset: &AssetRef) -> String {
    let mut hash = Fnv::new();
    hash.add(b"thumbnail v2");
    hash.add(asset.part.as_deref().unwrap_or("").as_bytes());
    add_asset(database, asset.id, &mut hash, 0);
    format!("{:016x}", hash.0)
}

/// File name of a picture, without the folder.
fn file_stem(asset: &AssetRef) -> String {
    match &asset.part {
        None => asset.id.to_string(),
        Some(part) => {
            let mut hash = Fnv::new();
            hash.add(part.as_bytes());
            format!("{}-{:016x}", asset.id, hash.0)
        }
    }
}

enum Slot {
    /// Being made (queued or on the worker).
    Waiting { key: String },
    Ready { key: String, image: Image },
    /// Could not be made; the card shows its icon. Tried again when the
    /// key changes.
    Failed { key: String },
}

impl Slot {
    fn key(&self) -> &str {
        match self {
            Slot::Waiting { key } | Slot::Ready { key, .. } | Slot::Failed { key } => key,
        }
    }
}

/// Work for the texture thread.
struct Job {
    asset: AssetRef,
    source: PathBuf,
    png: PathBuf,
    key: String,
}

/// What the texture thread made.
struct Done {
    asset: AssetRef,
    key: String,
    picture: Result<image::RgbaImage, String>,
}

/// A picture still to be drawn on the render thread.
struct Draw {
    asset: AssetRef,
    key: String,
}

pub struct Thumbnails {
    folder: PathBuf,
    slots: HashMap<AssetRef, Slot>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// Drawn on the render thread, one per frame (`render`).
    queue: RefCell<VecDeque<Draw>>,
    drawn: RefCell<Vec<(AssetRef, String, Result<image::RgbaImage, String>)>>,
    sphere: RefCell<Option<Model>>,
    /// No picture is drawn before this: after one that took `t`, the
    /// editor gets at least `t` to itself, so it stays usable meanwhile.
    rest_until: std::cell::Cell<Instant>,
    /// What the loader made of the materials of recently loaded model
    /// files (by model asset), for materials whose maps are "auto".
    file_materials: RefCell<VecDeque<(AssetId, Vec<Option<ferrite::core::material::Material>>)>>,
    /// Database state and time of the last key check.
    checked: (u64, Instant),
}

impl Thumbnails {
    pub fn new(project: &Path) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, done) = channel::<Done>();
        std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                while let Ok(job) = inbox.recv() {
                    let picture = shrink(&job.source);
                    if let Ok(picture) = &picture {
                        save(&job.png, &job.key, picture);
                    }
                    if outbox.send(Done { asset: job.asset, key: job.key, picture }).is_err() {
                        break;
                    }
                }
            })
            .expect("the thumbnail thread starts");
        Self {
            folder: project.join(FOLDER),
            slots: HashMap::new(),
            jobs,
            done,
            queue: RefCell::new(VecDeque::new()),
            drawn: RefCell::new(Vec::new()),
            sphere: RefCell::new(None),
            rest_until: std::cell::Cell::new(Instant::now()),
            file_materials: RefCell::new(VecDeque::new()),
            checked: (u64::MAX, Instant::now()),
        }
    }

    /// The project changed: forget everything.
    pub fn reopen(&mut self, project: &Path) {
        self.folder = project.join(FOLDER);
        self.slots.clear();
        self.queue.borrow_mut().clear();
    }

    fn png_of(&self, asset: &AssetRef) -> PathBuf {
        self.folder.join(format!("{}.png", file_stem(asset)))
    }

    /// Ask for the pictures of the cards on screen; call every update.
    /// Takes what the worker and the last render made, and starts what is
    /// missing or out of date.
    pub fn want(&mut self, database: &AssetDatabase, visible: &[AssetRef]) {
        while let Ok(done) = self.done.try_recv() {
            self.finish(done.asset, done.key, done.picture);
        }
        let drawn = std::mem::take(&mut *self.drawn.borrow_mut());
        for (asset, key, picture) in drawn {
            if let Ok(picture) = &picture {
                save(&self.png_of(&asset), &key, picture);
            }
            self.finish(asset, key, picture);
        }

        // Keys are made again when the database changed, or now and then.
        let recheck = self.checked.0 != database.changes() || self.checked.1.elapsed().as_secs_f32() > RECHECK_SECONDS;
        if recheck {
            self.checked = (database.changes(), Instant::now());
        }
        for asset in visible {
            let Some(entry) = database.get(asset.id) else {
                continue;
            };
            if !has_thumbnail(&entry.meta.kind) {
                continue;
            }
            if self.slots.contains_key(asset) && !recheck {
                continue;
            }
            let key = key_of(database, asset);
            if self.slots.get(asset).is_some_and(|slot| slot.key() == key) {
                continue;
            }
            // Made in an earlier run, or by another editor?
            let png = self.png_of(asset);
            let stored = fs::read_to_string(png.with_extension("key")).ok();
            if stored.as_deref().map(str::trim) == Some(key.as_str()) {
                if let Ok(picture) = image::open(&png) {
                    self.finish(asset.clone(), key, Ok(picture.to_rgba8()));
                    continue;
                }
            }
            self.slots.insert(asset.clone(), Slot::Waiting { key: key.clone() });
            match entry.meta.kind {
                AssetKind::Texture(_) => {
                    let _ = self.jobs.send(Job { asset: asset.clone(), source: entry.source_path(), png, key });
                }
                _ => {
                    let mut queue = self.queue.borrow_mut();
                    queue.retain(|d| &d.asset != asset);
                    queue.push_back(Draw { asset: asset.clone(), key });
                }
            }
        }
    }

    fn finish(&mut self, asset: AssetRef, key: String, picture: Result<image::RgbaImage, String>) {
        // A newer key was asked for meanwhile: this one is stale.
        if self.slots.get(&asset).is_some_and(|slot| slot.key() != key && !matches!(slot, Slot::Ready { .. })) {
            return;
        }
        let slot = match picture {
            Ok(picture) => Slot::Ready {
                key,
                image: Image::from_rgba(Position::default(), Size::new(SIZE as f32, SIZE as f32), &picture),
            },
            Err(e) => {
                log::warn!("thumbnail of {asset:?}: {e}");
                Slot::Failed { key }
            }
        };
        self.slots.insert(asset, slot);
    }

    /// Whether a picture is there to draw for this asset.
    pub fn ready(&self, asset: &AssetRef) -> bool {
        matches!(self.slots.get(asset), Some(Slot::Ready { .. }))
    }

    /// Where the picture of `asset` goes next time it is drawn.
    pub fn place(&mut self, asset: &AssetRef, x: f32, y: f32, size: f32) {
        if let Some(Slot::Ready { image, .. }) = self.slots.get_mut(asset) {
            image.set_offset(ferrite::core::primitives::Offset { x, y });
            image.set_size(Size::new(size, size));
        }
    }

    /// Draw the picture of `asset` where it was placed.
    pub fn draw(&self, asset: &AssetRef) {
        if let Some(Slot::Ready { image, .. }) = self.slots.get(asset) {
            image.render();
        }
    }

    /// Draw one queued material or model picture. Call while the frame is
    /// being drawn.
    pub fn render_pending(&self) {
        if !gfx::is_drawing() || Instant::now() < self.rest_until.get() {
            return;
        }
        let Some(draw) = self.queue.borrow_mut().pop_front() else {
            return;
        };
        let started = Instant::now();
        let picture = self.render_one(&draw.asset);
        self.rest_until.set(Instant::now() + started.elapsed());
        self.drawn.borrow_mut().push((draw.asset, draw.key, picture));
    }

    fn render_one(&self, asset: &AssetRef) -> Result<image::RgbaImage, String> {
        let (kind, name) = {
            let database = assets::database();
            let entry = database.get(asset.id).ok_or("the asset is gone")?;
            (entry.meta.kind.clone(), entry.meta.name.clone())
        };
        match kind {
            AssetKind::Material(_) => {
                // Maps left on "auto" come from the model file of a slot
                // using the material.
                let auto = self.auto_material(asset.id);
                let material =
                    material_asset::material_for_asset(asset.id, auto.as_ref()).ok_or("the material could not be built")?;
                self.picture_of_sphere(material)
            }
            AssetKind::MaterialGraph(_) => {
                let file = MaterialFile {
                    graph: GraphRef::Asset(asset.id),
                    alpha: AlphaMode::Opaque,
                    ..Default::default()
                };
                self.picture_of_sphere(material_asset::build(&file, &name, None))
            }
            AssetKind::Model(model) => {
                // Through the model cache: the scene (a scatter graph
                // planting the variant) has it loaded mostly already.
                let variant = asset.part.clone().or_else(|| model.variants.first().map(|v| v.name.clone()));
                let reference = match variant {
                    Some(variant) => AssetRef::part(asset.id, &variant),
                    None => AssetRef::whole(asset.id),
                };
                let (loaded, _) = ScatterModel::load_asset(&reference)?;
                let (min, max) = loaded.bounds(0).unwrap_or(([0.0; 3], [1.0; 3]));
                let center = Point3::new((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5);
                let radius = (0..3).map(|a| (max[a] - min[a]) * 0.5).map(|h| h * h).sum::<f32>().sqrt().max(0.01);
                let instances = ScatterInstances::new(&ScatterSpec::of_model(loaded), &[Placement::upright([0.0; 3], 0.0)]);
                Ok(picture(center, radius, |vp, lighting| {
                    instances.draw(vp, &Matrix4::identity(), lighting, LodChoice::Distance(0.0));
                }))
            }
            _ => Err("no picture for this kind".into()),
        }
    }

    /// What the loader makes of the file material of the first model slot
    /// drawn with material `id`, if any slot is.
    fn auto_material(&self, id: AssetId) -> Option<ferrite::core::material::Material> {
        let (model, index, path, scale) = {
            let database = assets::database();
            let found = database.iter().find_map(|entry| {
                let AssetKind::Model(settings) = &entry.meta.kind else { return None };
                let index = settings.materials.iter().position(|slot| slot.material == Some(id))?;
                Some((entry.meta.id, index, entry.source_path(), settings.scale))
            });
            found
        }?;
        let mut cache = self.file_materials.borrow_mut();
        if !cache.iter().any(|(m, _)| *m == model) {
            let path = path.to_string_lossy().replace('\\', "/");
            let mut loaded = ModelBuilder::from_file(&path).ok()?.with_scale(scale).build();
            loaded.init();
            let count = loaded.materials().len();
            let materials = (0..count).map(|i| loaded.file_material(i).cloned()).collect();
            if cache.len() >= 4 {
                cache.pop_front();
            }
            cache.push_back((model, materials));
        }
        cache.iter().find(|(m, _)| *m == model).and_then(|(_, materials)| materials.get(index).cloned().flatten())
    }

    fn picture_of_sphere(&self, material: ferrite::core::material::Material) -> Result<image::RgbaImage, String> {
        let mut sphere = self.sphere.borrow_mut();
        if sphere.is_none() {
            *sphere = Some(Shape::Sphere.model()?);
        }
        let model = sphere.as_mut().expect("just made");
        let mut material = material;
        // Still: no wind in a picture.
        material.set_float("wind", 0.0);
        material.set_float("wind_flutter", 0.0);
        for index in 0..model.materials().len() {
            model.set_material(index, material.clone());
        }
        let (c, radius) = Shape::Sphere.framing();
        Ok(picture(Point3::new(c[0], c[1], c[2]), radius * 0.8, |vp, lighting| {
            model.render(lighting, &Matrix4::identity(), vp);
        }))
    }
}

/// Draw something `radius` around `center` into a picture of its own, from
/// the front and a bit above, and read it back (top row first). The pass
/// that was running goes on afterwards as it was.
fn picture(center: Point3<f32>, radius: f32, draw: impl FnOnce(&Matrix4<f32>, &Lighting)) -> image::RgbaImage {
    let color = gfx::Texture::new(TextureDesc::d2(Format::Rgba8, SIZE, SIZE));
    let depth = gfx::Texture::new(TextureDesc::d2(Format::Depth24, SIZE, SIZE));
    let target = gfx::RenderTarget::new(&[color.attachment(0, None)], Some(depth.attachment(0, None)));

    let overrides = gfx::pass_overrides();
    let viewport = gfx::viewport();
    gfx::set_pass_overrides(gfx::PassOverrides::default());
    gfx::begin_pass(target.reference(), (SIZE, SIZE), Load::Color([0.0; 4]), Load::Depth(1.0));
    // Display colour, as a preview in the window, not a shadow map.
    hdr::set_picture_target(Some(target.reference()));
    gfx::set_viewport(gfx::Rect::sized(SIZE, SIZE));
    let (yaw, pitch) = (35f32.to_radians(), 20f32.to_radians());
    let distance = radius / (20f32.to_radians()).sin();
    let eye = center + Vector3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * distance;
    let projection = cgmath::perspective(cgmath::Deg(40.0), 1.0, (distance - radius * 2.0).max(0.01), distance + radius * 2.0);
    let vp = projection * Matrix4::look_at_rh(eye, center, Vector3::unit_y());
    let lighting = Lighting::preview(Vector3::new(3.0, 5.0, 4.0), eye);
    draw(&vp, &lighting);
    gfx::end_pass();
    hdr::set_picture_target(None);
    let pixels = gfx::read_pixels(target.reference(), gfx::Rect::sized(SIZE, SIZE));
    gfx::set_pass_overrides(overrides);
    gfx::set_viewport(viewport);
    let image = image::RgbaImage::from_raw(SIZE, SIZE, pixels).unwrap_or_else(|| image::RgbaImage::new(SIZE, SIZE));
    // Rows start at the bottom.
    image::imageops::flip_vertical(&image)
}

/// A texture file shrunk to fit `SIZE` (aspect kept, centred, the rest
/// clear).
fn shrink(source: &Path) -> Result<image::RgbaImage, String> {
    let image = image::open(source).map_err(|e| e.to_string())?;
    let small = image.thumbnail(SIZE, SIZE).to_rgba8();
    let mut square = image::RgbaImage::new(SIZE, SIZE);
    let x = (SIZE - small.width()) / 2;
    let y = (SIZE - small.height()) / 2;
    image::imageops::overlay(&mut square, &small, x as i64, y as i64);
    Ok(square)
}

/// Keep a picture and its key on disk; a failure only costs a redraw.
fn save(png: &Path, key: &str, picture: &image::RgbaImage) {
    if let Some(folder) = png.parent() {
        let _ = fs::create_dir_all(folder);
    }
    if picture.save(png).is_ok() {
        let _ = fs::write(png.with_extension("key"), key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_change_with_the_file_and_its_dependencies() {
        let project = std::env::temp_dir().join(format!("ferrite-thumbs-{}-{}", std::process::id(), AssetId::new()));
        let root = project.join("assets");
        fs::create_dir_all(&root).unwrap();
        let mut database = AssetDatabase::new(&root);
        fs::write(root.join("stone.png"), b"one").unwrap();
        let texture = database
            .register(
                assets::AssetMeta {
                    version: assets::ASSET_META_VERSION,
                    id: AssetId::new(),
                    name: "stone".into(),
                    source: "stone.png".into(),
                    kind: AssetKind::Texture(Default::default()),
                },
                &root,
            )
            .unwrap();
        let mut file = MaterialFile::default();
        file.params.insert("base_color".into(), ParamSetting::Texture(TextureSetting::Asset(texture)));
        file.save(&root.join("Stone.material.ron")).unwrap();
        let material = database.import(&root.join("Stone.material.ron")).unwrap();

        let whole = AssetRef::whole(material);
        let first = key_of(&database, &whole);
        assert_eq!(first, key_of(&database, &whole), "keys are stable");
        // The texture the material uses changes: so does the key.
        fs::write(root.join("stone.png"), b"longer now").unwrap();
        let second = key_of(&database, &whole);
        assert_ne!(first, second);
        // A part has a key and a file of its own.
        let part = AssetRef::part(material, "x");
        assert_ne!(key_of(&database, &part), second);
        assert_ne!(file_stem(&part), file_stem(&whole));
        let _ = fs::remove_dir_all(&project);
    }
}
