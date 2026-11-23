//! Editor state shared between the layer and its panels.
//!
//! Panels never replace the scene themselves; they queue `Command`s that the
//! `EditorLayer` executes after the UI pass, because some commands (open,
//! play/stop) swap the whole `Scene` out from under them.

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

use ferrite::core::{
    assets::{AssetId, AssetRef, NewAsset},
    entity::EntityHandle,
    scene::scene_file::ComponentData,
    utils::DataSource,
};
use ferrite_ui::ui::elements::tree::NO_SELECTION;

pub mod anim;
pub mod asset_browser;
mod asset_bridge;
pub mod bridge_handler;
pub mod asset_editor;
pub mod clip_editor;
pub mod console;
pub mod dialogs;
pub mod drag;
pub mod export;
pub mod field_row;
pub mod graph_canvas;
pub mod graphics_panel;
pub mod hierarchy;
pub mod inspector;
pub mod layout;
pub mod material;
pub mod outline;
pub mod project_settings;
pub mod scatter;
pub mod scripts;
pub mod settings_dialog;
pub mod showcase;
pub mod splitter;
pub mod stats;
pub mod titlebar;
pub mod toolbar;
pub mod toggle_window;
pub mod viewport;
pub mod world_map;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Play,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Scene,
    Animation,
    /// Settings and preview of one asset.
    Asset,
    /// A scatter graph's nodes (see `scatter`).
    ScatterGraph,
    /// A material or material graph (see `material`).
    Material,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
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

    /// Its name in the Add menu.
    pub fn label(self) -> &'static str {
        match self {
            EntityTemplate::Empty => tr!("entity_template.empty"),
            EntityTemplate::Camera => tr!("entity_template.camera"),
            EntityTemplate::SkyLight => tr!("entity_template.sky_light"),
            EntityTemplate::Terrain => tr!("entity_template.terrain"),
            EntityTemplate::Player => tr!("entity_template.player"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    NewScene,
    OpenDialog,
    Open(PathBuf),
    Save,
    SaveAsDialog,
    /// Save the scene to this path, then run `then` (only if it was saved).
    SaveAs {
        path: PathBuf,
        then: Option<Box<Command>>,
    },
    /// Run `then` once unsaved changes are saved or discarded.
    ConfirmDiscard(Box<Command>),
    /// Save whatever `then` would lose (or leave out), then run it; nothing
    /// runs if a save fails or is cancelled.
    SaveFirst(Box<Command>),
    Play,
    Stop,
    AddEntity {
        parent: Option<EntityHandle>,
        template: EntityTemplate,
    },
    AddComponent(EntityHandle, ComponentData),
    RemoveComponent(EntityHandle, usize),
    /// Let the user import files as assets (system file dialog).
    ImportAssets,
    /// Scan the assets folder again.
    RefreshAssets,
    /// Build the game's scripts and load them again.
    ReloadScripts,
    /// Take a RAM map snapshot (`ferrite::core::memory`) and compare it with
    /// the one before.
    MemorySnapshot,
    /// Let the user choose a folder and export the game for a platform
    /// into it.
    ExportGame(ferrite::core::export::Platform),
    /// The same, without asking about unsaved edits first.
    ExportAnyway(ferrite::core::export::Platform),
    /// Let the user choose a model asset for a scatter layer of this
    /// entity's terrain. Without a layer index a new layer is added.
    PickScatterAsset {
        entity: EntityHandle,
        layer: Option<usize>,
    },
    /// Use a model asset for a scatter layer of this entity's terrain.
    /// Without a layer index a new layer is added.
    SetScatterAsset {
        entity: EntityHandle,
        layer: Option<usize>,
        asset: AssetRef,
    },
    /// Open an asset in the asset workspace, showing this variant if given.
    EditAsset {
        id: AssetId,
        variant: Option<String>,
    },
    /// Write the edited asset to disk; whatever uses it reloads.
    SaveAsset,
    /// Discard the unsaved changes of the edited asset.
    RevertAsset,
    /// Leave the asset workspace (unsaved edits stay in memory).
    CloseAssetEditor,
    /// Ask for the name of a new animation clip posed on this model asset.
    NewClipDialog(AssetId),
    /// Create an animation clip posed on `model` and open it.
    NewClip { model: AssetId, name: String },
    /// Ask the open clip for something (shortcuts).
    Clip(clip_editor::ClipRequest),
    /// Let the user choose the material asset a material slot of the
    /// edited model asset is drawn with.
    PickModelMaterial { material: usize },
    /// Draw a material slot of the edited model asset with a material
    /// asset (`None`: what the loader makes of the file's material).
    SetModelMaterial { material: usize, asset: Option<AssetId> },
    /// Let the user choose a texture for a texture parameter of the
    /// material being edited.
    PickMaterialTexture { param: String },
    /// Set a texture parameter of the material being edited.
    SetMaterialTexture {
        param: String,
        texture: ferrite::core::material::asset::TextureSetting,
    },
    /// Let the user choose the model asset an entity shows; an entity
    /// without a model component gets one.
    PickModelAsset(EntityHandle),
    /// Let the user choose one of the project's scripts to add to an entity.
    PickScript(EntityHandle),
    /// Ask for the name of a new script.
    NewScriptDialog,
    /// Create a script file with this name in the project's scripts folder.
    NewScript(String),
    /// Make an entity show a model asset; an entity without a model
    /// component gets one.
    SetEntityModel { entity: EntityHandle, asset: AssetRef },
    /// Add an entity showing a model asset in front of the camera.
    SpawnModel(AssetRef),
    /// Remove a scatter layer from this entity's terrain.
    RemoveScatterModel(EntityHandle, usize),
    Delete(EntityHandle),
    Duplicate(EntityHandle),
    Focus(EntityHandle),
    /// Close the editor window.
    Quit,
    /// Show the editor's settings (File > Settings).
    SettingsDialog,
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
    /// Leave the scatter graph editor (unsaved edits stay in memory).
    CloseScatterEditor,
    SaveScatter,
    RevertScatter,
    /// Open the scatter canvas' "add node" menu.
    ScatterAddMenu,
    ScatterFit,
    ScatterUndo,
    ScatterRedo,
    ScatterDelete,
    ScatterCopy,
    ScatterPaste,
    /// Leave the material workspace (unsaved edits stay in memory).
    CloseMaterialEditor,
    SaveMaterial,
    RevertMaterial,
    MaterialAddMenu,
    MaterialFit,
    MaterialUndo,
    MaterialRedo,
    MaterialDelete,
    /// Ask for the name of a new asset of `kind` in `folder` (default: the
    /// kind's own folder).
    NewAssetDialog { kind: NewAsset, folder: Option<PathBuf> },
    /// Make a new asset of `kind` in `folder` and show it in the browser.
    NewAsset { kind: NewAsset, folder: Option<PathBuf>, name: Option<String> },
    /// Ask for the name of a new folder inside this one.
    NewFolderDialog(PathBuf),
    NewFolder { parent: PathBuf, name: String },
    RenameAssetDialog(AssetId),
    /// Rename an asset: display name, source file and sidecar.
    RenameAsset { id: AssetId, name: String },
    RenameFolderDialog(PathBuf),
    RenameFolder { path: PathBuf, name: String },
    /// Copy an asset next to it under a new id.
    DuplicateAsset(AssetId),
    /// Ask before deleting an asset, naming what refers to it.
    DeleteAssetDialog(AssetId),
    /// Move an asset's source file and sidecar to the trash.
    DeleteAsset(AssetId),
    DeleteFolderDialog(PathBuf),
    /// Move a folder with all its assets to the trash.
    DeleteFolder(PathBuf),
    /// Add a parameter to the open material graph.
    MaterialAddParam,
    MaterialRemoveParam(String),
    /// Ask which model a scatter graph's `Spawn mesh` node plants.
    PickScatterModel(ferrite::terrain::scatter::graph::NodeId),
    SetScatterModel {
        node: ferrite::terrain::scatter::graph::NodeId,
        asset: AssetRef,
    },
}

/// What the MCP bridge asks the asset browser to show.
#[derive(Clone, Debug, Default)]
pub struct BrowserRequest {
    pub category: Option<String>,
    pub folder: Option<PathBuf>,
    pub search: Option<String>,
    pub asset: Option<AssetId>,
    pub menu: bool,
    pub expand: Option<bool>,
}

/// An agent at the controls (see `BridgeRequest::AgentControl`).
pub struct AgentControl {
    pub note: Option<String>,
    pub since: std::time::Instant,
    /// The agent's last request: control it gave no word of for
    /// `AGENT_TIMEOUT` ends by itself (the agent may have stopped).
    pub last_seen: std::time::Instant,
}

/// See `AgentControl::last_seen`.
pub const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub struct EditorState {
    /// Selected entity id, shared with the hierarchy tree (`NO_SELECTION`
    /// when empty).
    pub selection: DataSource<u64>,
    pub mode: Mode,
    pub workspace: Workspace,
    /// Visibility flags for the panels of each workspace.
    pub scene_panels: DataSource<bool>,
    pub anim_panels: DataSource<bool>,
    pub asset_panels: DataSource<bool>,
    pub scatter_panels: DataSource<bool>,
    pub material_panels: DataSource<bool>,
    /// The asset browser is part of the scene and the asset workspace.
    pub browser_panel: DataSource<bool>,
    /// The asset open in the asset workspace.
    pub asset_edit: Option<asset_editor::AssetEdit>,
    pub anim: anim::AnimEditor,
    /// The scatter graph open in the scatter graph workspace.
    pub scatter: Option<scatter::ScatterEdit>,
    /// The material or material graph open in the material workspace.
    pub material: Option<material::MaterialEdit>,
    /// Asks the material graph canvas to open its add menu, or to fit.
    pub material_open_menu: bool,
    pub material_fit: bool,
    /// Asks the scatter canvas to open its add menu, or to fit the view.
    pub scatter_open_menu: bool,
    pub scatter_fit: bool,
    /// The scatter preview shows the world in 3D (else the map of points).
    pub scatter_preview_3d: bool,
    /// Whether the scatter preview shows the points (in 3D and on the map).
    pub scatter_show_points: DataSource<bool>,
    /// Where the editor camera is (x, z), for the scatter preview's map.
    pub scatter_camera: Option<[f32; 2]>,
    /// Graph-space point at the centre of the node canvas (new states go
    /// there).
    pub anim_canvas_center: Option<(f32, f32)>,
    pub gizmo: GizmoMode,
    /// Name of the open project, for the title.
    pub project_name: String,
    pub scene_path: Option<PathBuf>,
    pub dirty: bool,
    /// Counts the times the scene was replaced (opened, new, play mode
    /// stopped), so panels bound to its entities rebuild.
    pub scene_generation: u64,
    status: Option<(String, Instant)>,
    /// The latest status message, kept after it fades from the screen (for
    /// tools asking what happened, e.g. over the MCP bridge).
    last_status: Option<String>,
    /// An agent at the controls, shown in the toolbar and around the
    /// viewport.
    pub agent: Option<AgentControl>,
    /// What the last script build or load said.
    pub scripts_message: Option<String>,
    pub commands: Vec<Command>,
    pub layout: layout::Layout,
    /// Sizes the user dragged the panels to.
    pub panel_sizes: layout::PanelSizes,
    pub show_widgets: DataSource<bool>,
    pub show_stats: DataSource<bool>,
    /// Which sections of the Performance panel are open (in
    /// `stats::SECTIONS`'s order), kept in the project file.
    pub stats_open: Vec<DataSource<bool>>,
    /// Open/closed per group of graphics settings, by group id.
    pub graphics_groups_open: std::collections::BTreeMap<String, DataSource<bool>>,
    /// The switches of the Performance panel, shared with the MCP bridge.
    pub debug: stats::DebugSwitches,
    /// Asset browser below the viewport.
    pub show_assets: DataSource<bool>,
    /// Log console below the viewport (beside the asset browser).
    pub show_console: DataSource<bool>,
    /// The console's filters, kept in the project file.
    pub console: console::ConsoleFilter,
    /// What is selected in the asset browser.
    pub asset_selection: Option<AssetRef>,
    /// An asset the browser should show and select next (just made,
    /// renamed or opened from elsewhere).
    pub asset_reveal: Option<AssetId>,
    /// A folder the browser should open next.
    pub asset_folder_reveal: Option<PathBuf>,
    /// A wish for the browser's view from the MCP bridge.
    pub asset_browser_request: Option<BrowserRequest>,
    /// The browser's category key and open folder (below the asset root),
    /// kept in the project file.
    pub asset_view: (String, String),
    /// The asset pressed or being dragged in the asset browser.
    pub drag: Option<drag::AssetDrag>,
    /// Where an asset can be dropped this frame; panels register their
    /// targets on every update.
    pub drop_targets: Vec<drag::DropTarget>,
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
            asset_panels: DataSource::new(false),
            scatter_panels: DataSource::new(false),
            material_panels: DataSource::new(false),
            browser_panel: DataSource::new(true),
            asset_edit: None,
            anim: anim::AnimEditor::new(),
            scatter: None,
            material: None,
            material_open_menu: false,
            material_fit: false,
            scatter_open_menu: false,
            scatter_fit: false,
            scatter_preview_3d: true,
            scatter_show_points: DataSource::new(true),
            scatter_camera: None,
            anim_canvas_center: None,
            gizmo: GizmoMode::Translate,
            project_name: String::new(),
            scene_path: None,
            dirty: false,
            scene_generation: 0,
            status: None,
            last_status: None,
            agent: None,
            scripts_message: None,
            commands: Vec::new(),
            layout: layout::Layout::compute(
                1280.0,
                720.0,
                layout::RightColumn::Panels {
                    stats: true,
                    widgets: false,
                },
                true,
                true,
                &layout::PanelSizes::default(),
            ),
            panel_sizes: layout::PanelSizes::default(),
            show_widgets: DataSource::new(false),
            show_stats: DataSource::new(true),
            stats_open: stats::section_titles().map(|_| DataSource::new(true)).collect(),
            graphics_groups_open: Default::default(),
            debug: stats::DebugSwitches::new(),
            show_assets: DataSource::new(true),
            show_console: DataSource::new(true),
            console: console::ConsoleFilter::default(),
            asset_selection: None,
            asset_reveal: None,
            asset_folder_reveal: None,
            asset_view: (String::new(), String::new()),
            asset_browser_request: None,
            drag: None,
            drop_targets: Vec::new(),
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
        let message = message.into();
        self.last_status = Some(message.clone());
        self.status = Some((message, Instant::now()));
    }

    /// The latest status message, also after it fades from the screen.
    pub fn last_status(&self) -> Option<&str> {
        self.last_status.as_deref()
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
            .unwrap_or_else(|| tr!("editor.untitled").into());
        let name = if self.project_name.is_empty() {
            name
        } else {
            format!("{} - {name}", self.project_name)
        };
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
