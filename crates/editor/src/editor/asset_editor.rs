//! The asset workspace: settings of one asset in the left column and a
//! preview of it where the scene viewport normally is. Edits are made on a
//! working copy and reach the asset (and whatever uses it) when saved; the
//! preview always shows the working copy.

use std::{path::PathBuf, time::Instant};

use cgmath::{perspective, Deg, Matrix4, Point3, SquareMatrix, Vector3};
use ferrite::{
    core::{
        assets::{self, AssetId, AssetKind, AssetMeta, MaterialSettings},
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            text::{icons, Text},
            texture::{Texture, TextureRenderer},
        },
        scene::Scene,
        utils::DataSource,
    },
    terrain::scatter::{LodChoice, ScatterInstances, ScatterModel, ScatterSpec},
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
        section::Section,
    },
    theme, UIElement, UI,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{
    drag::{DropAction, DropTarget},
    field_row::{FieldRow, LABEL_COLUMN},
    layout::Rect,
    send, Command, Shared,
};

/// A texture a material can be given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureSlot {
    BaseColor,
    Normal,
}

impl TextureSlot {
    pub const ALL: [TextureSlot; 2] = [TextureSlot::BaseColor, TextureSlot::Normal];

    pub fn label(self) -> &'static str {
        match self {
            TextureSlot::BaseColor => "base color",
            TextureSlot::Normal => "normal",
        }
    }

    pub fn get(self, material: &MaterialSettings) -> Option<AssetId> {
        match self {
            TextureSlot::BaseColor => material.base_color,
            TextureSlot::Normal => material.normal,
        }
    }

    pub fn set(self, material: &mut MaterialSettings, texture: Option<AssetId>) {
        match self {
            TextureSlot::BaseColor => material.base_color = texture,
            TextureSlot::Normal => material.normal = texture,
        }
    }
}

/// The asset open in the asset workspace.
pub struct AssetEdit {
    /// Working copy of the asset's sidecar.
    pub meta: AssetMeta,
    /// Folder of the asset's source file.
    pub dir: PathBuf,
    /// Whether the working copy differs from what is saved.
    pub dirty: bool,
    /// Variant shown in the preview (index into the model's variants).
    pub variant: usize,
    /// LOD shown in the preview; `None` picks it by camera distance.
    pub lod: Option<usize>,
    /// Goes up when the settings panel has to be rebuilt.
    pub structure: u64,
    /// Goes up when the preview has to load the model again.
    pub preview: u64,
}

impl AssetEdit {
    /// Start editing an asset, showing the named variant if given.
    pub fn open(id: AssetId, variant: Option<&str>) -> Result<Self, String> {
        let database = assets::database();
        let entry = database
            .get(id)
            .ok_or_else(|| format!("asset {id} does not exist"))?;
        let variant = variant
            .and_then(|name| {
                entry
                    .meta
                    .as_model()?
                    .variants
                    .iter()
                    .position(|v| v.name == name)
            })
            .unwrap_or(0);
        Ok(Self {
            meta: entry.meta.clone(),
            dir: entry.dir.clone(),
            dirty: false,
            variant,
            lod: None,
            structure: 0,
            preview: 0,
        })
    }

    pub fn source_path(&self) -> PathBuf {
        self.dir.join(&self.meta.source)
    }

    /// Give a material a texture (or none, for the automatic one).
    pub fn set_texture(&mut self, material: usize, slot: TextureSlot, texture: Option<AssetId>) {
        let Some(material) = self
            .meta
            .as_model_mut()
            .and_then(|model| model.materials.get_mut(material))
        else {
            return;
        };
        if slot.get(material) != texture {
            slot.set(material, texture);
            self.dirty = true;
            self.structure += 1;
            self.preview += 1;
        }
    }

    /// Write the working copy to the asset.
    pub fn save(&mut self) -> Result<(), String> {
        assets::database()
            .update(self.meta.clone())
            .map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// Throw the edits away and go back to what is saved.
    pub fn revert(&mut self) -> Result<(), String> {
        let saved = assets::database()
            .get(self.meta.id)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| format!("asset {} does not exist any more", self.meta.id))?;
        self.meta = saved;
        self.dirty = false;
        self.structure += 1;
        self.preview += 1;
        Ok(())
    }
}

