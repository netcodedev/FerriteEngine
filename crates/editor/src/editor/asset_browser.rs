//! Docked panel listing the project's assets by folder. A model asset with
//! several variants (a tree pack) expands into them.

use std::collections::HashMap;

use ferrite::core::{
    assets::{self, AssetDatabase, AssetEntry, AssetId, AssetKind, AssetRef},
    primitives::{Offset, Position, Size},
    renderer::text::icons,
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        label::Label,
        panel::Panel,
        tree::{TreeItem, TreeView, NO_SELECTION},
    },
    theme, UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::{send, Command, Shared};

const ACTIONS_HEIGHT: f32 = 30.0;

/// What a row of the tree stands for.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Folder,
    Asset(AssetId),
    Variant(AssetRef),
}

pub struct AssetBrowser {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    tree: TreeView,
    import: Button,
    refresh: Button,
    details: Label,
    selection: DataSource<u64>,
    rows: HashMap<u64, Row>,
    /// Database change counter the tree was last built for.
    built_for: Option<u64>,
    last_selection: u64,
}

impl AssetBrowser {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Assets".into(), Position::default(), Size::new(600.0, 200.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);

        let selection = DataSource::new(NO_SELECTION);
        let tree = TreeView::new(Position::default(), Size::new(590.0, 120.0), selection.clone());

        let import_state = state.clone();
        let mut import = Button::new(
            Position::default(),
            Size::new(10.0, 26.0),
            Box::new(move |_| send(&import_state, Command::ImportAssets)),
        );
        import.set_style(ButtonStyle::Secondary);
        import.set_icon(icons::PLUS);
        import.set_label("Import");
        import.set_tooltip("Import models or textures into the project");
        import.fit(26.0);

