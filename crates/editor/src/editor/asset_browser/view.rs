//! What the browser shows, apart from how: the categories, and the cards of
//! a folder or a search in one of them. Kept free of drawing so it can be
//! tested.

use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};

use ferrite::core::{
    assets::{AssetDatabase, AssetEntry, AssetId, AssetKind, AssetRef, NewAsset},
    renderer::text::icons,
};

/// A group of asset kinds in the side bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    All,
    Models,
    Textures,
    Materials,
    MaterialGraphs,
    Animations,
    ScatterGraphs,
    /// The project's scenes (`scenes/`, beside the assets).
    Scenes,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::All,
        Category::Models,
        Category::Textures,
        Category::Materials,
        Category::MaterialGraphs,
        Category::Animations,
        Category::ScatterGraphs,
        Category::Scenes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::All => tr!("asset_browser.category.all"),
            Category::Models => tr!("asset_browser.category.models"),
            Category::Textures => tr!("asset_browser.category.textures"),
            Category::Materials => tr!("asset_browser.category.materials"),
            Category::MaterialGraphs => tr!("asset_browser.category.material_graphs"),
            Category::Animations => tr!("asset_browser.category.animations"),
            Category::ScatterGraphs => tr!("asset_browser.category.scatter_graphs"),
            Category::Scenes => tr!("asset_browser.category.scenes"),
        }
    }

    /// Name kept in the project file.
    pub fn key(self) -> &'static str {
        match self {
            Category::All => "all",
            Category::Models => "models",
            Category::Textures => "textures",
            Category::Materials => "materials",
            Category::MaterialGraphs => "material_graphs",
            Category::Animations => "animations",
            Category::ScatterGraphs => "scatter_graphs",
            Category::Scenes => "scenes",
        }
    }

    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::All => icons::LAYERS,
            Category::Models => icons::PACKAGE,
            Category::Textures => icons::LAYOUT_GRID,
            Category::Materials => icons::SPARKLES,
            Category::MaterialGraphs => icons::COMPONENT,
            Category::Animations => icons::ACTIVITY,
            Category::ScatterGraphs => icons::LIST_TREE,
            Category::Scenes => icons::MOUNTAIN,
        }
    }

    pub fn contains(self, kind: &AssetKind) -> bool {
        match self {
            Category::All => true,
            Category::Models => matches!(kind, AssetKind::Model(_)),
            Category::Textures => matches!(kind, AssetKind::Texture(_)),
            Category::Materials => matches!(kind, AssetKind::Material(_)),
            Category::MaterialGraphs => matches!(kind, AssetKind::MaterialGraph(_)),
            Category::Animations => matches!(kind, AssetKind::Animation(_)),
            Category::ScatterGraphs => matches!(kind, AssetKind::ScatterGraph(_)),
            Category::Scenes => false,
        }
    }

    /// The kind of asset the "New" menu offers first in this category.
    pub fn new_kind(self) -> Option<NewAsset> {
        match self {
            Category::Materials => Some(NewAsset::Material),
            Category::MaterialGraphs => Some(NewAsset::MaterialGraph),
            Category::ScatterGraphs => Some(NewAsset::ScatterGraph),
            _ => None,
        }
    }
}

/// How many assets (scenes) each category holds, in `Category::ALL`'s
/// order.
pub fn counts(database: &AssetDatabase, scenes: &Scenes) -> Vec<usize> {
    Category::ALL
        .iter()
        .map(|c| match c {
            Category::Scenes => scenes.files.len(),
            _ => database.iter().filter(|e| c.contains(&e.meta.kind)).count(),
        })
        .collect()
}

/// The project's scene files: `scenes/` beside the asset root, and every
/// `.ron` in it (and below), sorted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scenes {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

impl Scenes {
    /// The scenes of the project whose assets `database` holds.
    pub fn of(database: &AssetDatabase) -> Self {
        let project = database.project_root();
        let dir = if project == Path::new(".") { PathBuf::from("scenes") } else { project.join("scenes") };
        let mut files = Vec::new();
        let mut pending = vec![dir.clone()];
        while let Some(folder) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            for path in entries.flatten().map(|e| e.path()) {
                let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
                if hidden {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == "ron") {
                    files.push(path);
                }
            }
        }
        files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        Self { dir, files }
    }

    /// Whether `folder` is the scenes' folder (or one below it).
    pub fn holds(&self, folder: &Path) -> bool {
        !self.dir.as_os_str().is_empty() && folder.starts_with(&self.dir)
    }

    fn card(&self, file: &Path) -> Card {
        let place = file.parent().and_then(|p| p.strip_prefix(&self.dir).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        Card {
            kind: CardKind::Scene(file.to_path_buf()),
            label: file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            detail: if place.is_empty() { tr!("asset_browser.kind.scene").to_string() } else { place },
            icon: icons::MOUNTAIN,
            parts: 0,
            expanded: false,
        }
    }
}