fn header_button(
    label: &str,
    icon: &str,
    style: ButtonStyle,
    on_click: Box<dyn Fn(&mut Scene)>,
) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), on_click);
    button.set_style(style);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(26.0);
    button
}

/// Name of a texture asset for a slot row.
fn texture_name(texture: Option<AssetId>) -> String {
    match texture {
        None => "automatic".into(),
        Some(id) => assets::database()
            .get(id)
            .map(|entry| entry.meta.name.clone())
            .unwrap_or_else(|| format!("missing ({id})")),
    }
}

/// Widgets bound to the working copy, compared against it every frame.
struct Bindings {
    name: DataSource<String>,
    scale: DataSource<f32>,
    variant: DataSource<u32>,
    /// 0 is "automatic", then the LODs.
    lod: DataSource<u32>,
    foliage: Vec<DataSource<bool>>,
}

/// Left column of the asset workspace: the asset's settings.
pub struct AssetInspector {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    save: Button,
    revert: Button,
    back: Button,
    bindings: Option<Bindings>,
    /// Rows a texture asset can be dropped on.
    slot_rows: Vec<(UIElementHandle, usize, TextureSlot)>,
    /// `(asset, structure counter, width)` the panel was built for.
    built_for: Option<(AssetId, u64, i32)>,
}

const ACTIONS_HEIGHT: f32 = 34.0;