        let refresh_state = state.clone();
        let mut refresh = Button::new(
            Position::default(),
            Size::new(26.0, 26.0),
            Box::new(move |_| send(&refresh_state, Command::RefreshAssets)),
        );
        refresh.set_style(ButtonStyle::Ghost);
        refresh.set_icon(icons::ROTATE_CW);
        refresh.set_tooltip("Scan the assets folder again");

        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            tree,
            import,
            refresh,
            details: Label::muted("", theme::FONT_SIZE_SMALL + 0.5),
            selection,
            rows: HashMap::new(),
            built_for: None,
            last_selection: NO_SELECTION,
        }
    }

    fn rebuild(&mut self, database: &AssetDatabase) {
        let (items, rows) = build_items(database);
        self.tree.set_items(items);
        self.rows = rows;
    }

    fn details_text(&self, database: &AssetDatabase) -> String {
        if database.is_empty() {
            return "No assets yet. Import a model or texture to get started.".into();
        }
        match self.rows.get(&self.selection.read()) {
            Some(Row::Asset(id)) => database.get(*id).map(describe).unwrap_or_default(),
            Some(Row::Variant(asset)) => database
                .get(asset.id)
                .and_then(|entry| {
                    let part = asset.part.as_deref()?;
                    let variant = entry.meta.as_model()?.variant(part)?;
                    Some(format!(
                        "{part}  ·  variant of {}  ·  {}",
                        entry.meta.name,
                        count(variant.lods.len(), "LOD")
                    ))
                })
                .unwrap_or_default(),
            _ => format!("{}", count(database.len(), "asset")),
        }
    }

    fn widgets_mut(&mut self) -> [&mut dyn UIElement; 3] {
        [&mut self.import, &mut self.refresh, &mut self.tree]
    }

    fn visible(&self) -> bool {
        self.state.borrow().layout.assets.h > 0.0
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// One line about an asset for the details row.
fn describe(entry: &AssetEntry) -> String {
    let source = entry.source_path().to_string_lossy().replace('\\', "/");
    match &entry.meta.kind {
        AssetKind::Model(model) => format!(
            "{}  ·  Model  ·  {}  ·  {}  ·  {source}",
            entry.meta.name,
            count(model.variants.len(), "variant"),
            count(model.materials.len(), "material"),
        ),
        AssetKind::Texture(_) => format!("{}  ·  Texture  ·  {source}", entry.meta.name),
    }
}

/// FNV-1a, for row ids of folders and variants.
fn row_id(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash.max(1)
}

/// Folder of an asset as path components below the asset root.
fn folder_of(entry: &AssetEntry, database: &AssetDatabase) -> Vec<String> {
    entry
        .dir
        .strip_prefix(database.root())
        .unwrap_or(&entry.dir)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

/// The tree rows in depth-first order: folders, the assets in them, and
/// below a model asset with more than one variant its variants.
fn build_items(database: &AssetDatabase) -> (Vec<TreeItem>, HashMap<u64, Row>) {
    let mut entries: Vec<(Vec<String>, &AssetEntry)> = database
        .iter()
        .map(|entry| (folder_of(entry, database), entry))
        .collect();
    entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.meta.name.to_lowercase().cmp(&b.1.meta.name.to_lowercase()))
            .then_with(|| a.1.meta.id.cmp(&b.1.meta.id))
    });

    let mut items = Vec::new();
    let mut rows = HashMap::new();
    let mut open: Vec<String> = Vec::new();
    for (folder, entry) in entries {
        let shared = open
            .iter()
            .zip(folder.iter())
            .take_while(|(a, b)| a == b)
            .count();
        open.truncate(shared);
        for name in &folder[shared..] {
            open.push(name.clone());
            let id = row_id(&format!("folder:{}", open.join("/")));
            items.push(TreeItem {
                id,
                label: name.clone(),
                depth: open.len() - 1,
                has_children: true,
                icon: Some(icons::FOLDER_OPEN),
            });
            rows.insert(id, Row::Folder);
        }

        let depth = open.len();
        let variants: &[_] = match &entry.meta.kind {
            AssetKind::Model(model) if model.variants.len() > 1 => &model.variants,
            _ => &[],
        };
        let id = entry.meta.id.as_u64().max(1);
        items.push(TreeItem {
            id,
            label: entry.meta.name.clone(),
            depth,
            has_children: !variants.is_empty(),
            icon: Some(match &entry.meta.kind {
                AssetKind::Model(_) if !variants.is_empty() => icons::BOXES,
                AssetKind::Model(_) => icons::PACKAGE,
                AssetKind::Texture(_) => icons::LAYOUT_GRID,
            }),
        });
        rows.insert(id, Row::Asset(entry.meta.id));
        for variant in variants {
            let id = row_id(&format!("variant:{}:{}", entry.meta.id, variant.name));
            items.push(TreeItem {
                id,
                label: variant.name.clone(),
                depth: depth + 1,
                has_children: false,
                icon: Some(icons::BOX),
            });
            rows.insert(id, Row::Variant(AssetRef::part(entry.meta.id, variant.name.clone())));
        }
    }
    (items, rows)
}

