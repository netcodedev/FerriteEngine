use std::path::{Path, PathBuf};

use ferrite::core::{
    assets::{self, AssetDatabase, AssetId, AssetKind, AssetRef, NewAsset},
    entity::EntityHandle,
    primitives::{Offset, Size},
    scene::{scene_file::ComponentData, Scene},
    utils::DataSource,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, elements::popup::Popup, UIElement, UI};
use glfw::{Glfw, WindowEvent};

use ferrite::core::material::asset::TextureSetting;

use super::{send, Command, Shared};

pub const SCENE_DIR: &str = "scenes";

/// Let the user choose files to import with the system's file dialog
/// (Windows: the native dialog; Linux: the desktop portal, or zenity
/// without one). Blocks until the dialog is closed; empty if cancelled.
pub fn pick_import_files() -> Vec<PathBuf> {
    let extensions = AssetDatabase::importable_extensions();
    rfd::FileDialog::new()
        .set_title(tr!("dialogs.import.title"))
        .add_filter(tr!("dialogs.import.filter_models"), &extensions)
        .add_filter(tr!("dialogs.import.filter_all"), &["*"])
        .pick_files()
        .unwrap_or_default()
}

/// The model assets a scatter layer can use: each asset as a whole and,
/// for assets with several variants, each variant.
fn model_choices() -> Vec<(String, AssetRef)> {
    let database = assets::database();
    let mut models: Vec<_> = database
        .iter()
        .filter_map(|entry| Some((entry, entry.meta.as_model()?)))
        .collect();
    models.sort_by_key(|(entry, _)| entry.meta.name.to_lowercase());
    let mut choices = Vec::new();
    for (entry, model) in models {
        let id = entry.meta.id;
        if model.variants.len() > 1 {
            choices.push((
                tr!("dialogs.model_all_variants", name = entry.meta.name, count = model.variants.len()),
                AssetRef::whole(id),
            ));
            for variant in &model.variants {
                choices.push((
                    format!("{} / {}", entry.meta.name, variant.name),
                    AssetRef::part(id, variant.name.clone()),
                ));
            }
        } else {
            choices.push((entry.meta.name.clone(), AssetRef::whole(id)));
        }
    }
    choices
}

/// Ask which model asset a scatter layer of `entity`'s terrain should use.
/// Without a layer index the choice becomes a new layer.
/// A scatter layer plants a scatter graph.
pub fn pick_scatter_asset(state: &Shared, entity: EntityHandle, layer: Option<usize>) {
    pick_asset_of(state, scatter_graph_choices(), move |asset| Command::SetScatterAsset {
        entity,
        layer,
        asset,
    });
}

