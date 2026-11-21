//! The pictures on the browser's cards: textures shrunk, materials on a
//! sphere, models from the front. Each is made once and kept as a PNG in
//! `.ferrite-cache/thumbnails/` with a key file beside it; it is made
//! again only when the key (the asset's settings, its file's size and time,
//! and those of the assets it uses) no longer fits.
//!
//! Textures are shrunk on a worker thread. For materials and models a
//! second worker reads the files and decodes their maps (`PreparedModel`);
//! the render thread then only uploads and draws, within a few
//! milliseconds per frame, and takes the pixels back from the GPU some
//! frames later (`gfx::read_pixels_later`): it never waits.

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
        model::{Model, ModelBuilder, PreparedModel},
        primitives::{Position, Size},
        renderer::{
            hdr,
            pbr::Lighting,
            texture::decode::{self, Decoded},
        },
    },
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

/// Work for the worker threads.
enum Job {
    /// Shrink a texture file (and keep the picture).
    Texture { asset: AssetRef, source: PathBuf, png: PathBuf, key: String },
    /// Read and decode what a material or model picture needs.
    Prepare { asset: AssetRef, key: String, what: Prepare },
    /// Keep a picture drawn on the render thread.
    Save { png: PathBuf, key: String, picture: image::RgbaImage },
}

/// What to read for a picture drawn on the render thread.
enum Prepare {
    /// A model file: the variant to show, the material assets of its slots.
    Model { source: String, variant: Option<String>, slots: Vec<assets::MaterialSettings> },
    /// A material: its own texture assets, and the model file whose slot
    /// `slot` gives its "auto" maps, if a model uses it (`cached`: that
    /// model's materials are at hand already, nothing to read).
    Material { id: AssetId, auto: Option<(AssetId, String, usize)>, cached: Option<(AssetId, usize)> },
    /// Nothing to read (a material graph).
    Nothing,
}

/// What a worker read for a picture: ready to upload.
enum Ready {
    Model { model: PreparedModel, variant: Option<String>, slots: Vec<assets::MaterialSettings> },
    Material { textures: HashMap<PathBuf, Decoded>, auto: Option<(AssetId, PreparedModel, usize)>, auto_slot: Option<(AssetId, usize)> },
    Graph,
}

/// What a worker sends back.
enum Done {
    /// A texture's picture.
    Picture { asset: AssetRef, key: String, picture: Result<image::RgbaImage, String> },
    /// Read for drawing.
    Prepared { asset: AssetRef, key: String, ready: Result<Ready, String> },
}

/// Main-thread time per frame for pictures: uploading and drawing go on
/// while there is some left (one model is never split).
const BUDGET_MS: f32 = 4.0;

pub struct Thumbnails {
    folder: PathBuf,
    slots: HashMap<AssetRef, Slot>,
    /// Texture pictures and saving.
    textures: Sender<Job>,
    /// Reading models and materials.
    prepare: Sender<Job>,
    done: Receiver<Done>,
    /// Read, waiting for the render thread (`render_pending`).
    ready: RefCell<VecDeque<(AssetRef, String, Ready)>>,
    /// Drawn, the pixels on their way back from the GPU.
    reading: RefCell<Vec<(AssetRef, String, u64)>>,
    /// Back from the GPU or failed, for `want` to take.
    drawn: RefCell<Vec<(AssetRef, String, Result<image::RgbaImage, String>)>>,
    sphere: RefCell<Option<Model>>,
    /// No picture is drawn before this: after a step that took longer than
    /// the budget, the editor gets as long to itself.
    rest_until: std::cell::Cell<Instant>,
    /// Recently read model files (by model asset), for materials whose maps
    /// are "auto": the model is kept, it holds the textures.
    file_materials: RefCell<VecDeque<(AssetId, Model)>>,
    /// Database state and time of the last key check.
    checked: (u64, Instant),
}

fn worker(name: &str, inbox: Receiver<Job>, outbox: Sender<Done>) {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok(job) = inbox.recv() {
                let done = match job {
                    Job::Texture { asset, source, png, key } => {
                        let picture = shrink(&source);
                        if let Ok(picture) = &picture {
                            save(&png, &key, picture);
                        }
                        Done::Picture { asset, key, picture }
                    }
                    Job::Prepare { asset, key, what } => Done::Prepared { asset, key, ready: prepare(what) },
                    Job::Save { png, key, picture } => {
                        save(&png, &key, &picture);
                        continue;
                    }
                };
                if outbox.send(done).is_err() {
                    break;
                }
            }
        })
        .expect("a thumbnail thread starts");
}