/// What a card stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum CardKind {
    Folder(PathBuf),
    Asset(AssetId),
    /// A variant of a model pack or a clip of an animation file.
    Part(AssetRef),
    /// A scene file (opened in the editor, not an asset).
    Scene(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub kind: CardKind,
    pub label: String,
    /// Second line: the kind, the number of parts, or (in a search) the
    /// folder.
    pub detail: String,
    pub icon: &'static str,
    /// Parts it can unfold into (variants, clips); 0 for none.
    pub parts: usize,
    pub expanded: bool,
}

impl Card {
    /// The asset (or part) it shows, if it is not a folder.
    pub fn asset(&self) -> Option<AssetRef> {
        match &self.kind {
            CardKind::Asset(id) => Some(AssetRef::whole(*id)),
            CardKind::Part(asset) => Some(asset.clone()),
            CardKind::Folder(_) | CardKind::Scene(_) => None,
        }
    }

    /// Stable key for selection and thumbnails.
    pub fn key(&self) -> String {
        match &self.kind {
            CardKind::Folder(path) => format!("folder:{}", path.display()),
            CardKind::Asset(id) => format!("asset:{id}"),
            CardKind::Part(asset) => format!("part:{}:{}", asset.id, asset.part.as_deref().unwrap_or("")),
            CardKind::Scene(path) => format!("scene:{}", path.display()),
        }
    }
}

/// The icon of an asset kind (a pack of several models has its own).
pub fn kind_icon(kind: &AssetKind) -> &'static str {
    match kind {
        AssetKind::Model(model) if model.variants.len() > 1 => icons::BOXES,
        AssetKind::Model(_) => icons::PACKAGE,
        AssetKind::Texture(_) => icons::LAYOUT_GRID,
        AssetKind::Animation(_) => icons::ACTIVITY,
        AssetKind::ScatterGraph(_) => icons::LIST_TREE,
        AssetKind::Material(_) => icons::SPARKLES,
        AssetKind::MaterialGraph(_) => icons::COMPONENT,
    }
}

