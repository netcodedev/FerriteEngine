//! Editor state shared between the layer and its panels.
//!
//! Panels never replace the scene themselves; they queue `Command`s that the
//! `EditorLayer` executes after the UI pass, because some commands (open,
//! play/stop) swap the whole `Scene` out from under them.

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

use ferrite::core::{entity::EntityHandle, scene::scene_file::ComponentData, utils::DataSource};
use ferrite_ui::ui::elements::tree::NO_SELECTION;

pub mod anim;
pub mod dialogs;
pub mod field_row;
pub mod hierarchy;
pub mod inspector;
pub mod layout;
pub mod outline;
pub mod showcase;
pub mod splitter;
pub mod stats;
pub mod titlebar;
pub mod toolbar;
pub mod toggle_window;
pub mod viewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Play,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Scene,
    Animation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
}

/// What "Add entity" creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityTemplate {
    Empty,
    Camera,
    SkyLight,
    Terrain,
    Player,
}

impl EntityTemplate {
    pub const ALL: [(&'static str, EntityTemplate); 5] = [
        ("Empty", EntityTemplate::Empty),
        ("Camera", EntityTemplate::Camera),
        ("Sky light", EntityTemplate::SkyLight),
        ("Terrain", EntityTemplate::Terrain),
        ("Player", EntityTemplate::Player),
    ];
}

#[derive(Clone, Debug)]
pub enum Command {
    NewScene,
    OpenDialog,
    Open(PathBuf),
    Save,
    SaveAsDialog,
    SaveAs(PathBuf),
    /// Run `then` once unsaved changes are saved or discarded.
    ConfirmDiscard(Box<Command>),
    Play,
    Stop,
    AddEntity {
        parent: Option<EntityHandle>,
        template: EntityTemplate,
    },
    AddComponent(EntityHandle, ComponentData),
    RemoveComponent(EntityHandle, usize),
    /// Choose a model file for a scatter layer of this entity's terrain.
    /// Without a layer index a new layer is added.
    PickScatterModel {
        entity: EntityHandle,
        layer: Option<usize>,
    },
    /// Remove a scatter layer from this entity's terrain.
    RemoveScatterModel(EntityHandle, usize),
    Delete(EntityHandle),
    Duplicate(EntityHandle),
    Focus(EntityHandle),
    /// Close the editor window.
    Quit,
    SetWorkspace(Workspace),
    /// Save the animation graph and apply it to players in the scene.
    SaveGraph,
    RevertGraph,
    /// Delete the selected state or transition of the animation graph.
    DeleteGraphSelection,
    /// Open the graph editor on the graph file of this entity's animation
    /// component.
    EditAnimationGraph(EntityHandle),
    /// Leave the graph editor (unsaved edits stay in memory).
    CloseGraphEditor,
}

pub struct EditorState {
    /// Selected entity id, shared with the hierarchy tree (`NO_SELECTION`
    /// when empty).
    pub selection: DataSource<u64>,
    pub mode: Mode,
    pub workspace: Workspace,
    /// Visibility flags for the panels of each workspace.
    pub scene_panels: DataSource<bool>,
    pub anim_panels: DataSource<bool>,
    pub anim: anim::AnimEditor,
    /// Graph-space point at the centre of the node canvas (new states go
    /// there).
    pub anim_canvas_center: Option<(f32, f32)>,
    pub gizmo: GizmoMode,
    pub scene_path: Option<PathBuf>,
    pub dirty: bool,
    status: Option<(String, Instant)>,
    pub commands: Vec<Command>,
    pub layout: layout::Layout,
    /// Sizes the user dragged the panels to.
    pub panel_sizes: layout::PanelSizes,
    pub show_widgets: DataSource<bool>,
    pub show_stats: DataSource<bool>,
    pub stats: stats::Stats,
    /// A popup the modal host should show next (save-as, open, confirm).
    pub pending_modal: Option<dialogs::Modal>,
}

pub type Shared = Rc<RefCell<EditorState>>;

impl EditorState {
    pub fn new() -> Shared {
        Rc::new(RefCell::new(Self {
            selection: DataSource::new(NO_SELECTION),
            mode: Mode::Edit,
            workspace: Workspace::Scene,
            scene_panels: DataSource::new(true),
            anim_panels: DataSource::new(false),
            anim: anim::AnimEditor::new(),
            anim_canvas_center: None,
            gizmo: GizmoMode::Translate,
            scene_path: None,
            dirty: false,
            status: None,
            commands: Vec::new(),
            layout: layout::Layout::compute(
                1280.0,
                720.0,
                layout::RightColumn::Panels {
                    stats: true,
                    widgets: false,
                },
                &layout::PanelSizes::default(),
            ),
            panel_sizes: layout::PanelSizes::default(),
            show_widgets: DataSource::new(false),
            show_stats: DataSource::new(true),
            stats: stats::Stats::new(),
            pending_modal: None,
        }))
    }

    pub fn selected(&self) -> Option<EntityHandle> {
        match self.selection.read() {
            NO_SELECTION => None,
            id => Some(EntityHandle::from(id)),
        }
    }

    pub fn select(&self, id: Option<EntityHandle>) {
        self.selection
            .write(id.map(u64::from).unwrap_or(NO_SELECTION));
    }

    pub fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    /// The latest status message, hidden after a few seconds.
    pub fn status(&self) -> Option<&str> {
        match &self.status {
            Some((text, at)) if at.elapsed().as_secs_f32() < 6.0 => Some(text),
            _ => None,
        }
    }

    pub fn scene_title(&self) -> String {
        let name = self
            .scene_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        if self.dirty {
            format!("{name} *")
        } else {
            name
        }
    }
}

/// Queue a command from a UI callback.
pub fn send(state: &Shared, command: Command) {
    state.borrow_mut().push(command);
}