impl AssetInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Asset".into(), Position::default(), Size::new(330.0, 600.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 500.0));
        body.set_gap(4.0);

        let save_state = state.clone();
        let mut save = header_button(
            "Save",
            icons::SAVE,
            ButtonStyle::Primary,
            Box::new(move |_| send(&save_state, Command::SaveAsset)),
        );
        save.set_tooltip("Save the asset (Ctrl+S)");
        let revert_state = state.clone();
        let mut revert = header_button(
            "Revert",
            icons::ROTATE_CW,
            ButtonStyle::Secondary,
            Box::new(move |_| send(&revert_state, Command::RevertAsset)),
        );
        revert.set_tooltip("Discard the unsaved changes");
        let back_state = state.clone();
        let back = header_button(
            "Back to scene",
            icons::ARROW_LEFT,
            ButtonStyle::Ghost,
            Box::new(move |_| send(&back_state, Command::CloseAssetEditor)),
        );

        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            save,
            revert,
            back,
            bindings: None,
            slot_rows: Vec::new(),
            built_for: None,
        }
    }

    /// The whole left column.
    fn rect(&self) -> Rect {
        let layout = self.state.borrow().layout;
        Rect {
            x: layout.hierarchy.x,
            y: layout.hierarchy.y,
            w: layout.hierarchy.w,
            h: layout.hierarchy.h + layout.inspector.h,
        }
    }

    fn rebuild(&mut self, width: f32) {
        self.body.clear();
        self.slot_rows.clear();
        self.bindings = None;
        let state = self.state.borrow();
        let Some(edit) = &state.asset_edit else {
            return;
        };
        let value_w = width - LABEL_COLUMN - 16.0;
        let muted = |text: String| -> Box<dyn UIElement> {
            Box::new(Label::muted(text, theme::FONT_SIZE_SMALL + 0.5))
        };

        // What it is.
        let name = DataSource::new(edit.meta.name.clone());
        let mut general = Section::new("Asset", Some(icons::PACKAGE), width);
        general.add_child(Box::new(FieldRow::new(
            "Name",
            vec![UI::input_sized(name.clone(), Size::new(value_w, 24.0))],
        )));
        general.add_child(Box::new(FieldRow::new(
            "Kind",
            vec![muted(edit.meta.kind.label().to_string())],
        )));
        general.add_child(Box::new(FieldRow::new(
            "File",
            vec![muted(edit.source_path().to_string_lossy().replace('\\', "/"))],
        )));
        self.body.add_child(Box::new(general));

        let mut bindings = Bindings {
            name,
            scale: DataSource::new(1.0),
            variant: DataSource::new(edit.variant as u32),
            lod: DataSource::new(edit.lod.map_or(0, |lod| lod as u32 + 1)),
            foliage: Vec::new(),
        };

        if let AssetKind::Model(model) = &edit.meta.kind {
            bindings.scale.write(model.scale);
            let mut settings = Section::new("Model", Some(icons::BOX), width);
            settings.add_child(Box::new(FieldRow::new(
                "Scale",
                vec![UI::input_sized(
                    bindings.scale.clone(),
                    Size::new(value_w.min(140.0), 24.0),
                )],
            )));
            self.body.add_child(Box::new(settings));

            // What the preview shows.
            let mut preview = Section::new("Preview", Some(icons::EYE), width);
            let variants: Vec<String> = model.variants.iter().map(|v| v.name.clone()).collect();
            let mut lods = vec!["automatic".to_string()];
            if let Some(variant) = model.variants.get(edit.variant) {
                lods.extend(variant.lods.iter().cloned());
            }
            preview.add_child(Box::new(FieldRow::new(
                "Variant",
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    variants,
                    bindings.variant.clone(),
                ))],
            )));
            preview.add_child(Box::new(FieldRow::new(
                "LOD",
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    lods,
                    bindings.lod.clone(),
                ))],
            )));
            self.body.add_child(Box::new(preview));

            // One section per material.
            for (index, material) in model.materials.iter().enumerate() {
                let title = if material.name.is_empty() {
                    format!("Material {}", index + 1)
                } else {
                    format!("Material: {}", material.name)
                };
                let mut section = Section::new(&title, Some(icons::LAYERS), width);
                for slot in TextureSlot::ALL {
                    let pick_state = self.state.clone();
                    let mut pick = Button::new(
                        Position::default(),
                        Size::new(24.0, 24.0),
                        Box::new(move |_| {
                            send(
                                &pick_state,
                                Command::PickAssetTexture {
                                    material: index,
                                    slot,
                                },
                            )
                        }),
                    );
                    pick.set_style(ButtonStyle::Secondary);
                    pick.set_icon(icons::LAYOUT_GRID);
                    pick.set_tooltip("Choose a texture asset");
                    let clear_state = self.state.clone();
                    let mut clear = Button::new(
                        Position::default(),
                        Size::new(24.0, 24.0),
                        Box::new(move |_| {
                            send(
                                &clear_state,
                                Command::SetAssetTexture {
                                    material: index,
                                    slot,
                                    texture: None,
                                },
                            )
                        }),
                    );
                    clear.set_style(ButtonStyle::Danger);
                    clear.set_icon(icons::X);
                    clear.set_tooltip("Back to the automatic texture");
                    let row = Box::new(FieldRow::new(
                        slot.label(),
                        vec![
                            Box::new(pick),
                            Box::new(clear),
                            muted(texture_name(slot.get(material))),
                        ],
                    ));
                    self.slot_rows.push((*row.get_handle(), index, slot));
                    section.add_child(row);
                }
                let foliage = DataSource::new(material.foliage);
                section.add_child(Box::new(FieldRow::new(
                    "foliage",
                    vec![
                        UI::checkbox(foliage.clone()),
                        muted("no collision".to_string()),
                    ],
                )));
                bindings.foliage.push(foliage);
                self.body.add_child(Box::new(section));
            }
        }
        self.bindings = Some(bindings);
    }

    /// Carry widget edits over to the working copy.
    fn sync(&mut self) {
        let Some(bindings) = &self.bindings else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else {
            return;
        };
        let name = bindings.name.read();
        if name != edit.meta.name {
            edit.meta.name = name;
            edit.dirty = true;
        }
        let variant = bindings.variant.read() as usize;
        if variant != edit.variant {
            edit.variant = variant;
            // The LOD list belongs to the variant.
            edit.lod = None;
            edit.structure += 1;
            edit.preview += 1;
        }
        let lod = bindings.lod.read().checked_sub(1).map(|lod| lod as usize);
        if lod != edit.lod {
            edit.lod = lod;
        }
        let mut reload = false;
        let mut dirty = false;
        if let Some(model) = edit.meta.as_model_mut() {
            let scale = bindings.scale.read();
            // A half-typed number must not shrink the model to nothing.
            if scale > 0.0 && (scale - model.scale).abs() > 1e-6 {
                model.scale = scale;
                dirty = true;
                reload = true;
            }
            for (material, foliage) in model.materials.iter_mut().zip(&bindings.foliage) {
                if material.foliage != foliage.read() {
                    material.foliage = foliage.read();
                    dirty = true;
                }
            }
        }
        if dirty {
            edit.dirty = true;
        }
        if reload {
            edit.preview += 1;
        }
    }

    /// Texture slots take texture assets dragged out of the browser.
    fn register_drop_targets(&self) {
        let panel = self.rect();
        let mut targets = Vec::new();
        for (handle, material, slot) in &self.slot_rows {
            let Some(row) = self.body.get_child(handle) else {
                continue;
            };
            let rect = Rect {
                x: panel.x + 4.0,
                y: row.get_offset().y - 1.0,
                w: (panel.w - 8.0).max(1.0),
                h: row.get_size().height + 2.0,
            };
            if rect.y < panel.y || rect.y + rect.h > panel.y + panel.h {
                continue;
            }
            targets.push(DropTarget {
                rect,
                action: DropAction::MaterialTexture {
                    material: *material,
                    slot: *slot,
                },
            });
        }
        self.state.borrow_mut().drop_targets.extend(targets);
    }

    fn buttons_mut(&mut self) -> [&mut Button; 3] {
        [&mut self.save, &mut self.revert, &mut self.back]
    }
}

