//! Docked panel showing the project's assets as cards: categories on the
//! left, the open folder (or a search) as cards on the right, folders
//! first, pictures where they help. A right click offers what can be done
//! with a card or the folder. See `docs/asset-browser.md`.

mod thumbnails;
mod view;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Instant,
};

use ferrite::core::{
    assets::{self, AssetDatabase, AssetEntry, AssetId, AssetKind, NewAsset},
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    clip,
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        context_menu::{ContextEntry, ContextMenu, ContextResult},
        input::Input,
        label::Label,
        panel::Panel,
    },
    theme, UIElement,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{
    drag::{AssetDrag, DragKind},
    send, Command, Shared,
};
use thumbnails::Thumbnails;
use view::{Card, CardKind, Category};

const SIDEBAR_WIDTH: f32 = 170.0;
const SIDE_ROW: f32 = 28.0;
const TOOLBAR_HEIGHT: f32 = 34.0;
const DETAILS_HEIGHT: f32 = 22.0;
const CARD_WIDTH: f32 = 108.0;
const CARD_HEIGHT: f32 = 142.0;
const THUMB: f32 = 92.0;
const GAP: f32 = 8.0;
const SEARCH_WIDTH: f32 = 190.0;

/// Two presses on the same card within this many seconds open it.
const DOUBLE_CLICK: f32 = 0.4;

/// What a line of the open context menu does.
#[derive(Clone, Debug)]
enum MenuAction {
    Open(Card),
    Rename(AssetId),
    RenameFolder(PathBuf),
    Duplicate(AssetId),
    CopyPath(PathBuf),
    Reveal(PathBuf),
    Delete(AssetId),
    DeleteFolder(PathBuf),
    New(NewAsset),
    NewFolder,
    Import,
    Rescan,
    /// A separator.
    Nothing,
}

/// One category of the side bar.
struct SideRow {
    category: Category,
    bg: Plane,
    icon: Text,
    label: Text,
    count: Text,
    region: Region,
}

/// The drawn parts of one card.
struct CardView {
    bg: Plane,
    thumb_bg: Plane,
    icon: Text,
    label: Text,
    detail: Text,
    chevron_bg: Plane,
    chevron: Text,
    chevron_region: Region,
    region: Region,
    /// Inside the grid's visible part.
    shown: bool,
}

impl CardView {
    fn new() -> Self {
        Self {
            bg: PlaneBuilder::new().color(theme::BG_CONTROL).border_radius_uniform(theme::RADIUS).border_thickness(1.0).build(),
            thumb_bg: PlaneBuilder::new().color(theme::BG_FIELD).border_radius_uniform(theme::RADIUS_SMALL).build(),
            icon: theme::icon(icons::BOX, 34.0, theme::TEXT_MUTED),
            label: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT),
            detail: theme::text("", theme::FONT_SIZE_SMALL - 1.0, theme::TEXT_DIM),
            chevron_bg: PlaneBuilder::new().color(theme::BG_MENU).border_radius_uniform(10.0).border_thickness(1.0).border_color(theme::BORDER_STRONG).build(),
            chevron: theme::icon(icons::CHEVRON_RIGHT, 14.0, theme::TEXT),
            chevron_region: Region::default(),
            region: Region::default(),
            shown: false,
        }
    }
}

/// `text` cut to fit `width` pixels, with an ellipsis where cut.
fn fit(text: &Text, content: &str, width: f32) -> String {
    if text.measure_str(content) <= width {
        return content.to_string();
    }
    let chars: Vec<char> = content.chars().collect();
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if text.measure_str(&candidate) <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    chars[..low].iter().collect::<String>() + "…"
}

pub struct AssetBrowser {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    sidebar: Plane,
    side_rows: Vec<SideRow>,
    up: Button,
    crumbs: Vec<(Text, Region, PathBuf)>,
    search: Input<String>,
    search_text: DataSource<String>,
    search_hint: Text,
    new_button: Button,
    import: Button,
    refresh: Button,
    grid: Plane,
    empty: Text,
    cards: Vec<Card>,
    views: Vec<CardView>,
    details: Label,
    menu: ContextMenu,
    menu_actions: Vec<MenuAction>,
    thumbs: Thumbnails,
    project: PathBuf,

    category: Category,
    folder: PathBuf,
    expanded: HashSet<AssetId>,
    /// `Card::key` of the selected card.
    selected: Option<String>,
    scroll: f32,
    /// Scroll the selected card into view on the next layout.
    scroll_to_selected: bool,
    hover: Option<usize>,
    /// What the cards were made for: database changes, category, folder,
    /// search, unfolded assets.
    built_for: Option<(u64, Category, PathBuf, String, Vec<AssetId>)>,
    details_for: Option<(Option<String>, u64, Category)>,
    counts: Vec<usize>,
    /// Restored from the project file once.
    restored: bool,
    /// Card index and time of the last press, to tell a double click.
    last_press: Option<(usize, Instant)>,
}