/// Let the user choose one of the project's scripts to add to `entity`.
pub fn pick_script(state: &Shared, entity: EntityHandle) {
    let names = ferrite::core::script::names();
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    if names.is_empty() {
        let children: Vec<Box<dyn UIElement>> = vec![
            UI::label(tr!("dialogs.add_script.none"), 14.0),
            UI::label(tr!("dialogs.add_script.none_hint"), 14.0),
            button_row(vec![UI::button(
                tr!("common.close"),
                Box::new(move |_| cancel_open.write(false)),
            )]),
        ];
        show(state, tr!("dialogs.add_script.title"), open_flag, children);
        return;
    }
    let selected = DataSource::new(0u32);
    let height = (names.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let choices = names.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(tr!("dialogs.add_script.heading"), 14.0),
        UI::list(names, selected, Size::new(380.0, height)),
        button_row(vec![
            UI::button(
                tr!("common.add"),
                Box::new(move |_| {
                    if let Some(name) = choices.get(use_selected.read() as usize) {
                        send(
                            &use_state,
                            Command::AddComponent(entity, ComponentData::script(name)),
                        );
                        use_open.write(false);
                    }
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.add_script.title"), open_flag, children);
}

/// Ask for the name of a new script.
pub fn new_script(state: &Shared) {
    let name = DataSource::new(String::new());
    let open_flag = DataSource::new(true);
    let create_state = state.clone();
    let create_name = name.clone();
    let create_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(tr!("dialogs.new_script.prompt"), 14.0),
        UI::input_sized(name, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                tr!("common.create"),
                Box::new(move |_| {
                    let name = create_name.read().trim().to_owned();
                    if name.is_empty() {
                        return;
                    }
                    send(&create_state, Command::NewScript(name));
                    create_open.write(false);
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.new_script.title"), open_flag, children);
}

/// Ask for the name of a new animation clip posed on `model`.
pub fn new_clip(state: &Shared, model: AssetId) {
    let name = DataSource::new(String::new());
    let open_flag = DataSource::new(true);
    let create_state = state.clone();
    let create_name = name.clone();
    let create_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(tr!("dialogs.new_clip.prompt"), 14.0),
        UI::input_sized(name, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                tr!("common.create"),
                Box::new(move |_| {
                    let name = create_name.read().trim().to_owned();
                    if name.is_empty() {
                        return;
                    }
                    send(&create_state, Command::NewClip { model, name });
                    create_open.write(false);
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.new_clip.title"), open_flag, children);
}

/// Ask which model asset `entity` should show.
pub fn pick_entity_model(state: &Shared, entity: EntityHandle) {
    pick_model_asset(state, move |asset| Command::SetEntityModel { entity, asset });
}

/// Let the user choose a model asset (or one of its variants) and queue
/// the command `then` makes of the choice.
/// Ask which model a scatter graph's `Spawn mesh` node plants.
pub fn pick_scatter_model(state: &Shared, node: ferrite::terrain::scatter::graph::NodeId) {
    pick_model_asset(state, move |asset| Command::SetScatterModel { node, asset });
}

fn pick_model_asset(state: &Shared, then: impl Fn(AssetRef) -> Command + 'static) {
    pick_asset_of(state, model_choices(), then);
}

/// The project's scatter graphs, by name.
fn scatter_graph_choices() -> Vec<(String, AssetRef)> {
    let database = assets::database();
    let mut graphs: Vec<(String, AssetRef)> = database
        .iter()
        .filter(|entry| matches!(entry.meta.kind, AssetKind::ScatterGraph(_)))
        .map(|entry| (tr!("dialogs.scatter_graph_choice", name = entry.meta.name), AssetRef::whole(entry.meta.id)))
        .collect();
    graphs.sort_by_key(|(name, _)| name.to_lowercase());
    graphs
}

fn pick_asset_of(state: &Shared, choices: Vec<(String, AssetRef)>, then: impl Fn(AssetRef) -> Command + 'static) {
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    if choices.is_empty() {
        let children: Vec<Box<dyn UIElement>> = vec![
            UI::label(tr!("dialogs.choose_model.none"), 14.0),
            UI::label(tr!("dialogs.choose_model.none_hint"), 14.0),
            button_row(vec![UI::button(
                tr!("common.close"),
                Box::new(move |_| cancel_open.write(false)),
            )]),
        ];
        show(state, tr!("dialogs.choose_model.title"), open_flag, children);
        return;
    }

    let selected = DataSource::new(0u32);
    let labels: Vec<String> = choices.iter().map(|(label, _)| label.clone()).collect();
    let height = (labels.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(tr!("dialogs.choose_model.heading"), 14.0),
        UI::list(labels, selected, Size::new(380.0, height)),
        button_row(vec![
            UI::button(
                tr!("dialogs.use"),
                Box::new(move |_| {
                    if let Some((_, asset)) = choices.get(use_selected.read() as usize) {
                        send(&use_state, then(asset.clone()));
                        use_open.write(false);
                    }
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.choose_model.title"), open_flag, children);
}

/// A list to choose one of `choices` from; `then` gets the chosen value.
fn pick_from<T: Clone + 'static>(
    state: &Shared,
    title: &str,
    heading: &str,
    empty_note: Option<&str>,
    choices: Vec<(String, T)>,
    then: impl Fn(T) -> Command + 'static,
) {
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    let selected = DataSource::new(0u32);
    let labels: Vec<String> = choices.iter().map(|(label, _)| label.clone()).collect();
    let height = (labels.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let mut children: Vec<Box<dyn UIElement>> = vec![
        UI::label(heading, 14.0),
        UI::list(labels, selected, Size::new(380.0, height)),
    ];
    if let Some(note) = empty_note {
        children.push(UI::label(note, 14.0));
    }
    children.push(button_row(vec![
        UI::button(
            tr!("dialogs.use"),
            Box::new(move |_| {
                if let Some((_, value)) = choices.get(use_selected.read() as usize) {
                    send(&use_state, then(value.clone()));
                    use_open.write(false);
                }
            }),
        ),
        UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
    ]));
    show(state, title, open_flag, children);
}

/// Assets of a kind, by name.
fn assets_of(kind: impl Fn(&AssetKind) -> bool) -> Vec<(String, AssetId)> {
    let database = assets::database();
    let mut found: Vec<(String, AssetId)> = database
        .iter()
        .filter(|entry| kind(&entry.meta.kind))
        .map(|entry| (entry.meta.name.clone(), entry.meta.id))
        .collect();
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

/// Ask which material asset a material slot of the edited model asset is
/// drawn with. The first choice goes back to the file's own material.
pub fn pick_model_material(state: &Shared, material: usize) {
    let mut choices: Vec<(String, Option<AssetId>)> = vec![(tr!("dialogs.material.from_file").to_string(), None)];
    choices.extend(assets_of(|k| matches!(k, AssetKind::Material(_))).into_iter().map(|(n, id)| (n, Some(id))));
    let empty = (choices.len() == 1).then_some(tr!("dialogs.material.none"));
    pick_from(state, tr!("dialogs.material.title"), tr!("dialogs.material.heading"), empty, choices, move |asset| {
        Command::SetModelMaterial { material, asset }
    });
}

/// Ask which texture a texture parameter of the material being edited
/// uses: automatic (what the model loader finds), none, or a texture asset.
pub fn pick_material_texture(state: &Shared, param: String) {
    let mut choices: Vec<(String, TextureSetting)> = vec![
        (tr!("dialogs.texture.auto").to_string(), TextureSetting::Auto),
        (tr!("dialogs.texture.none").to_string(), TextureSetting::None),
    ];
    choices.extend(
        assets_of(|k| matches!(k, AssetKind::Texture(_))).into_iter().map(|(n, id)| (n, TextureSetting::Asset(id))),
    );
    let empty = (choices.len() == 2).then_some(tr!("dialogs.texture.none_yet"));
    let heading = tr!("dialogs.texture.heading", param = param);
    pick_from(state, tr!("dialogs.texture.title"), &heading, empty, choices, move |texture| {
        Command::SetMaterialTexture { param: param.clone(), texture }
    });
}

/// A popup plus the flag that keeps it open (its close button and the
/// dialog's own buttons write `false`).
pub struct Modal {
    pub open: DataSource<bool>,
    pub popup: Popup,
}

/// Queue a popup for the modal host. `open` is shared with the dialog's
/// buttons, so either they or the popup's own "x" can close it.
fn show(state: &Shared, title: &str, open: DataSource<bool>, children: Vec<Box<dyn UIElement>>) {
    let popup = Popup::new(title.to_owned(), open.clone(), children);
    state.borrow_mut().pending_modal = Some(Modal { open, popup });
}

fn button_row(buttons: Vec<Box<dyn UIElement>>) -> Box<dyn UIElement> {
    let mut row = UI::container_horizontal();
    for button in buttons {
        row.add_child(button);
    }
    row
}

/// Ask where to save the scene. `then` runs once it is saved.
pub fn save_as(state: &Shared, then: Option<Command>) {
    let current = state
        .borrow()
        .scene_path
        .clone()
        .unwrap_or_else(|| Path::new(SCENE_DIR).join("untitled.ron"));
    let path = DataSource::new(current.to_string_lossy().into_owned());
    let open_flag = DataSource::new(true);

    let save_state = state.clone();
    let save_path = path.clone();
    let save_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(tr!("dialogs.save_as.prompt"), 14.0),
        UI::input_sized(path, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                tr!("common.save"),
                Box::new(move |_| {
                    let mut text = save_path.read().trim().to_owned();
                    if text.is_empty() {
                        return;
                    }
                    if !text.ends_with(".ron") {
                        text.push_str(".ron");
                    }
                    let path = PathBuf::from(text);
                    let then = then.clone().map(Box::new);
                    let current = save_state.borrow().scene_path.clone();
                    let replaces_another = path.exists()
                        && !current.is_some_and(|current| same_file(&current, &path));
                    if replaces_another {
                        // Replaces this popup with the question.
                        confirm_overwrite(&save_state, path, then);
                    } else {
                        send(&save_state, Command::SaveAs { path, then });
                        save_open.write(false);
                    }
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.save_as.title"), open_flag, children);
}

pub fn open(state: &Shared) {
    let files = scene_files();
    let selected = DataSource::new(0u32);
    let typed = DataSource::new(String::new());
    let open_flag = DataSource::new(true);

    let mut children: Vec<Box<dyn UIElement>> = Vec::new();
    if files.is_empty() {
        children.push(UI::label(&tr!("dialogs.open.none", dir = SCENE_DIR), 14.0));
    } else {
        children.push(UI::label(&tr!("dialogs.open.heading", dir = SCENE_DIR), 14.0));
        let labels = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let height = (files.len().min(8) as f32) * 20.0 + 4.0;
        children.push(UI::list(labels, selected.clone(), Size::new(380.0, height)));
    }
    children.push(UI::label(tr!("dialogs.open.type_path"), 14.0));
    children.push(UI::input_sized(typed.clone(), Size::new(380.0, 22.0)));

    let open_state = state.clone();
    let accept_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    children.push(button_row(vec![
        UI::button(
            tr!("common.open"),
            Box::new(move |_| {
                let typed = typed.read().trim().to_owned();
                let path = if !typed.is_empty() {
                    Some(PathBuf::from(typed))
                } else {
                    files.get(selected.read() as usize).cloned()
                };
                if let Some(path) = path {
                    send(&open_state, Command::Open(path));
                    accept_open.write(false);
                }
            }),
        ),
        UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
    ]));
    show(state, tr!("dialogs.open.title"), open_flag, children);
}

/// Whether two paths name the same file (`a` may be relative, `b` absolute).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

/// `path` exists already and is not the open scene: replace it?
fn confirm_overwrite(state: &Shared, path: PathBuf, then: Option<Box<Command>>) {
    let open_flag = DataSource::new(true);
    let replace_state = state.clone();
    let replace_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&tr!("dialogs.replace.exists", path = path.display()), 14.0),
        UI::label(tr!("dialogs.replace.question"), 14.0),
        button_row(vec![
            UI::button(
                tr!("dialogs.replace.button"),
                Box::new(move |_| {
                    send(
                        &replace_state,
                        Command::SaveAs {
                            path: path.clone(),
                            then: then.clone(),
                        },
                    );
                    replace_open.write(false);
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.replace.title"), open_flag, children);
}

/// What has not been saved yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Unsaved {
    pub scene: bool,
    pub graph: bool,
    pub asset: bool,
}

impl Unsaved {
    pub fn any(&self) -> bool {
        self.scene || self.graph || self.asset
    }

    fn describe(&self) -> String {
        let parts: Vec<&str> = [
            (self.scene, tr!("dialogs.unsaved.the_scene")),
            (self.graph, tr!("dialogs.unsaved.the_graph")),
            (self.asset, tr!("dialogs.unsaved.the_asset")),
        ]
        .into_iter()
        .filter_map(|(unsaved, what)| unsaved.then_some(what))
        .collect();
        let list = match parts.as_slice() {
            [] => tr!("dialogs.unsaved.nothing").to_string(),
            [one] => one.to_string(),
            [rest @ .., last] => tr!("dialogs.unsaved.and", rest = rest.join(", "), last = last),
        };
        tr!("dialogs.unsaved.changes_in", list = list)
    }
}

/// Unsaved changes stand in the way of `then`: save them first, discard
/// them, or cancel. With `go_on`, its button replaces "Discard" and runs its
/// command with the changes kept (an export leaves them out).
pub fn confirm_discard(
    state: &Shared,
    then: Command,
    unsaved: Unsaved,
    go_on: Option<(&str, Command)>,
) {
    let open_flag = DataSource::new(true);
    let save_state = state.clone();
    let save_open = open_flag.clone();
    let save_then = then.clone();
    let cancel_open = open_flag.clone();
    let other_state = state.clone();
    let other_open = open_flag.clone();
    let other = match go_on {
        Some((label, command)) => UI::button(
            label,
            Box::new(move |_| {
                send(&other_state, command.clone());
                other_open.write(false);
            }),
        ),
        None => UI::button(
            tr!("dialogs.discard"),
            Box::new(move |_| {
                other_state.borrow_mut().dirty = false;
                send(&other_state, then.clone());
                other_open.write(false);
            }),
        ),
    };
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&unsaved.describe(), 14.0),
        button_row(vec![
            UI::button(
                tr!("dialogs.save_first"),
                Box::new(move |_| {
                    // Runs `then` only once everything is saved.
                    send(&save_state, Command::SaveFirst(Box::new(save_then.clone())));
                    save_open.write(false);
                }),
            ),
            other,
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.unsaved.title"), open_flag, children);
}

/// The open graph has unsaved edits and another graph should be opened.
pub fn confirm_graph_switch(state: &Shared, current: &str, then: Command) {
    let open_flag = DataSource::new(true);
    let (save_state, save_open, save_then) = (state.clone(), open_flag.clone(), then.clone());
    let (discard_state, discard_open) = (state.clone(), open_flag.clone());
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&tr!("dialogs.graph_switch.message", name = current), 14.0),
        button_row(vec![
            UI::button(
                tr!("dialogs.save_first"),
                Box::new(move |_| {
                    // Switches only once the graph is saved.
                    send(&save_state, Command::SaveFirst(Box::new(save_then.clone())));
                    save_open.write(false);
                }),
            ),
            UI::button(
                tr!("dialogs.discard"),
                Box::new(move |_| {
                    discard_state.borrow_mut().anim.dirty = false;
                    send(&discard_state, then.clone());
                    discard_open.write(false);
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, tr!("dialogs.graph_switch.title"), open_flag, children);
}

fn scene_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(SCENE_DIR)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |ext| ext == "ron"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// Shows the editor's current modal dialog (at most one at a time).
pub struct ModalHost {
    handle: UIElementHandle,
    state: Shared,
    current: Option<Modal>,
    offset: Offset,
    size: Size,
}

impl ModalHost {
    pub fn new(state: Shared) -> Self {
        Self {
            handle: UIElementHandle::new(),
            state,
            current: None,
            offset: Offset::default(),
            size: Size::default(),
        }
    }

}

impl UIElement for ModalHost {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(modal) = self.state.borrow_mut().pending_modal.take() {
            self.current = Some(modal);
        }
        if self.current.as_ref().map_or(false, |m| !m.open.read()) {
            self.current = None;
        }
        if let Some(modal) = &mut self.current {
            modal.popup.update(scene);
        }
    }

    fn render(&self) {}

    fn is_modal(&self) -> bool {
        self.current.is_some()
    }

    fn render_overlay(&self) {
        if let Some(modal) = &self.current {
            modal.popup.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match &mut self.current {
            Some(modal) => modal.popup.handle_events(scene, window, glfw, event),
            None => false,
        }
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, _z_index: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_changes_are_named() {
        let scene = Unsaved {
            scene: true,
            ..Default::default()
        };
        assert!(scene.any());
        assert!(!Unsaved::default().any());
        assert_eq!(scene.describe(), "There are unsaved changes in the scene.");
        let all = Unsaved {
            scene: true,
            graph: true,
            asset: true,
        };
        assert_eq!(
            all.describe(),
            "There are unsaved changes in the scene, the animation graph and the edited asset."
        );
    }
}

/// A dialog asking for one name: `prompt`, a field holding `initial`, and
/// `action` / Cancel. `make` turns the entered name into the command to run.
fn ask_name(state: &Shared, title: &str, prompt: &str, initial: &str, action: &str, make: impl Fn(String) -> Command + 'static) {
    let name = DataSource::new(initial.to_string());
    let open_flag = DataSource::new(true);
    let (ok_state, ok_name, ok_open) = (state.clone(), name.clone(), open_flag.clone());
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(prompt, 14.0),
        UI::input_sized(name, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                action,
                Box::new(move |_| {
                    let name = ok_name.read().trim().to_owned();
                    if name.is_empty() {
                        return;
                    }
                    send(&ok_state, make(name));
                    ok_open.write(false);
                }),
            ),
            UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, title, open_flag, children);
}

/// Ask for the name of a new asset of `kind` in `folder`.
pub fn new_asset(state: &Shared, kind: NewAsset, folder: Option<PathBuf>) {
    let root = assets::database().root().to_path_buf();
    let target = folder.clone().unwrap_or_else(|| root.join(kind.default_folder()));
    let suggestion = (1..)
        .map(|n| tr!("dialogs.new_asset.suggestion", kind = kind_in_sentence(new_asset_kind(kind)), n = n))
        .find(|n| !target.join(format!("{n}{}", kind.suffix())).exists())
        .expect("a free name");
    let place = target.strip_prefix(root.parent().unwrap_or(&root)).unwrap_or(&target).to_string_lossy().replace('\\', "/");
    ask_name(
        state,
        &tr!("dialogs.new_asset.title", kind = kind_in_sentence(new_asset_kind(kind))),
        &tr!("dialogs.new_asset.prompt", kind = kind_in_sentence(new_asset_kind(kind)), place = place),
        &suggestion,
        tr!("common.create"),
        move |name| Command::NewAsset { kind, folder: folder.clone(), name: Some(name) },
    );
}

/// Ask for the name of a new folder inside `parent`.
pub fn new_folder(state: &Shared, parent: PathBuf) {
    ask_name(state, tr!("dialogs.new_folder.title"), tr!("dialogs.new_folder.prompt"), tr!("dialogs.new_folder.default"), tr!("common.create"), move |name| Command::NewFolder {
        parent: parent.clone(),
        name,
    });
}

/// Ask for an asset's new name.
pub fn rename_asset(state: &Shared, id: AssetId, current: &str) {
    ask_name(
        state,
        tr!("dialogs.rename_asset.title"),
        tr!("dialogs.rename_asset.prompt"),
        current,
        tr!("common.rename"),
        move |name| Command::RenameAsset { id, name },
    );
}

/// Ask for a folder's new name.
pub fn rename_folder(state: &Shared, path: PathBuf) {
    let current = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ask_name(state, tr!("dialogs.rename_folder.title"), tr!("dialogs.rename_folder.prompt"), &current, tr!("common.rename"), move |name| Command::RenameFolder {
        path: path.clone(),
        name,
    });
}

/// At most this many referring files are listed by name.
const LISTED_REFERENCES: usize = 8;

/// Lines naming the files in `references`: the asset they belong to (with
/// its kind) or their path below the project.
fn reference_lines(database: &AssetDatabase, references: &[assets::Reference]) -> Vec<String> {
    let project = database.project_root();
    let mut lines: Vec<String> = references
        .iter()
        .take(LISTED_REFERENCES)
        .map(|r| {
            let path = r.file.strip_prefix(&project).unwrap_or(&r.file).to_string_lossy().replace('\\', "/");
            match r.asset.and_then(|id| database.get(id)) {
                Some(entry) => format!("  ·  {} ({})", entry.meta.name, kind_in_sentence(super::asset_browser::kind_label(&entry.meta.kind))),
                None => format!("  ·  {}", path.trim_start_matches("./")),
            }
        })
        .collect();
    if references.len() > LISTED_REFERENCES {
        lines.push(tr!("dialogs.references.more", count = references.len() - LISTED_REFERENCES));
    }
    lines
}

/// "1 file" / "n files".
fn file_count(count: usize) -> String {
    if count == 1 {
        tr!("dialogs.references.one_file").to_string()
    } else {
        tr!("dialogs.references.n_files", count = count)
    }
}

/// Ask before moving an asset to the trash, saying what still refers to it.
pub fn delete_asset(state: &Shared, id: AssetId) {
    let (lines, warning) = {
        let database = assets::database();
        let Some(entry) = database.get(id) else {
            return;
        };
        let project = database.project_root();
        let source = entry.source_path();
        let path = source.strip_prefix(&project).unwrap_or(&source).to_string_lossy().replace('\\', "/");
        let mut lines = vec![
            tr!("dialogs.delete_asset.question", kind = kind_in_sentence(super::asset_browser::kind_label(&entry.meta.kind)), name = entry.meta.name),
            tr!("dialogs.delete_asset.with_meta", path = path.trim_start_matches("./")),
        ];
        let references = database.references_to(id);
        let warning = !references.is_empty();
        if warning {
            lines.push(String::new());
            lines.push(tr!("dialogs.delete_asset.still_used", files = file_count(references.len())));
            lines.extend(reference_lines(&database, &references));
        } else {
            lines.push(tr!("dialogs.delete_asset.unused").into());
        }
        (lines, warning)
    };
    confirm_delete(state, if warning { tr!("dialogs.delete_asset.title_in_use") } else { tr!("dialogs.delete_asset.title") }, lines, Command::DeleteAsset(id));
}

/// Ask before moving a folder and all assets in it to the trash.
pub fn delete_folder(state: &Shared, path: PathBuf) {
    let lines = {
        let database = assets::database();
        let inside = database.assets_in(&path);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut lines = vec![
            tr!("dialogs.delete_folder.question", name = name),
            match inside.len() {
                0 => tr!("dialogs.delete_folder.holds_none").to_string(),
                1 => tr!("dialogs.delete_folder.holds_one").to_string(),
                n => tr!("dialogs.delete_folder.holds_many", count = n),
            },
        ];
        let references = database.references_into(&path);
        if !references.is_empty() {
            lines.push(String::new());
            lines.push(tr!("dialogs.delete_folder.referenced", files = file_count(references.len())));
            lines.extend(reference_lines(&database, &references));
        }
        lines
    };
    confirm_delete(state, tr!("dialogs.delete_folder.title"), lines, Command::DeleteFolder(path));
}

fn confirm_delete(state: &Shared, title: &str, lines: Vec<String>, then: Command) {
    let open_flag = DataSource::new(true);
    let (ok_state, ok_open) = (state.clone(), open_flag.clone());
    let cancel_open = open_flag.clone();
    let mut children: Vec<Box<dyn UIElement>> = lines.iter().map(|line| UI::label(line, 14.0) as Box<dyn UIElement>).collect();
    children.push(button_row(vec![
        UI::button(
            tr!("dialogs.move_to_trash"),
            Box::new(move |_| {
                send(&ok_state, then.clone());
                ok_open.write(false);
            }),
        ),
        UI::button(tr!("common.cancel"), Box::new(move |_| cancel_open.write(false))),
    ]));
    show(state, title, open_flag, children);
}

/// The shown name of a kind of new asset.
pub fn new_asset_kind(kind: NewAsset) -> &'static str {
    match kind {
        NewAsset::Material => tr!("asset_browser.kind.material"),
        NewAsset::MaterialGraph => tr!("asset_browser.kind.material_graph"),
        NewAsset::ScatterGraph => tr!("asset_browser.kind.scatter_graph"),
    }
}

/// A kind's name inside a sentence: lower case in English, as it is in
/// German (nouns keep their capital).
pub fn kind_in_sentence(label: &str) -> String {
    match crate::i18n::language() {
        crate::i18n::Language::English => label.to_lowercase(),
        _ => label.to_string(),
    }
}