impl UIElement for AssetInspector {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let (key, dirty) = {
            let state = self.state.borrow();
            match &state.asset_edit {
                Some(edit) => (
                    Some((edit.meta.id, edit.structure, rect.w as i32)),
                    edit.dirty,
                ),
                None => (None, false),
            }
        };

        let header = self.panel.header_height();
        let ay = rect.y + header + 4.0;
        let mut x = rect.x + 6.0;
        self.save.set_enabled(dirty);
        self.revert.set_enabled(dirty);
        for button in [&mut self.save, &mut self.revert] {
            button.set_offset(Offset { x, y: ay });
            x += button.get_size().width + 4.0;
        }
        let back_x = (rect.x + rect.w - 6.0 - self.back.get_size().width).max(x);
        self.back.set_offset(Offset { x: back_x, y: ay });

        let body_top = rect.y + header + ACTIONS_HEIGHT + 4.0;
        self.body
            .set_position(Position::new(rect.x + 4.0, body_top, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.y + rect.h - body_top - 4.0).max(10.0),
        ));

        if key != self.built_for {
            self.rebuild((rect.w - 24.0).max(200.0));
            self.body.set_z_index(2.0);
            self.built_for = key;
        }
        self.sync();

        self.panel.update(scene);
        for button in self.buttons_mut() {
            button.update(scene);
        }
        self.body.update(scene);
        self.register_drop_targets();
    }

    fn render(&self) {
        self.panel.render();
        self.body.render();
        self.save.render();
        self.revert.render();
        self.back.render();
    }

    fn render_overlay(&self) {
        self.body.render_overlay();
        self.save.render_overlay();
        self.revert.render_overlay();
        self.back.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        for button in self.buttons_mut() {
            if button.handle_events(scene, window, glfw, event)
                && !matches!(event, WindowEvent::CursorPos(..))
            {
                return true;
            }
        }
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.rect().contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _parent: UIElementHandle, _child: Box<dyn UIElement>) {}
    fn contains_child(&self, _handle: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index);
        self.body.set_z_index(z_index + 2.0);
        for button in self.buttons_mut() {
            button.set_z_index(z_index + 3.0);
        }
    }
}