impl AssetBrowser {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Assets".into(), Position::default(), Size::new(600.0, 200.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);

        let mut up = Button::new(Position::default(), Size::new(26.0, 26.0), Box::new(|_| {}));
        up.set_style(ButtonStyle::Ghost);
        up.set_icon(icons::ARROW_LEFT);
        up.set_tooltip("Up to the folder above");

        let search_text = DataSource::new(String::new());
        let search = Input::new(Position::default(), Size::new(SEARCH_WIDTH, 24.0), String::new(), Some(search_text.clone()));

        let mut new_button = Button::new(Position::default(), Size::new(10.0, 26.0), Box::new(|_| {}));
        new_button.set_style(ButtonStyle::Secondary);
        new_button.set_icon(icons::PLUS);
        new_button.set_label("New");
        new_button.set_tooltip("Make a new folder, material, material graph or scatter graph here");
        new_button.fit(26.0);

        let import_state = state.clone();
        let mut import = Button::new(
            Position::default(),
            Size::new(10.0, 26.0),
            Box::new(move |_| send(&import_state, Command::ImportAssets)),
        );
        import.set_style(ButtonStyle::Ghost);
        import.set_icon(icons::ARROW_DOWN_TO_LINE);
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

        let side_rows = Category::ALL
            .iter()
            .map(|&category| SideRow {
                category,
                bg: PlaneBuilder::new().color(theme::TRANSPARENT).border_radius_uniform(theme::RADIUS_SMALL).build(),
                icon: theme::icon(category.icon(), theme::ICON_SIZE, theme::TEXT_MUTED),
                label: theme::text(category.label(), theme::FONT_SIZE, theme::TEXT),
                count: theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT_DIM),
                region: Region::default(),
            })
            .collect();

        let project = assets::database().project_root();
        let root = assets::database().root().to_path_buf();
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            sidebar: PlaneBuilder::new().color(theme::BG_HEADER).build(),
            side_rows,
            up,
            crumbs: Vec::new(),
            search,
            search_text,
            search_hint: theme::text("Search…", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_DIM),
            new_button,
            import,
            refresh,
            grid: PlaneBuilder::new().color(theme::TRANSPARENT).build(),
            empty: theme::text("", theme::FONT_SIZE, theme::TEXT_DIM),
            cards: Vec::new(),
            views: Vec::new(),
            details: Label::muted("", theme::FONT_SIZE_SMALL + 0.5),
            menu: ContextMenu::new(),
            menu_actions: Vec::new(),
            thumbs: Thumbnails::new(&project),
            project,
            category: Category::All,
            folder: root,
            expanded: HashSet::new(),
            selected: None,
            scroll: 0.0,
            scroll_to_selected: false,
            hover: None,
            built_for: None,
            details_for: None,
            counts: Vec::new(),
            restored: false,
            last_press: None,
        }
    }

    fn visible(&self) -> bool {
        self.state.borrow().layout.assets.h > 0.0
    }

    fn search_query(&self) -> String {
        self.search_text.read().trim().to_string()
    }

    /// The part of the panel the cards scroll in.
    fn grid_rect(&self) -> (f32, f32, f32, f32) {
        let rect = self.state.borrow().layout.assets;
        let header = self.panel.header_height();
        let x = rect.x + SIDEBAR_WIDTH + 8.0;
        let y = rect.y + header + TOOLBAR_HEIGHT + 4.0;
        let w = (rect.w - SIDEBAR_WIDTH - 14.0).max(10.0);
        let h = (rect.y + rect.h - DETAILS_HEIGHT - 4.0 - y).max(10.0);
        (x, y, w, h)
    }

    fn columns(&self) -> usize {
        let (_, _, w, _) = self.grid_rect();
        (((w + GAP) / (CARD_WIDTH + GAP)).floor() as usize).max(1)
    }

    fn content_height(&self) -> f32 {
        let rows = self.cards.len().div_ceil(self.columns());
        rows as f32 * (CARD_HEIGHT + GAP)
    }

    fn clamp_scroll(&mut self) {
        let (_, _, _, h) = self.grid_rect();
        let max = (self.content_height() - h).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }

    /// Open `folder` (leaving the search).
    fn open_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
        self.search_text.write(String::new());
        self.scroll = 0.0;
        self.save_view();
    }

    fn set_category(&mut self, category: Category) {
        if self.category != category {
            self.category = category;
            self.scroll = 0.0;
            self.save_view();
        }
    }

    /// Note the category and folder for the project file.
    fn save_view(&self) {
        let root = assets::database().root().to_path_buf();
        let folder = self.folder.strip_prefix(&root).unwrap_or(Path::new("")).to_string_lossy().replace('\\', "/");
        self.state.borrow_mut().asset_view = (self.category.key().to_string(), folder);
    }

    /// Show an asset: open its folder, pick a category that has it,
    /// select its card and scroll to it.
    fn reveal(&mut self, id: AssetId) {
        let (dir, kind) = {
            let database = assets::database();
            let Some(entry) = database.get(id) else {
                return;
            };
            (entry.dir.clone(), entry.meta.kind.clone())
        };
        if !self.category.contains(&kind) {
            self.category = Category::All;
        }
        self.open_folder(dir);
        self.selected = Some(format!("asset:{id}"));
        self.scroll_to_selected = true;
    }

    /// Do what the MCP bridge asked for.
    fn follow(&mut self, request: super::BrowserRequest) {
        self.menu.close();
        if let Some(category) = request.category.as_deref().and_then(Category::from_key) {
            self.set_category(category);
        }
        if let Some(folder) = request.folder {
            self.open_folder(folder);
        }
        if let Some(search) = request.search {
            self.search_text.write(search);
        }
        if let Some(id) = request.asset {
            {
                let database = assets::database();
                self.rebuild(&database);
            }
            self.reveal(id);
        }
        {
            let database = assets::database();
            self.rebuild(&database);
        }
        if let Some(index) = self.selected.as_ref().and_then(|k| self.cards.iter().position(|c| &c.key() == k)) {
            let asset = self.cards[index].asset();
            self.state.borrow_mut().asset_selection = asset;
            if let (Some(open), CardKind::Asset(id)) = (request.expand, &self.cards[index].kind) {
                if open {
                    self.expanded.insert(*id);
                } else {
                    self.expanded.remove(id);
                }
                let database = assets::database();
                self.rebuild(&database);
            }
        }
        if request.menu {
            self.layout();
            let (gx, gy, _, _) = self.grid_rect();
            match self.selected.as_ref().and_then(|k| self.cards.iter().position(|c| &c.key() == k)) {
                Some(index) => {
                    let at = self.views[index].region.get_absolute_position();
                    let entries = self.card_menu(&self.cards[index].clone());
                    self.open_menu(at.x + CARD_WIDTH * 0.6, at.y + 30.0, entries);
                }
                None => {
                    let entries = self.folder_menu();
                    self.open_menu(gx + 40.0, gy + 20.0, entries);
                }
            }
        }
    }

    fn selected_card(&self) -> Option<&Card> {
        let key = self.selected.as_ref()?;
        self.cards.iter().find(|c| &c.key() == key)
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.and_then(|i| self.cards.get(i)).map(Card::key);
        let asset = index.and_then(|i| self.cards.get(i)).and_then(Card::asset);
        self.state.borrow_mut().asset_selection = asset;
    }

    fn card_at(&self, x: f32, y: f32) -> Option<usize> {
        let (gx, gy, gw, gh) = self.grid_rect();
        if x < gx || y < gy || x > gx + gw || y > gy + gh {
            return None;
        }
        self.views.iter().take(self.cards.len()).position(|v| v.shown && v.region.contains(x, y))
    }

    fn in_grid(&self, x: f32, y: f32) -> bool {
        let (gx, gy, gw, gh) = self.grid_rect();
        x >= gx && y >= gy && x <= gx + gw && y <= gy + gh
    }

    /// Make the cards again if what they show changed.
    fn rebuild(&mut self, database: &AssetDatabase) {
        let search = self.search_query();
        let mut expanded: Vec<AssetId> = self.expanded.iter().copied().collect();
        expanded.sort();
        let wanted = (database.changes(), self.category, self.folder.clone(), search.clone(), expanded);
        if self.built_for.as_ref() == Some(&wanted) {
            return;
        }
        // A folder that went away (deleted, renamed): back to the nearest
        // one still there.
        while !self.folder.is_dir() && self.folder != database.root() {
            match self.folder.parent() {
                Some(parent) if parent.starts_with(database.root()) => self.folder = parent.to_path_buf(),
                _ => self.folder = database.root().to_path_buf(),
            }
        }
        if !self.folder.starts_with(database.root()) {
            self.folder = database.root().to_path_buf();
        }
        self.cards = view::cards(database, self.category, &self.folder, &search, &self.expanded);
        while self.views.len() < self.cards.len() {
            self.views.push(CardView::new());
        }
        self.counts = view::counts(database);
        self.built_for = Some(wanted);
        // The selected card may be gone.
        if self.selected.is_some() && self.selected_card().is_none() {
            self.selected = None;
            self.state.borrow_mut().asset_selection = None;
        }
        self.crumbs = view::crumbs(database.root(), &self.folder)
            .into_iter()
            .map(|(name, path)| (theme::text(&name, theme::FONT_SIZE, theme::TEXT_MUTED), Region::default(), path))
            .collect();
        self.empty.set_content(if !search.is_empty() {
            "Nothing found."
        } else if database.is_empty() {
            "No assets yet. Import a model or texture, or right-click to make one."
        } else {
            "This folder is empty. Right-click to make something here."
        });
        self.clamp_scroll();
    }

    fn details_text(&self, database: &AssetDatabase) -> String {
        match self.selected_card().map(|c| &c.kind) {
            Some(CardKind::Asset(id)) => database.get(*id).map(describe).unwrap_or_default(),
            Some(CardKind::Part(asset)) => {
                let Some(entry) = database.get(asset.id) else {
                    return String::new();
                };
                let part = asset.part.as_deref().unwrap_or("");
                if let Some(clip) = entry.meta.clips().iter().find(|c| c.name == part) {
                    return format!("{part}  ·  clip of {}  ·  {:.2} s", entry.meta.name, clip.duration);
                }
                let lods = entry.meta.as_model().and_then(|m| m.variant(part)).map_or(0, |v| v.lods.len());
                format!("{part}  ·  variant of {}  ·  {}", entry.meta.name, count(lods, "LOD"))
            }
            Some(CardKind::Folder(path)) => {
                let n = database.assets_in(path).len();
                format!("{}  ·  folder  ·  {}  ·  {}", file_name(path), count(n, "asset"), relative(&self.project, path))
            }
            None => {
                let index = Category::ALL.iter().position(|c| *c == self.category).unwrap_or(0);
                format!("{}  ·  {}", self.category.label(), count(self.counts.get(index).copied().unwrap_or(0), "asset"))
            }
        }
    }

    fn layout(&mut self) {
        let rect = self.state.borrow().layout.assets;
        let header = self.panel.header_height();
        let top = rect.y + header;

        // Side bar.
        self.sidebar.set_position(Position::new(rect.x, top, 1.0));
        self.sidebar.set_size(Size::new(SIDEBAR_WIDTH, (rect.h - header).max(0.0)));
        for (i, row) in self.side_rows.iter_mut().enumerate() {
            let y = top + 6.0 + i as f32 * SIDE_ROW;
            let p = Position::new(rect.x + 6.0, y, 2.0);
            row.region = Region::new(p, Size::new(SIDEBAR_WIDTH - 12.0, SIDE_ROW - 2.0));
            row.bg.set_position(p);
            row.bg.set_size(row.region.size);
            let active = row.category == self.category;
            row.bg.set_color(if active { theme::ACCENT_SOFT } else { theme::TRANSPARENT });
            row.icon.set_color(if active { theme::ACCENT_TEXT } else { theme::TEXT_MUTED });
            row.label.set_color(if active { theme::ACCENT_TEXT } else { theme::TEXT });
            let iy = theme::center_y(&row.icon, SIDE_ROW - 2.0);
            row.icon.prepare_render_at(Position::new(p.x + 8.0, y + iy, 3.0));
            let ly = theme::center_y(&row.label, SIDE_ROW - 2.0);
            row.label.prepare_render_at(Position::new(p.x + 32.0, y + ly, 3.0));
            row.count.set_content(&self.counts.get(i).map(|n| n.to_string()).unwrap_or_default());
            let cy = theme::center_y(&row.count, SIDE_ROW - 2.0);
            let cx = p.x + row.region.size.width - 8.0 - row.count.measure_width();
            row.count.prepare_render_at(Position::new(cx, y + cy, 3.0));
        }

        // Tool bar: up, path, ..., search, New, Import, refresh.
        let x0 = rect.x + SIDEBAR_WIDTH + 8.0;
        let ty = top + 4.0;
        let right = rect.x + rect.w - 6.0;
        self.refresh.set_offset(Offset { x: right - self.refresh.get_size().width, y: ty });
        let import_x = right - self.refresh.get_size().width - 2.0 - self.import.get_size().width;
        self.import.set_offset(Offset { x: import_x, y: ty });
        let new_x = import_x - 4.0 - self.new_button.get_size().width;
        self.new_button.set_offset(Offset { x: new_x, y: ty });
        let search_x = new_x - 10.0 - SEARCH_WIDTH;
        self.search.set_offset(Offset { x: search_x, y: ty + 1.0 });
        self.search_hint.prepare_render_at(Position::new(search_x + 8.0, ty + 1.0 + theme::center_y(&self.search_hint, 24.0), 4.0));

        let searching = !self.search_query().is_empty();
        self.up.set_offset(Offset { x: x0, y: ty });
        let root = assets::database().root().to_path_buf();
        self.up.set_enabled(self.folder != root && !searching);
        let mut cx = x0 + 30.0;
        let crumb_count = self.crumbs.len();
        for (i, (text, region, _)) in self.crumbs.iter_mut().enumerate() {
            let last = i + 1 == crumb_count;
            text.set_color(if searching { theme::TEXT_DIM } else if last { theme::TEXT } else { theme::TEXT_MUTED });
            let width = text.measure_width();
            let y = ty + theme::center_y(text, 26.0);
            text.prepare_render_at(Position::new(cx, y, 3.0));
            *region = Region::new(Position::new(cx - 2.0, ty, 3.0), Size::new(width + 4.0, 26.0));
            cx += width + 18.0;
        }

        // Cards.
        let (gx, gy, gw, gh) = self.grid_rect();
        self.grid.set_position(Position::new(gx - 2.0, gy, 1.0));
        self.grid.set_size(Size::new(gw + 4.0, gh));
        let columns = self.columns();
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            if let Some(index) = self.selected.as_ref().and_then(|k| self.cards.iter().position(|c| &c.key() == k)) {
                let row_top = (index / columns) as f32 * (CARD_HEIGHT + GAP);
                if row_top < self.scroll {
                    self.scroll = row_top;
                } else if row_top + CARD_HEIGHT > self.scroll + gh {
                    self.scroll = row_top + CARD_HEIGHT - gh;
                }
            }
        }
        self.clamp_scroll();
        let selected = self.selected.clone();
        let mut visible_assets = Vec::new();
        for (i, card) in self.cards.iter().enumerate() {
            let view = &mut self.views[i];
            let x = gx + (i % columns) as f32 * (CARD_WIDTH + GAP);
            let y = gy + (i / columns) as f32 * (CARD_HEIGHT + GAP) - self.scroll;
            view.shown = y + CARD_HEIGHT > gy && y < gy + gh;
            view.region = Region::new(Position::new(x, y, 2.0), Size::new(CARD_WIDTH, CARD_HEIGHT));
            if !view.shown {
                continue;
            }
            let is_selected = selected.as_deref() == Some(card.key().as_str());
            let hovered = self.hover == Some(i);
            let part = matches!(card.kind, CardKind::Part(_));
            view.bg.set_position(Position::new(x, y, 2.0));
            view.bg.set_size(Size::new(CARD_WIDTH, CARD_HEIGHT));
            view.bg.set_color(match (is_selected, hovered, part) {
                (true, _, _) => theme::ACCENT_SOFT,
                (false, true, _) => theme::BG_CONTROL_HOVER,
                (false, false, true) => theme::BG_HEADER,
                (false, false, false) => theme::BG_CONTROL,
            });
            view.bg.set_border_color(if is_selected { theme::ACCENT } else if part { theme::BORDER_STRONG } else { theme::TRANSPARENT });
            let tx = x + (CARD_WIDTH - THUMB) / 2.0;
            let ty = y + 7.0;
            view.thumb_bg.set_position(Position::new(tx, ty, 3.0));
            view.thumb_bg.set_size(Size::new(THUMB, THUMB));
            view.icon.set_content(card.icon);
            view.icon.set_color(match card.kind {
                CardKind::Folder(_) => theme::WARNING,
                _ if is_selected => theme::ACCENT_TEXT,
                _ => theme::TEXT_MUTED,
            });
            let iw = view.icon.measure_width();
            let ih = view.icon.line_height();
            view.icon.prepare_render_at(Position::new(tx + (THUMB - iw) / 2.0, ty + (THUMB - ih) / 2.0, 4.0));
            let label = fit(&view.label, &card.label, CARD_WIDTH - 10.0);
            view.label.set_content(&label);
            view.label.set_color(if is_selected { theme::ACCENT_TEXT } else { theme::TEXT });
            let lw = view.label.measure_width();
            view.label.prepare_render_at(Position::new(x + (CARD_WIDTH - lw) / 2.0, ty + THUMB + 5.0, 4.0));
            let detail = fit(&view.detail, &card.detail, CARD_WIDTH - 10.0);
            view.detail.set_content(&detail);
            let dw = view.detail.measure_width();
            view.detail.prepare_render_at(Position::new(x + (CARD_WIDTH - dw) / 2.0, ty + THUMB + 21.0, 4.0));
            // The fold arrow of a pack, on the picture's top right.
            let c = Position::new(tx + THUMB - 22.0, ty + 3.0, 5.0);
            view.chevron_region = if card.parts > 0 { Region::new(c, Size::new(20.0, 20.0)) } else { Region::default() };
            view.chevron_bg.set_position(c);
            view.chevron_bg.set_size(Size::new(20.0, 20.0));
            view.chevron.set_content(if card.expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT });
            let cw = view.chevron.measure_width();
            view.chevron.prepare_render_at(Position::new(c.x + (20.0 - cw) / 2.0, c.y + theme::center_y(&view.chevron, 20.0), 6.0));
            if let Some(asset) = card.asset() {
                self.thumbs.place(&asset, tx, ty, THUMB);
                visible_assets.push(asset);
            }
        }
        let ew = self.empty.measure_width();
        self.empty.prepare_render_at(Position::new(gx + (gw - ew).max(0.0) / 2.0, gy + 30.0, 3.0));

        let database = assets::database();
        self.thumbs.want(&database, &visible_assets);

        // Details.
        self.details.set_offset(Offset { x: x0, y: rect.y + rect.h - DETAILS_HEIGHT });
        self.details.set_max_width(Some((rect.w - SIDEBAR_WIDTH - 16.0).max(0.0)));
    }

    /// The menu for a card.
    fn card_menu(&self, card: &Card) -> Vec<(ContextEntry, MenuAction)> {
        let open = (ContextEntry::item(icons::PENCIL, "Open"), MenuAction::Open(card.clone()));
        let path_of = |id: AssetId| assets::database().get(id).map(|e| e.source_path());
        match &card.kind {
            CardKind::Folder(path) => vec![
                open,
                (ContextEntry::item(icons::FILE_TEXT, "Rename…"), MenuAction::RenameFolder(path.clone())),
                (ContextEntry::item(icons::FOLDER_OPEN, "Show in file manager"), MenuAction::Reveal(path.clone())),
                (ContextEntry::Separator, MenuAction::Nothing),
                (ContextEntry::danger(icons::TRASH, "Delete…"), MenuAction::DeleteFolder(path.clone())),
            ],
            CardKind::Asset(id) => {
                let source = path_of(*id).unwrap_or_default();
                vec![
                    open,
                    (ContextEntry::item(icons::FILE_TEXT, "Rename…"), MenuAction::Rename(*id)),
                    (ContextEntry::item(icons::COPY_PLUS, "Duplicate"), MenuAction::Duplicate(*id)),
                    (ContextEntry::item(icons::COPY, "Copy path"), MenuAction::CopyPath(source.clone())),
                    (ContextEntry::item(icons::FOLDER_OPEN, "Show in file manager"), MenuAction::Reveal(source)),
                    (ContextEntry::Separator, MenuAction::Nothing),
                    (ContextEntry::danger(icons::TRASH, "Delete…"), MenuAction::Delete(*id)),
                ]
            }
            CardKind::Part(asset) => {
                let source = path_of(asset.id).unwrap_or_default();
                vec![open, (ContextEntry::item(icons::COPY, "Copy path"), MenuAction::CopyPath(source))]
            }
        }
    }

    /// The "New" entries, the category's own kind first.
    fn new_entries(&self) -> Vec<(ContextEntry, MenuAction)> {
        let mut kinds: Vec<NewAsset> = NewAsset::ALL.to_vec();
        if let Some(first) = self.category.new_kind() {
            kinds.retain(|k| *k != first);
            kinds.insert(0, first);
        }
        let icon = |kind: NewAsset| match kind {
            NewAsset::Material => icons::SPARKLES,
            NewAsset::MaterialGraph => icons::COMPONENT,
            NewAsset::ScatterGraph => icons::LIST_TREE,
        };
        let mut entries: Vec<(ContextEntry, MenuAction)> =
            kinds.into_iter().map(|k| (ContextEntry::item(icon(k), format!("New {}…", k.label().to_lowercase())), MenuAction::New(k))).collect();
        entries.push((ContextEntry::item(icons::FOLDER_OPEN, "New folder…"), MenuAction::NewFolder));
        entries
    }

    /// The menu for the folder itself (a right click beside the cards).
    fn folder_menu(&self) -> Vec<(ContextEntry, MenuAction)> {
        let mut entries = self.new_entries();
        entries.push((ContextEntry::Separator, MenuAction::Nothing));
        entries.push((ContextEntry::item(icons::ARROW_DOWN_TO_LINE, "Import…"), MenuAction::Import));
        entries.push((ContextEntry::item(icons::ROTATE_CW, "Scan again"), MenuAction::Rescan));
        entries.push((ContextEntry::item(icons::FOLDER_OPEN, "Show in file manager"), MenuAction::Reveal(self.folder.clone())));
        entries
    }

    fn open_menu(&mut self, x: f32, y: f32, entries: Vec<(ContextEntry, MenuAction)>) {
        let (entries, actions): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        self.menu_actions = actions;
        self.menu.open_at(x, y, entries);
    }

    /// Open a card: a folder in the browser, an asset (or part) in its editor.
    fn open_card(&mut self, card: &Card) {
        match &card.kind {
            CardKind::Folder(path) => self.open_folder(path.clone()),
            CardKind::Asset(id) => send(&self.state, Command::EditAsset { id: *id, variant: None }),
            CardKind::Part(asset) => send(&self.state, Command::EditAsset { id: asset.id, variant: asset.part.clone() }),
        }
    }

    fn run_menu_action(&mut self, action: MenuAction, window: &mut glfw::Window) {
        // New things go where the browser is (a search shows no folder: the
        // kind's own folder then).
        let here = if self.search_query().is_empty() { Some(self.folder.clone()) } else { None };
        match action {
            MenuAction::Open(card) => self.open_card(&card),
            MenuAction::Rename(id) => send(&self.state, Command::RenameAssetDialog(id)),
            MenuAction::RenameFolder(path) => send(&self.state, Command::RenameFolderDialog(path)),
            MenuAction::Duplicate(id) => send(&self.state, Command::DuplicateAsset(id)),
            MenuAction::CopyPath(path) => {
                let text = relative(&self.project, &path);
                window.set_clipboard_string(&text);
                self.state.borrow_mut().set_status(format!("Copied {text}"));
            }
            MenuAction::Reveal(path) => reveal_in_file_manager(&path),
            MenuAction::Delete(id) => send(&self.state, Command::DeleteAssetDialog(id)),
            MenuAction::DeleteFolder(path) => send(&self.state, Command::DeleteFolderDialog(path)),
            MenuAction::New(kind) => send(&self.state, Command::NewAssetDialog { kind, folder: here }),
            MenuAction::NewFolder => send(&self.state, Command::NewFolderDialog(here.unwrap_or_else(|| self.folder.clone()))),
            MenuAction::Import => send(&self.state, Command::ImportAssets),
            MenuAction::Rescan => send(&self.state, Command::RefreshAssets),
            MenuAction::Nothing => {}
        }
    }

    /// The asset (or part) on a card, ready to be dragged: not yet active,
    /// the drag layer starts it once the cursor moves.
    fn drag_of(&self, card: &Card, x: f32, y: f32) -> Option<AssetDrag> {
        let asset = card.asset()?;
        let database = assets::database();
        let entry = database.get(asset.id)?;
        let kind = match entry.meta.kind {
            AssetKind::Model(_) => DragKind::Model,
            AssetKind::Texture(_) => DragKind::Texture,
            AssetKind::Animation(_) => DragKind::Animation,
            AssetKind::Material(_) => DragKind::Material,
            // Nothing takes a graph by dragging yet.
            AssetKind::ScatterGraph(_) | AssetKind::MaterialGraph(_) => return None,
        };
        Some(AssetDrag { label: database.display_name(&asset)?, kind, asset, origin: (x, y), position: (x, y), active: false })
    }

    fn widgets_mut(&mut self) -> [&mut dyn UIElement; 5] {
        [&mut self.up, &mut self.search, &mut self.new_button, &mut self.import, &mut self.refresh]
    }

    /// A left press at `(x, y)` (the menu is closed). `true` if taken.
    fn press(&mut self, x: f32, y: f32) -> bool {
        if let Some(row) = self.side_rows.iter().find(|r| r.region.contains(x, y)) {
            let category = row.category;
            self.set_category(category);
            return true;
        }
        if hits(&self.up, x, y) && self.up.is_enabled() {
            if let Some(parent) = self.folder.parent() {
                self.open_folder(parent.to_path_buf());
            }
            return true;
        }
        if hits(&self.new_button, x, y) {
            let offset = *self.new_button.get_offset();
            let (bx, by) = (offset.x, offset.y + self.new_button.get_size().height + 4.0);
            let entries = self.new_entries();
            self.open_menu(bx, by, entries);
            return true;
        }
        if let Some(path) = self.crumbs.iter().find(|c| c.1.contains(x, y)).map(|c| c.2.clone()) {
            self.open_folder(path);
            return true;
        }
        if let Some(index) = self.card_at(x, y) {
            let card = self.cards[index].clone();
            if self.views[index].chevron_region.contains(x, y) {
                if let CardKind::Asset(id) = card.kind {
                    if !self.expanded.remove(&id) {
                        self.expanded.insert(id);
                    }
                }
                return true;
            }
            let double = self.last_press.is_some_and(|(last, at)| last == index && at.elapsed().as_secs_f32() < DOUBLE_CLICK);
            self.last_press = if double { None } else { Some((index, Instant::now())) };
            self.select(Some(index));
            if double {
                self.open_card(&card);
            } else {
                let drag = self.drag_of(&card, x, y);
                self.state.borrow_mut().drag = drag;
            }
            return true;
        }
        if self.in_grid(x, y) {
            self.select(None);
            return true;
        }
        false
    }
}