/// The parts an asset unfolds into: the variants of a pack, the clips of
/// an animation file with several.
pub fn parts_of(entry: &AssetEntry) -> Vec<String> {
    match &entry.meta.kind {
        AssetKind::Model(model) if model.variants.len() > 1 => model.variants.iter().map(|v| v.name.clone()).collect(),
        AssetKind::Animation(animation) if animation.clips.len() > 1 => {
            animation.clips.iter().map(|c| c.name.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// "3 assets", "1 clip": `noun` is the unit's id (asset, clip, variant,
/// lod, material).
pub fn count_of(n: usize, noun: &str) -> String {
    match (noun, n == 1) {
        ("asset", true) => tr!("asset_browser.count.asset.one", n = n),
        ("asset", false) => tr!("asset_browser.count.asset.other", n = n),
        ("clip", true) => tr!("asset_browser.count.clip.one", n = n),
        ("clip", false) => tr!("asset_browser.count.clip.other", n = n),
        ("variant", true) => tr!("asset_browser.count.variant.one", n = n),
        ("variant", false) => tr!("asset_browser.count.variant.other", n = n),
        ("lod", true) => tr!("asset_browser.count.lod.one", n = n),
        ("lod", false) => tr!("asset_browser.count.lod.other", n = n),
        ("material", true) => tr!("asset_browser.count.material.one", n = n),
        ("scene", true) => tr!("asset_browser.count.scene.one", n = n),
        ("scene", false) => tr!("asset_browser.count.scene.other", n = n),
        _ => tr!("asset_browser.count.material.other", n = n),
    }
}

/// The shown name of an asset kind.
pub fn kind_label(kind: &AssetKind) -> &'static str {
    match kind {
        AssetKind::Model(_) => tr!("asset_browser.kind.model"),
        AssetKind::Texture(_) => tr!("asset_browser.kind.texture"),
        AssetKind::Animation(_) => tr!("asset_browser.kind.animation"),
        AssetKind::ScatterGraph(_) => tr!("asset_browser.kind.scatter_graph"),
        AssetKind::Material(_) => tr!("asset_browser.kind.material"),
        AssetKind::MaterialGraph(_) => tr!("asset_browser.kind.material_graph"),
    }
}

/// Folders directly inside `folder` worth showing in `category`: every
/// folder on disk for "All" (empty ones too), else those holding (at any
/// depth) an asset of the category. Hidden folders never.
fn subfolders(database: &AssetDatabase, category: Category, folder: &Path) -> Vec<PathBuf> {
    let mut found: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in database.iter().filter(|e| category.contains(&e.meta.kind)) {
        if let Ok(rest) = entry.dir.strip_prefix(folder) {
            if let Some(first) = rest.components().next() {
                found.insert(folder.join(first.as_os_str()));
            }
        }
    }
    if category == Category::All {
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
                if path.is_dir() && !hidden {
                    found.insert(path);
                }
            }
        }
    }
    let mut folders: Vec<PathBuf> = found.into_iter().collect();
    folders.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    folders
}

fn sort_entries(entries: &mut [&AssetEntry]) {
    entries.sort_by(|a, b| {
        a.meta
            .name
            .to_lowercase()
            .cmp(&b.meta.name.to_lowercase())
            .then_with(|| a.meta.id.cmp(&b.meta.id))
    });
}

fn asset_card(entry: &AssetEntry, detail: String, expanded: &HashSet<AssetId>) -> Vec<Card> {
    let parts = parts_of(entry);
    let open = !parts.is_empty() && expanded.contains(&entry.meta.id);
    let detail = match (&entry.meta.kind, parts.len()) {
        (_, 0) => detail,
        (AssetKind::Animation(_), n) => count_of(n, "clip"),
        (_, n) => count_of(n, "variant"),
    };
    let mut cards = vec![Card {
        kind: CardKind::Asset(entry.meta.id),
        label: entry.meta.name.clone(),
        detail,
        icon: kind_icon(&entry.meta.kind),
        parts: parts.len(),
        expanded: open,
    }];
    if open {
        let animation = matches!(entry.meta.kind, AssetKind::Animation(_));
        let icon = if animation { icons::ACTIVITY } else { icons::BOX };
        cards.extend(parts.into_iter().map(|part| Card {
            kind: CardKind::Part(AssetRef::part(entry.meta.id, &part)),
            label: part,
            detail: if animation {
                tr!("asset_browser.clip_of", name = entry.meta.name)
            } else {
                tr!("asset_browser.variant_of", name = entry.meta.name)
            },
            icon,
            parts: 0,
            expanded: false,
        }));
    }
    cards
}

/// The cards of `folder` in `category`: folders first, then assets, each
/// by name; an expanded asset followed by its parts. With a `search`, the
/// assets of the category anywhere below the asset root whose name or file
/// name contains it (case blind) instead, each with its folder as detail.
///
/// Scenes: the "Scenes" category lists every scene; "All" shows the
/// scenes' folder at the asset root and the scenes in it; a search in
/// either finds scenes by name too.
pub fn cards(
    database: &AssetDatabase,
    category: Category,
    folder: &Path,
    search: &str,
    expanded: &HashSet<AssetId>,
    scenes: &Scenes,
) -> Vec<Card> {
    let search = search.trim().to_lowercase();
    let mut cards = Vec::new();
    let scene_matches = |file: &&PathBuf| {
        file.file_stem().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&search))
    };
    if category == Category::Scenes {
        return scenes.files.iter().filter(|f| search.is_empty() || scene_matches(f)).map(|f| scenes.card(f)).collect();
    }
    if category == Category::All && search.is_empty() && scenes.holds(folder) {
        return scenes.files.iter().filter(|f| f.parent() == Some(folder)).map(|f| scenes.card(f)).collect();
    }
    if !search.is_empty() {
        if category == Category::All {
            cards.extend(scenes.files.iter().filter(scene_matches).map(|f| scenes.card(f)));
        }
        let mut hits: Vec<&AssetEntry> = database
            .iter()
            .filter(|e| category.contains(&e.meta.kind))
            .filter(|e| e.meta.name.to_lowercase().contains(&search) || e.meta.source.to_lowercase().contains(&search))
            .collect();
        sort_entries(&mut hits);
        for entry in hits {
            let place = entry.dir.strip_prefix(database.root()).unwrap_or(&entry.dir).to_string_lossy().replace('\\', "/");
            let detail = if place.is_empty() { tr!("asset_browser.root").to_string() } else { place };
            cards.extend(asset_card(entry, detail, expanded));
        }
        return cards;
    }
    // The scenes' folder, beside the assets, shown at their root.
    if category == Category::All && folder == database.root() && scenes.dir.is_dir() {
        cards.push(Card {
            label: scenes.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            detail: count_of(scenes.files.len(), "scene"),
            kind: CardKind::Folder(scenes.dir.clone()),
            icon: icons::FOLDER_OPEN,
            parts: 0,
            expanded: false,
        });
    }
    for path in subfolders(database, category, folder) {
        let inside = database
            .iter()
            .filter(|e| category.contains(&e.meta.kind) && e.dir.starts_with(&path))
            .count();
        cards.push(Card {
            label: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            detail: if inside == 0 { tr!("asset_browser.empty_folder").into() } else { count_of(inside, "asset") },
            kind: CardKind::Folder(path),
            icon: icons::FOLDER_OPEN,
            parts: 0,
            expanded: false,
        });
    }
    let mut here: Vec<&AssetEntry> = database
        .iter()
        .filter(|e| category.contains(&e.meta.kind) && e.dir == folder)
        .collect();
    sort_entries(&mut here);
    for entry in here {
        cards.extend(asset_card(entry, kind_label(&entry.meta.kind).to_string(), expanded));
    }
    cards
}