/// Read and decode on a worker what a picture needs (no GPU).
fn prepare(what: Prepare) -> Result<Ready, String> {
    match what {
        Prepare::Model { source, variant, slots } => {
            let materials: Vec<AssetId> = slots.iter().filter_map(|s| s.material).collect();
            let model = PreparedModel::load(&source)?.with_material_textures(&materials);
            Ok(Ready::Model { model, variant, slots })
        }
        Prepare::Material { id, auto, cached } => {
            let textures = decode::decode_files(&material_asset::texture_files_of(&[id]));
            // Not read: drawn without the model's maps.
            let auto = auto.and_then(|(model, source, slot)| PreparedModel::load(&source).ok().map(|p| (model, p, slot)));
            Ok(Ready::Material { textures, auto, auto_slot: cached })
        }
        Prepare::Nothing => Ok(Ready::Graph),
    }
}

impl Thumbnails {
    pub fn new(project: &Path) -> Self {
        let (textures, texture_inbox) = channel::<Job>();
        let (prepare, prepare_inbox) = channel::<Job>();
        let (outbox, done) = channel::<Done>();
        worker("thumbnails", texture_inbox, outbox.clone());
        worker("thumbnail-models", prepare_inbox, outbox);
        Self {
            folder: project.join(FOLDER),
            slots: HashMap::new(),
            textures,
            prepare,
            done,
            ready: RefCell::new(VecDeque::new()),
            reading: RefCell::new(Vec::new()),
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
        self.ready.borrow_mut().clear();
        self.reading.borrow_mut().clear();
    }

    fn png_of(&self, asset: &AssetRef) -> PathBuf {
        self.folder.join(format!("{}.png", file_stem(asset)))
    }

    /// Ask for the pictures of the cards on screen; call every update.
    /// Takes what the workers and the render thread made, and starts what
    /// is missing or out of date.
    pub fn want(&mut self, database: &AssetDatabase, visible: &[AssetRef]) {
        while let Ok(done) = self.done.try_recv() {
            match done {
                Done::Picture { asset, key, picture } => self.finish(asset, key, picture),
                Done::Prepared { asset, key, ready } => match ready {
                    Ok(ready) => self.ready.borrow_mut().push_back((asset, key, ready)),
                    Err(e) => self.finish(asset, key, Err(e)),
                },
            }
        }
        let drawn = std::mem::take(&mut *self.drawn.borrow_mut());
        for (asset, key, picture) in drawn {
            if let Ok(picture) = &picture {
                let job = Job::Save { png: self.png_of(&asset), key: key.clone(), picture: picture.clone() };
                let _ = self.textures.send(job);
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
            let source = entry.source_path();
            let job = match &entry.meta.kind {
                AssetKind::Texture(_) => {
                    let _ = self.textures.send(Job::Texture { asset: asset.clone(), source, png, key });
                    continue;
                }
                AssetKind::Model(model) => Prepare::Model {
                    source: source.to_string_lossy().replace('\\', "/"),
                    variant: asset.part.clone().or_else(|| model.variants.first().map(|v| v.name.clone())),
                    slots: model.materials.clone(),
                },
                AssetKind::Material(_) => {
                    let auto = auto_source(database, asset.id);
                    let cached = auto
                        .as_ref()
                        .filter(|(model, _, _)| self.file_materials.borrow().iter().any(|(m, _)| m == model))
                        .map(|(model, _, slot)| (*model, *slot));
                    match cached {
                        Some(cached) => Prepare::Material { id: asset.id, auto: None, cached: Some(cached) },
                        None => Prepare::Material { id: asset.id, auto, cached: None },
                    }
                }
                _ => Prepare::Nothing,
            };
            let _ = self.prepare.send(Job::Prepare { asset: asset.clone(), key, what: job });
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

    /// The render thread's part, within a few milliseconds per frame: take
    /// the pixels the GPU finished, then upload and draw what the workers
    /// read. Call while the frame is being drawn.
    pub fn render_pending(&self) {
        if !gfx::is_drawing() {
            return;
        }
        // Pixels back from the GPU (frames after they were drawn).
        self.reading.borrow_mut().retain(|(asset, key, id)| match gfx::take_pixels(*id) {
            Some(pixels) => {
                let picture = image::RgbaImage::from_raw(SIZE, SIZE, pixels)
                    .map(|image| image::imageops::flip_vertical(&image))
                    .ok_or_else(|| "the picture came back in pieces".to_string());
                self.drawn.borrow_mut().push((asset.clone(), key.clone(), picture));
                false
            }
            None => true,
        });
        let started = Instant::now();
        if started < self.rest_until.get() {
            return;
        }
        while started.elapsed().as_secs_f32() * 1000.0 < BUDGET_MS {
            let Some((asset, key, ready)) = self.ready.borrow_mut().pop_front() else {
                break;
            };
            match self.draw_ready(&asset, ready) {
                Ok(Some(id)) => self.reading.borrow_mut().push((asset, key, id)),
                // The model it counted on is gone from the cache: read again.
                Ok(None) => {
                    let auto = auto_source(&assets::database(), asset.id);
                    let what = Prepare::Material { id: asset.id, auto, cached: None };
                    let _ = self.prepare.send(Job::Prepare { asset, key, what });
                }
                Err(e) => self.drawn.borrow_mut().push((asset, key, Err(e))),
            }
        }
        // Over budget (a big model): as long again to the editor.
        let spent = started.elapsed();
        if spent.as_secs_f32() * 1000.0 > BUDGET_MS {
            self.rest_until.set(Instant::now() + spent);
        }
    }

    /// Upload what a worker read and draw it: the id of the pixels' way
    /// back, or `None` when it has to be read again.
    fn draw_ready(&self, asset: &AssetRef, ready: Ready) -> Result<Option<u64>, String> {
        match ready {
            Ready::Model { model, variant, slots } => {
                let mut model = ModelBuilder::from_prepared(model).build();
                model.init();
                material_asset::apply_to_model(&mut model, &slots);
                model.show_variant(variant.as_deref())?;
                let (min, max) = model.visible_bounds().ok_or("the model has no vertices")?;
                let center = Point3::new((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5);
                let radius = (0..3).map(|a| (max[a] - min[a]) * 0.5).map(|h| h * h).sum::<f32>().sqrt().max(0.01);
                Ok(Some(picture(center, radius, |vp, lighting| model.render(lighting, &Matrix4::identity(), vp))))
            }
            Ready::Material { textures, auto, auto_slot } => {
                material_asset::offer_textures(textures);
                let auto = match (auto, auto_slot) {
                    (Some((model, prepared, slot)), _) => self.file_material(model, Some(prepared), slot),
                    (None, Some((model, slot))) => {
                        if !self.file_materials.borrow().iter().any(|(m, _)| *m == model) {
                            return Ok(None);
                        }
                        self.file_material(model, None, slot)
                    }
                    _ => None,
                };
                let material = material_asset::material_for_asset(asset.id, auto.as_ref());
                material_asset::forget_offered_textures();
                let material = material.ok_or("the material could not be built")?;
                self.picture_of_sphere(material).map(Some)
            }
            Ready::Graph => {
                let name = assets::database().get(asset.id).map(|e| e.meta.name.clone()).unwrap_or_default();
                let file = MaterialFile { graph: GraphRef::Asset(asset.id), alpha: AlphaMode::Opaque, ..Default::default() };
                self.picture_of_sphere(material_asset::build(&file, &name, None)).map(Some)
            }
        }
    }

    /// What the loader made of material `slot` of model asset `model`:
    /// from the recently read ones, or uploaded from `prepared`.
    fn file_material(
        &self,
        model: AssetId,
        prepared: Option<PreparedModel>,
        slot: usize,
    ) -> Option<ferrite::core::material::Material> {
        let mut cache = self.file_materials.borrow_mut();
        if !cache.iter().any(|(m, _)| *m == model) {
            let mut loaded = ModelBuilder::from_prepared(prepared?).build();
            loaded.init();
            if cache.len() >= 4 {
                cache.pop_front();
            }
            cache.push_back((model, loaded));
        }
        cache.iter().find(|(m, _)| *m == model).and_then(|(_, loaded)| loaded.file_material(slot).cloned())
    }

    fn picture_of_sphere(&self, material: ferrite::core::material::Material) -> Result<u64, String> {
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

/// The model file whose slot gives material `id` its "auto" maps: the
/// first model with a slot drawn with it (asset, file, slot).
fn auto_source(database: &AssetDatabase, id: AssetId) -> Option<(AssetId, String, usize)> {
    database.iter().find_map(|entry| {
        let AssetKind::Model(settings) = &entry.meta.kind else { return None };
        let slot = settings.materials.iter().position(|slot| slot.material == Some(id))?;
        Some((entry.meta.id, entry.source_path().to_string_lossy().replace('\\', "/"), slot))
    })
}

/// Draw something `radius` around `center` into a picture of its own, from
/// the front and a bit above. The pixels come back later: the id is for
/// `gfx::take_pixels`. The pass that was running goes on afterwards as it
/// was.
fn picture(center: Point3<f32>, radius: f32, draw: impl FnOnce(&Matrix4<f32>, &Lighting)) -> u64 {
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
    // The copy goes into this frame's commands; the target and textures are
    // freed only once those have run.
    let id = gfx::read_pixels_later(target.reference(), gfx::Rect::sized(SIZE, SIZE));
    gfx::set_pass_overrides(overrides);
    gfx::set_viewport(viewport);
    id
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