/// Whether a point is on a widget placed by offset (buttons, fields).
fn hits(widget: &dyn UIElement, x: f32, y: f32) -> bool {
    let (o, s) = (widget.get_offset(), widget.get_size());
    x >= o.x && y >= o.y && x <= o.x + s.width && y <= o.y + s.height
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `path` below the project, `/`-separated.
fn relative(project: &Path, path: &Path) -> String {
    let rest = path.strip_prefix(project).unwrap_or(path).to_string_lossy().replace('\\', "/");
    rest.trim_start_matches("./").to_string()
}

/// Open the system's file manager on a folder (or the folder holding a file).
fn reveal_in_file_manager(path: &Path) {
    let folder = if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
    let folder = std::fs::canonicalize(&folder).unwrap_or(folder);
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(opener).arg(&folder).spawn() {
        log::warn!("could not open {}: {e}", folder.display());
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
            count(model.materials.iter().filter(|m| !m.unused).count(), "material"),
        ),
        AssetKind::Animation(animation) => {
            format!("{}  ·  Animation  ·  {}  ·  {source}", entry.meta.name, count(animation.clips.len(), "clip"))
        }
        kind => format!("{}  ·  {}  ·  {source}", entry.meta.name, kind.label()),
    }
}

impl UIElement for AssetBrowser {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.state.borrow().layout.assets;
        if rect.h <= 0.0 {
            return;
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        // Another project: start over.
        let project = assets::database().project_root();
        if project != self.project {
            self.thumbs.reopen(&project);
            self.project = project;
            self.restored = false;
            self.built_for = None;
        }
        if !self.restored {
            self.restored = true;
            let (category, folder) = self.state.borrow().asset_view.clone();
            self.category = Category::from_key(&category).unwrap_or(Category::All);
            let root = assets::database().root().to_path_buf();
            let folder = if folder.is_empty() { root.clone() } else { root.join(folder) };
            self.folder = if folder.is_dir() { folder } else { root };
        }
        // Wishes from elsewhere: show this asset or folder.
        let (reveal, folder_reveal) = {
            let mut state = self.state.borrow_mut();
            (state.asset_reveal.take(), state.asset_folder_reveal.take())
        };
        if let Some(folder) = folder_reveal {
            self.category = Category::All;
            self.open_folder(folder);
        }
        let request = self.state.borrow_mut().asset_browser_request.take();
        if let Some(request) = request {
            self.follow(request);
        }
        if let Some(id) = reveal {
            {
                let database = assets::database();
                self.rebuild(&database);
            }
            self.reveal(id);
        }

        {
            // Short-lived: commands and loading code use the database too.
            let database = assets::database();
            self.rebuild(&database);
            let details_for = (self.selected.clone(), database.changes(), self.category);
            if self.details_for.as_ref() != Some(&details_for) {
                self.details.set_text(self.details_text(&database));
                self.details_for = Some(details_for);
            }
        }
        self.layout();

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
        // A queued material or model picture, drawn in a pass of its own.
        self.thumbs.render_pending();

        self.panel.render();
        self.sidebar.render();
        for row in &self.side_rows {
            row.bg.render();
            row.icon.render();
            row.label.render();
            row.count.render();
        }
        for (text, _, _) in &self.crumbs {
            text.render();
        }

        clip::push(&self.grid);
        for (card, view) in self.cards.iter().zip(&self.views) {
            if !view.shown {
                continue;
            }
            view.bg.render();
            view.thumb_bg.render();
            match card.asset().filter(|a| self.thumbs.ready(a)) {
                Some(asset) => self.thumbs.draw(&asset),
                None => {
                    view.icon.render();
                }
            }
            view.label.render();
            view.detail.render();
            if card.parts > 0 {
                view.chevron_bg.render();
                view.chevron.render();
            }
        }
        if self.cards.is_empty() {
            self.empty.render();
        }
        clip::pop(&self.grid);

        self.details.render();
        self.up.render();
        self.search.render();
        if self.search_text.read().is_empty() && !self.search.is_focused() {
            self.search_hint.render();
        }
        self.new_button.render();
        self.import.render();
        self.refresh.render();
    }