impl UIElement for AssetBrowser {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.state.borrow().layout.assets;
        if rect.h <= 0.0 {
            return;
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let header = self.panel.header_height();
        let x = rect.x + 6.0;
        let ay = rect.y + header + 4.0;
        self.import.set_offset(Offset { x, y: ay });
        let after_import = x + self.import.get_size().width + 2.0;
        self.refresh.set_offset(Offset {
            x: after_import,
            y: ay,
        });
        self.details.set_offset(Offset {
            x: after_import + self.refresh.get_size().width + 10.0,
            y: ay + 5.0,
        });

        let tree_top = rect.y + header + ACTIONS_HEIGHT + 6.0;
        self.tree.set_position(Position::new(x, tree_top, 1.0));
        self.tree.set_size(Size::new(
            (rect.w - 12.0).max(10.0),
            (rect.y + rect.h - tree_top - 6.0).max(10.0),
        ));

        {
            // Short-lived: commands and loading code use the database too.
            let database = assets::database();
            let changes = database.changes();
            if self.built_for != Some(changes) {
                self.rebuild(&database);
                self.built_for = Some(changes);
            }
            self.details.set_text(self.details_text(&database));
        }

        let selection = self.selection.read();
        if selection != self.last_selection {
            self.last_selection = selection;
            let selected = match self.rows.get(&selection) {
                Some(Row::Asset(id)) => Some(AssetRef::whole(*id)),
                Some(Row::Variant(asset)) => Some(asset.clone()),
                _ => None,
            };
            self.state.borrow_mut().asset_selection = selected;
        }

        self.panel.update(scene);
        self.details.update(scene);
        for widget in self.widgets_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        if !self.visible() {
            return;
        }
        self.panel.render();
        self.tree.render();
        self.details.render();
        self.import.render();
        self.refresh.render();
    }

    fn render_overlay(&self) {
        if !self.visible() {
            return;
        }
        self.import.render_overlay();
        self.refresh.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if !self.visible() {
            return false;
        }
        let mut handled = false;
        for widget in self.widgets_mut() {
            if widget.handle_events(scene, window, glfw, event) {
                handled = true;
                if !matches!(event, WindowEvent::CursorPos(..)) {
                    return true;
                }
            }
        }
        if handled {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self
                .state
                .borrow()
                .layout
                .assets
                .contains(x as f32, y as f32);
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
        self.details.set_z_index(z_index + 2.0);
        for widget in self.widgets_mut() {
            widget.set_z_index(z_index + 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite::core::assets::{
        AssetMeta, ModelSettings, TextureSettings, VariantInfo, ASSET_META_VERSION,
    };

    fn meta(name: &str, source: &str, kind: AssetKind) -> AssetMeta {
        AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name: name.into(),
            source: source.into(),
            kind,
        }
    }

    fn model(variants: &[&str]) -> AssetKind {
        AssetKind::Model(ModelSettings {
            scale: 1.0,
            variants: variants
                .iter()
                .map(|name| VariantInfo {
                    name: name.to_string(),
                    lods: vec!["lod0".into()],
                })
                .collect(),
            materials: Vec::new(),
        })
    }

    #[test]
    fn rows_follow_folders_and_packs_expand_into_variants() {
        let root = std::env::temp_dir().join(format!(
            "ferrite-browser-{}-{}",
            std::process::id(),
            AssetId::new()
        ));
        let trees = root.join("models").join("trees");
        let textures = root.join("textures");
        std::fs::create_dir_all(&trees).unwrap();
        std::fs::create_dir_all(&textures).unwrap();

        let mut database = AssetDatabase::new(&root);
        let pack = database
            .register(meta("Tree pack", "pack.fbx", model(&["Oak", "Pine"])), &trees)
            .unwrap();
        database
            .register(meta("birch", "birch.gltf", model(&["birch"])), &trees)
            .unwrap();
        database
            .register(
                meta("Bark", "bark.png", AssetKind::Texture(TextureSettings::default())),
                &textures,
            )
            .unwrap();

        let (items, rows) = build_items(&database);
        let listing: Vec<(usize, &str, bool)> = items
            .iter()
            .map(|item| (item.depth, item.label.as_str(), item.has_children))
            .collect();
        assert_eq!(
            listing,
            vec![
                (0, "models", true),
                (1, "trees", true),
                (2, "birch", false),
                (2, "Tree pack", true),
                (3, "Oak", false),
                (3, "Pine", false),
                (0, "textures", true),
                (1, "Bark", false),
            ]
        );
        // Every row can be told apart and resolved.
        assert_eq!(rows.len(), items.len());
        let oak = items.iter().find(|item| item.label == "Oak").unwrap();
        assert_eq!(rows[&oak.id], Row::Variant(AssetRef::part(pack, "Oak")));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