/// Seconds the settings have to stay untouched before the preview loads
/// the model again (typing a scale would otherwise reload per digit).
const RELOAD_DELAY: f32 = 0.35;
/// Camera distance in model radii at which a model fits the view.
const DEFAULT_ZOOM: f32 = 3.6;
/// Longest edge (px) a texture is shown at, room permitting.
const TEXTURE_SIDE: f32 = 384.0;

struct Orbit {
    yaw: f32,
    pitch: f32,
    /// In multiples of the model's radius.
    zoom: f32,
    dragging: Option<(f64, f64)>,
}

/// What the preview currently shows.
enum Shown {
    Nothing,
    Model {
        instances: ScatterInstances,
        /// Centre and radius of the shown variant.
        center: Point3<f32>,
        radius: f32,
        floor: f32,
    },
    Texture {
        texture: Texture,
        renderer: TextureRenderer,
        /// Size of the image in pixels.
        size: (u32, u32),
    },
    Failed(String),
}

/// The asset workspace's main area: a 3D view of the model with an orbit
/// camera, or the image of a texture.
pub struct AssetPreview {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    shown: Shown,
    /// `(asset, preview counter)` that is shown.
    shown_for: Option<(AssetId, u64)>,
    /// `(asset, variant)` the camera was framed for.
    framed_for: Option<(AssetId, usize)>,
    /// When a reload became necessary.
    pending_since: Option<Instant>,
    message: Text,
    hint: Text,
    orbit: Orbit,
}

impl AssetPreview {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Preview".into(), Position::default(), Size::new(600.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            shown: Shown::Nothing,
            shown_for: None,
            framed_for: None,
            pending_since: None,
            message: theme::text("", theme::FONT_SIZE, theme::TEXT_MUTED),
            hint: theme::text(
                "Drag to orbit, scroll to zoom",
                theme::FONT_SIZE_SMALL,
                theme::TEXT_DIM,
            ),
            orbit: Orbit {
                yaw: 0.6,
                pitch: 0.25,
                zoom: DEFAULT_ZOOM,
                dragging: None,
            },
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.viewport
    }

    /// The area below the panel header.
    fn view_rect(&self) -> Rect {
        let r = self.rect();
        let header = self.panel.header_height();
        Rect {
            x: r.x + 1.0,
            y: r.y + header,
            w: (r.w - 2.0).max(1.0),
            h: (r.h - header - 1.0).max(1.0),
        }
    }

    fn load(&mut self) {
        let state = self.state.borrow();
        let Some(edit) = &state.asset_edit else {
            self.shown = Shown::Nothing;
            return;
        };
        self.shown = match &edit.meta.kind {
            AssetKind::Texture(_) => {
                let texture = Texture::new();
                match texture.try_load_from_file(&edit.source_path()) {
                    Ok(size) => Shown::Texture {
                        texture,
                        renderer: TextureRenderer::new(),
                        size,
                    },
                    Err(e) => Shown::Failed(e.to_string()),
                }
            }
            AssetKind::Model(model) => {
                let variant = model.variants.get(edit.variant).map(|v| v.name.as_str());
                match ScatterModel::load_preview(&edit.source_path(), model, variant) {
                    Ok(loaded) => {
                        let (min, max) = loaded.bounds(0).unwrap_or(([0.0; 3], [1.0; 3]));
                        let center = Point3::new(
                            (min[0] + max[0]) * 0.5,
                            (min[1] + max[1]) * 0.5,
                            (min[2] + max[2]) * 0.5,
                        );
                        let radius = (0..3)
                            .map(|axis| (max[axis] - min[axis]) * 0.5)
                            .fold(0.0f32, |sum, half| sum + half * half)
                            .sqrt()
                            .max(0.01);
                        let spec = ScatterSpec {
                            model: loaded,
                            attempts: 1,
                            salt: 0,
                        };
                        Shown::Model {
                            instances: ScatterInstances::new(&spec, &[(0.0, 0.0, 0.0, 0.0)]),
                            center,
                            radius,
                            floor: min[1],
                        }
                    }
                    Err(e) => Shown::Failed(e),
                }
            }
        };
    }

    fn view_projection(&self, view: &Rect, center: Point3<f32>, radius: f32) -> Matrix4<f32> {
        let o = &self.orbit;
        let distance = radius * o.zoom;
        let eye = center
            + Vector3::new(
                distance * o.pitch.cos() * o.yaw.sin(),
                distance * o.pitch.sin(),
                distance * o.pitch.cos() * o.yaw.cos(),
            );
        let projection = perspective(
            Deg(38.0),
            view.w / view.h.max(1.0),
            (radius * 0.02).max(0.001),
            radius * 40.0,
        );
        projection * Matrix4::look_at_rh(eye, center, Vector3::unit_y())
    }
}