    fn render_overlay(&self) {
        if !self.visible() {
            return;
        }
        self.up.render_overlay();
        self.new_button.render_overlay();
        self.import.render_overlay();
        self.refresh.render_overlay();
        self.menu.render();
    }

    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        if !self.visible() {
            self.menu.close();
            return false;
        }
        // An open menu comes first.
        if let Some(result) = self.menu.handle_event(window, event) {
            match result {
                ContextResult::Chosen(index) => {
                    let action = self.menu_actions.get(index).cloned().unwrap_or(MenuAction::Nothing);
                    self.run_menu_action(action, window);
                    return true;
                }
                ContextResult::Inside => return true,
                // A press outside closes the menu and goes on as usual.
                ContextResult::Dismissed => {}
            }
        }
        let (x, y) = window.get_cursor_pos();
        let (x, y) = (x as f32, y as f32);
        match event {
            WindowEvent::CursorPos(..) => {
                self.hover = self.card_at(x, y);
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                // The search field takes its own presses (focus).
                if !hits(&self.search, x, y) && self.press(x, y) {
                    // Let the buttons see the press too, for their looks;
                    // their callbacks are empty or harmless.
                    for widget in [&mut self.import as &mut dyn UIElement, &mut self.refresh] {
                        widget.handle_events(scene, window, glfw, event);
                    }
                    return true;
                }
            }
            WindowEvent::MouseButton(MouseButton::Button2, Action::Press, _) => {
                if let Some(index) = self.card_at(x, y) {
                    self.select(Some(index));
                    let entries = self.card_menu(&self.cards[index].clone());
                    self.open_menu(x, y, entries);
                    return true;
                }
                if self.in_grid(x, y) {
                    self.select(None);
                    let entries = self.folder_menu();
                    self.open_menu(x, y, entries);
                    return true;
                }
            }
            WindowEvent::Scroll(_, dy) => {
                if self.in_grid(x, y) {
                    self.scroll -= *dy as f32 * 40.0;
                    self.clamp_scroll();
                    return true;
                }
            }
            _ => {}
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
            return self.state.borrow().layout.assets.contains(x, y);
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