/// The folders from the asset root down to `folder`, for the path bar:
/// `(name, path)`, the root named "assets".
pub fn crumbs(root: &Path, folder: &Path, scenes: &Scenes) -> Vec<(String, PathBuf)> {
    let mut crumbs = vec![(tr!("asset_browser.root").to_string(), root.to_path_buf())];
    // The scenes lie beside the assets: reached from their root.
    if let Some(rest) = scenes.holds(folder).then(|| folder.strip_prefix(&scenes.dir).ok()).flatten() {
        let mut path = scenes.dir.clone();
        crumbs.push((scenes.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path.clone()));
        for part in rest.components() {
            path.push(part.as_os_str());
            crumbs.push((part.as_os_str().to_string_lossy().into_owned(), path.clone()));
        }
        return crumbs;
    }
    if let Ok(rest) = folder.strip_prefix(root) {
        let mut path = root.to_path_buf();
        for part in rest.components() {
            path.push(part.as_os_str());
            crumbs.push((part.as_os_str().to_string_lossy().into_owned(), path.clone()));
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite::core::assets::{
        AssetMeta, MaterialAssetSettings, ModelSettings, TextureSettings, VariantInfo, ASSET_META_VERSION,
    };

    fn meta(name: &str, source: &str, kind: AssetKind) -> AssetMeta {
        AssetMeta { version: ASSET_META_VERSION, id: AssetId::new(), name: name.into(), source: source.into(), kind }
    }

    fn model(variants: &[&str]) -> AssetKind {
        AssetKind::Model(ModelSettings {
            variants: variants.iter().map(|n| VariantInfo { name: n.to_string(), lods: vec!["lod0".into()] }).collect(),
            ..Default::default()
        })
    }

    struct Fixture {
        root: PathBuf,
        database: AssetDatabase,
        pack: AssetId,
        scenes: Scenes,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.root.parent().unwrap());
        }
    }

    fn fixture() -> Fixture {
        let project = std::env::temp_dir().join(format!("ferrite-browser-view-{}-{}", std::process::id(), AssetId::new()));
        let root = project.join("assets");
        let trees = root.join("models").join("trees");
        let textures = root.join("textures");
        let materials = root.join("materials");
        for dir in [&trees, &textures, &materials, &root.join("empty"), &root.join(".thumbnails")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let mut database = AssetDatabase::new(&root);
        let pack = database.register(meta("Tree pack", "pack.fbx", model(&["Oak", "Pine"])), &trees).unwrap();
        database.register(meta("birch", "birch.gltf", model(&["birch"])), &trees).unwrap();
        database
            .register(meta("Bark", "bark.png", AssetKind::Texture(TextureSettings::default())), &textures)
            .unwrap();
        database
            .register(meta("Bark", "Bark.material.ron", AssetKind::Material(MaterialAssetSettings {})), &materials)
            .unwrap();
        database
            .register(meta("Stone", "Stone.material.ron", AssetKind::Material(MaterialAssetSettings {})), &root)
            .unwrap();
        let scenes_dir = project.join("scenes");
        std::fs::create_dir_all(scenes_dir.join("levels")).unwrap();
        for file in ["menu.ron", "levels/cave.ron", "notes.txt"] {
            std::fs::write(scenes_dir.join(file), "()").unwrap();
        }
        let scenes = Scenes::of(&database);
        Fixture { root, database, pack, scenes }
    }

    fn labels(cards: &[Card]) -> Vec<(&str, &str)> {
        cards.iter().map(|c| (c.label.as_str(), c.detail.as_str())).collect()
    }

    #[test]
    fn folders_come_first_and_all_shows_empty_ones_but_not_hidden_ones() {
        let f = fixture();
        let cards = cards(&f.database, Category::All, &f.root, "", &HashSet::new(), &f.scenes);
        assert_eq!(
            labels(&cards),
            vec![
                ("scenes", "2 scenes"),
                ("empty", "empty"),
                ("materials", "1 asset"),
                ("models", "2 assets"),
                ("textures", "1 asset"),
                ("Stone", "Material"),
            ]
        );
    }

    #[test]
    fn a_category_shows_only_folders_holding_its_kind() {
        let f = fixture();
        let cards = cards(&f.database, Category::Materials, &f.root, "", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&cards), vec![("materials", "1 asset"), ("Stone", "Material")]);
        assert_eq!(counts(&f.database, &f.scenes), vec![5, 2, 1, 2, 0, 0, 0, 2]);
    }

    #[test]
    fn packs_unfold_into_their_variants() {
        let f = fixture();
        let trees = f.root.join("models").join("trees");
        let folded = cards(&f.database, Category::Models, &trees, "", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&folded), vec![("birch", "Model"), ("Tree pack", "2 variants")]);
        assert_eq!(folded[1].parts, 2);
        let open = cards(&f.database, Category::Models, &trees, "", &HashSet::from([f.pack]), &f.scenes);
        assert_eq!(
            labels(&open),
            vec![
                ("birch", "Model"),
                ("Tree pack", "2 variants"),
                ("Oak", "Variant of Tree pack"),
                ("Pine", "Variant of Tree pack"),
            ]
        );
        assert_eq!(open[2].asset(), Some(AssetRef::part(f.pack, "Oak")));
    }

    #[test]
    fn a_search_lists_matches_of_the_category_from_everywhere() {
        let f = fixture();
        let all = cards(&f.database, Category::All, &f.root.join("empty"), "BARK", &HashSet::new(), &f.scenes);
        // Two assets named "Bark": their order follows their (random) ids.
        let mut found = labels(&all);
        found.sort();
        assert_eq!(found, vec![("Bark", "materials"), ("Bark", "textures")]);
        let materials = cards(&f.database, Category::Materials, &f.root, "bark", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&materials), vec![("Bark", "materials")]);
        // The file name counts too.
        let by_file = cards(&f.database, Category::All, &f.root, "pack.fbx", &HashSet::new(), &f.scenes);
        assert_eq!(by_file.len(), 1);
    }

    #[test]
    fn scenes_show_in_their_category_their_folder_and_searches() {
        let f = fixture();
        assert_eq!(f.scenes.files.len(), 2);
        let all = cards(&f.database, Category::Scenes, &f.root, "", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&all), vec![("cave", "levels"), ("menu", "Scene")]);
        assert!(matches!(&all[1].kind, CardKind::Scene(p) if p.ends_with("scenes/menu.ron")));
        // At the asset root, the scenes' folder comes first.
        let root = cards(&f.database, Category::All, &f.root, "", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&root)[0], ("scenes", "2 scenes"));
        // In it, its own scenes (and its subfolder's by searching).
        let inside = cards(&f.database, Category::All, &f.scenes.dir, "", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&inside), vec![("menu", "Scene")]);
        let found = cards(&f.database, Category::All, &f.root, "cav", &HashSet::new(), &f.scenes);
        assert_eq!(labels(&found), vec![("cave", "levels")]);
        let crumbs = crumbs(&f.root, &f.scenes.dir.join("levels"), &f.scenes);
        let names: Vec<&str> = crumbs.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["assets", "scenes", "levels"]);
    }

    #[test]
    fn crumbs_lead_from_the_root() {
        let root = PathBuf::from("assets");
        let crumbs = crumbs(&root, &root.join("models").join("trees"), &Scenes::default());
        let names: Vec<&str> = crumbs.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["assets", "models", "trees"]);
        assert_eq!(crumbs[2].1, PathBuf::from("assets/models/trees"));
    }
}