impl UIElement for AssetPreview {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let wanted = {
            let state = self.state.borrow();
            state
                .asset_edit
                .as_ref()
                .map(|edit| (edit.meta.id, edit.preview, edit.variant))
        };
        let wanted_key = wanted.map(|(id, preview, _)| (id, preview));
        if wanted_key != self.shown_for {
            // Another asset shows at once; edits of the same one wait until
            // they have settled.
            let same_asset = matches!(
                (wanted_key, self.shown_for),
                (Some((a, _)), Some((b, _))) if a == b
            );
            let since = *self.pending_since.get_or_insert_with(Instant::now);
            if !same_asset || since.elapsed().as_secs_f32() >= RELOAD_DELAY {
                self.load();
                self.shown_for = wanted_key;
                self.pending_since = None;
            }
        }
        // A fresh camera for another asset or variant, not for every edit.
        let frame_key = wanted.map(|(id, _, variant)| (id, variant));
        if frame_key != self.framed_for {
            self.framed_for = frame_key;
            self.orbit.yaw = 0.6;
            self.orbit.pitch = 0.25;
            self.orbit.zoom = DEFAULT_ZOOM;
        }

        let view = self.view_rect();
        let message = match &self.shown {
            Shown::Nothing => "No asset is open.".to_string(),
            Shown::Failed(error) => format!("The asset could not be loaded: {error}"),
            Shown::Texture { size, .. } => format!("{} x {} px", size.0, size.1),
            Shown::Model { .. } => String::new(),
        };
        self.message.set_content(&message);
        self.message
            .prepare_render_at(Position::new(view.x + 16.0, view.y + 16.0, 3.0));
        self.hint.prepare_render_at(Position::new(
            view.x + 12.0,
            view.y + view.h - 24.0,
            3.0,
        ));
        self.panel.update(scene);
    }

    fn render(&self) {
        self.panel.render();
        let view = self.view_rect();
        match &self.shown {
            Shown::Model {
                instances,
                center,
                radius,
                floor,
            } => {
                let (window_w, window_h, lod) = {
                    let state = self.state.borrow();
                    (
                        state.layout.window_w,
                        state.layout.window_h,
                        state.asset_edit.as_ref().and_then(|edit| edit.lod),
                    )
                };
                let gl_rect = [
                    view.x as i32,
                    (window_h - view.y - view.h) as i32,
                    view.w as i32,
                    view.h as i32,
                ];
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(gl_rect[0], gl_rect[1], gl_rect[2], gl_rect[3]);
                    gl::Viewport(gl_rect[0], gl_rect[1], gl_rect[2], gl_rect[3]);
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, *center, *radius);

                // Floor grid under the model, sized to it.
                let extent = radius * 1.6;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(
                        Point3::new(center.x + t, *floor, center.z - extent),
                        Vector3::unit_z(),
                        extent * 2.0,
                    ));
                    grid.push(Line::new(
                        Point3::new(center.x - extent, *floor, center.z + t),
                        Vector3::unit_x(),
                        extent * 2.0,
                    ));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);

                let light = *center + Vector3::new(radius * 3.0, radius * 5.0, radius * 4.0);
                let lod = match lod {
                    Some(tier) => LodChoice::Fixed(tier),
                    // Close up, as the camera never gets far in model terms.
                    None => LodChoice::Distance(0.0),
                };
                instances.draw(&vp, &Matrix4::identity(), light, lod);
                unsafe {
                    // Leave no 3D depth behind for the UI drawn over this
                    // region.
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                self.hint.render();
            }
            Shown::Texture {
                texture,
                renderer,
                size,
            } => {
                // As large as fits, keeping the image's proportions.
                let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
                let room = ((view.w - 48.0).max(16.0), (view.h - 72.0).max(16.0));
                let scale = (TEXTURE_SIDE / w.max(h)).min(room.0 / w).min(room.1 / h);
                let (w, h) = ((w * scale).round(), (h * scale).round());
                let x = (view.x + (view.w - w) * 0.5).round();
                let y = (view.y + (view.h - h) * 0.5).round();
                // The texture renderer works in clip space.
                let (window_w, window_h) = {
                    let state = self.state.borrow();
                    (state.layout.window_w, state.layout.window_h)
                };
                renderer.render_color(
                    texture,
                    x / window_w * 2.0 - 1.0,
                    1.0 - (y + h) / window_h * 2.0,
                    w / window_w * 2.0,
                    h / window_h * 2.0,
                );
                self.message.render();
            }
            Shown::Nothing | Shown::Failed(_) => {
                self.message.render();
            }
        }
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        window: &mut glfw::Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let view = self.view_rect();
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.orbit.dragging = Some((x, y));
                    return true;
                }
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.orbit.dragging.take().is_some() {
                    return true;
                }
            }
            WindowEvent::CursorPos(x, y) => {
                if let Some((lx, ly)) = self.orbit.dragging {
                    self.orbit.yaw -= (*x - lx) as f32 * 0.01;
                    self.orbit.pitch =
                        (self.orbit.pitch + (*y - ly) as f32 * 0.01).clamp(-1.4, 1.4);
                    self.orbit.dragging = Some((*x, *y));
                    return true;
                }
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.orbit.zoom = (self.orbit.zoom * (1.0 - *dy as f32 * 0.1)).clamp(1.2, 12.0);
                    return true;
                }
            }
            _ => {}
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.rect().contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _p: UIElementHandle, _c: Box<dyn UIElement>) {}
    fn contains_child(&self, _h: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _h: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _h: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z: f32) {
        self.panel.set_z_index(z);
        self.message.set_z_index(z + 3.0);
        self.hint.set_z_index(z + 3.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite::core::assets::{ModelSettings, ASSET_META_VERSION};

    fn edit() -> AssetEdit {
        AssetEdit {
            meta: AssetMeta {
                version: ASSET_META_VERSION,
                id: AssetId::from(5),
                name: "pack".into(),
                source: "pack.gltf".into(),
                kind: AssetKind::Model(ModelSettings {
                    scale: 1.0,
                    variants: Vec::new(),
                    materials: vec![MaterialSettings {
                        name: "bark".into(),
                        ..MaterialSettings::default()
                    }],
                }),
            },
            dir: PathBuf::from("assets/models/test"),
            dirty: false,
            variant: 0,
            lod: None,
            structure: 0,
            preview: 0,
        }
    }

    #[test]
    fn giving_a_material_a_texture_marks_the_asset_changed() {
        let mut edit = edit();
        let texture = AssetId::from(9);
        edit.set_texture(0, TextureSlot::Normal, Some(texture));
        let material = &edit.meta.as_model().unwrap().materials[0];
        assert_eq!(material.normal, Some(texture));
        assert_eq!(material.base_color, None);
        assert!(edit.dirty);
        assert_eq!((edit.structure, edit.preview), (1, 1));

        // The same texture again is not a change.
        edit.dirty = false;
        edit.set_texture(0, TextureSlot::Normal, Some(texture));
        assert!(!edit.dirty);
        assert_eq!((edit.structure, edit.preview), (1, 1));

        // A material that does not exist is ignored.
        edit.set_texture(7, TextureSlot::BaseColor, Some(texture));
        assert!(!edit.dirty);
    }
}
