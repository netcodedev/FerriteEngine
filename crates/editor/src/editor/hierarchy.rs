use ferrite::core::{
    entity::Entity,
    renderer::text::icons,
    primitives::{Offset, Position, Size},
    scene::Scene,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        menu_button::{MenuButton, MenuItem},
        panel::Panel,
        tree::{TreeItem, TreeView},
    },
    UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::{send, Command, EntityTemplate, Shared};

/// Docked scene tree with add / duplicate / delete actions.
pub struct HierarchyPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    tree: TreeView,
    add_menu: MenuButton,
    duplicate: Button,
    delete: Button,
    last_selection: u64,
    /// The scene's tree as last given to `tree`.
    items: Vec<TreeItem>,
}

const ACTIONS_HEIGHT: f32 = 30.0;

/// Components that tell what an entity is, most telling first, with the
/// icon each stands for.
const TELLING_COMPONENTS: [(&str, &str); 5] = [
    // The player template's marker component (see `ComponentData::Player`).
    ("Bone colliders", icons::USER),
    ("Camera", icons::CAMERA),
    ("Sky light", icons::SUN),
    ("Terrain", icons::MOUNTAIN),
    ("Debug controller", icons::BUG),
];

/// Icon for an entity, chosen from its most telling component.
pub fn entity_icon(entity: &Entity) -> &'static str {
    let mut best = TELLING_COMPONENTS.len();
    for component in entity.components() {
        let name = component.display_name();
        if let Some(rank) = TELLING_COMPONENTS[..best].iter().position(|(n, _)| *n == name) {
            best = rank;
        }
    }
    TELLING_COMPONENTS.get(best).map_or(icons::BOX, |(_, icon)| icon)
}

pub fn template_icon(template: EntityTemplate) -> &'static str {
    match template {
        EntityTemplate::Empty => icons::BOX,
        EntityTemplate::Camera => icons::CAMERA,
        EntityTemplate::SkyLight => icons::SUN,
        EntityTemplate::Terrain => icons::MOUNTAIN,
        EntityTemplate::Player => icons::USER,
    }
}

impl HierarchyPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new(tr!("hierarchy.title").into(), Position::default(), Size::new(260.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);

        let selection = state.borrow().selection.clone();
        let tree = TreeView::new(Position::default(), Size::new(250.0, 300.0), selection);

        let add_state = state.clone();
        let mut items: Vec<MenuItem> = EntityTemplate::ALL
            .iter()
            .map(|(_, template)| MenuItem::new(template.label()).icon(template_icon(*template)))
            .collect();
        items.push(MenuItem::new(tr!("hierarchy.empty_child")).icon(icons::LIST_TREE));
        let add_menu = MenuButton::with_items(
            tr!("common.add"),
            Some(icons::PLUS),
            items,
            Box::new(move |index, _scene| {
                let selected = add_state.borrow().selected();
                let (parent, template) = match EntityTemplate::ALL.get(index) {
                    Some((_, template)) => (None, *template),
                    None => (selected, EntityTemplate::Empty),
                };
                if index >= EntityTemplate::ALL.len() && parent.is_none() {
                    add_state
                        .borrow_mut()
                        .set_status(tr!("hierarchy.select_parent"));
                    return;
                }
                send(&add_state, Command::AddEntity { parent, template });
            }),
        );

        let dup_state = state.clone();
        let mut duplicate = Button::new(
            Position::default(),
            Size::new(26.0, 26.0),
            Box::new(move |_| {
                // Read first: the borrow must end before `send` borrows mutably.
                let selected = dup_state.borrow().selected();
                if let Some(id) = selected {
                    send(&dup_state, Command::Duplicate(id));
                }
            }),
        );
        duplicate.set_style(ButtonStyle::Ghost);
        duplicate.set_icon(icons::COPY_PLUS);
        duplicate.set_tooltip(tr!("hierarchy.duplicate_tooltip"));
        let del_state = state.clone();
        let mut delete = Button::new(
            Position::default(),
            Size::new(26.0, 26.0),
            Box::new(move |_| {
                // Read first: the borrow must end before `send` borrows mutably.
                let selected = del_state.borrow().selected();
                if let Some(id) = selected {
                    send(&del_state, Command::Delete(id));
                }
            }),
        );
        delete.set_style(ButtonStyle::Danger);
        delete.set_icon(icons::TRASH);
        delete.set_tooltip(tr!("hierarchy.delete_tooltip"));

        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            tree,
            add_menu,
            duplicate,
            delete,
            last_selection: 0,
            items: Vec::new(),
        }
    }

    /// Bring `items` in line with the scene's tree, in place: nothing is
    /// rebuilt while the scene stays the same. True when anything changed.
    fn sync_items(scene: &Scene, items: &mut Vec<TreeItem>) -> bool {
        fn visit(entity: &Entity, depth: usize, items: &mut Vec<TreeItem>, count: &mut usize, changed: &mut bool) {
            if entity.is_transient() {
                return;
            }
            let id = u64::from(entity.id);
            let has_children = entity.get_children().iter().any(|c| !c.is_transient());
            let icon = Some(entity_icon(entity));
            let label = entity.get_name();
            match items.get_mut(*count) {
                Some(item) => {
                    if item.id != id || item.depth != depth || item.has_children != has_children || item.icon != icon {
                        item.id = id;
                        item.depth = depth;
                        item.has_children = has_children;
                        item.icon = icon;
                        *changed = true;
                    }
                    if item.label != label {
                        item.label = label;
                        *changed = true;
                    }
                }
                None => {
                    items.push(TreeItem {
                        id,
                        label,
                        depth,
                        has_children,
                        icon,
                    });
                    *changed = true;
                }
            }
            *count += 1;
            for child in entity.get_children() {
                visit(child, depth + 1, items, count, changed);
            }
        }
        let (mut count, mut changed) = (0, false);
        for entity in scene.get_entities() {
            visit(entity, 0, items, &mut count, &mut changed);
        }
        if items.len() > count {
            items.truncate(count);
            changed = true;
        }
        changed
    }

    fn widgets_mut(&mut self) -> [&mut dyn UIElement; 4] {
        [
            &mut self.add_menu,
            &mut self.duplicate,
            &mut self.delete,
            &mut self.tree,
        ]
    }
}

impl UIElement for HierarchyPanel {
    fn update(&mut self, scene: &mut Scene) {
        let (rect, selection) = {
            let state = self.state.borrow();
            (state.layout.hierarchy, state.selection.read())
        };
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let header = self.panel.header_height();
        let x = rect.x + 6.0;
        let ay = rect.y + header + 4.0;
        self.add_menu.set_offset(Offset { x, y: ay });
        let mut right = rect.x + rect.w - 6.0;
        for widget in [&mut self.delete as &mut dyn UIElement, &mut self.duplicate] {
            right -= widget.get_size().width;
            widget.set_offset(Offset { x: right, y: ay });
            right -= 2.0;
        }

        let tree_top = rect.y + header + ACTIONS_HEIGHT + 6.0;
        self.tree
            .set_position(Position::new(x, tree_top, 1.0));
        self.tree.set_size(Size::new(
            (rect.w - 12.0).max(10.0),
            (rect.y + rect.h - tree_top - 6.0).max(10.0),
        ));
        if Self::sync_items(scene, &mut self.items) {
            self.tree.set_items(self.items.clone());
        }
        // Duplicate and delete act on the selection; without one they are
        // greyed out.
        let has_selection = self.state.borrow().selected().is_some();
        self.duplicate.set_enabled(has_selection);
        self.delete.set_enabled(has_selection);
        if selection != self.last_selection {
            self.last_selection = selection;
            self.tree.reveal(selection);
        }

        self.panel.update(scene);
        for widget in self.widgets_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        self.panel.render();
        self.tree.render();
        self.add_menu.render();
        self.duplicate.render();
        self.delete.render();
    }

    fn render_overlay(&self) {
        self.add_menu.render_overlay();
        self.duplicate.render_overlay();
        self.delete.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
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
                .hierarchy
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
        for widget in self.widgets_mut() {
            widget.set_z_index(z_index + 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The items built from scratch, as the tree shows them.
    fn built(scene: &Scene) -> Vec<TreeItem> {
        fn visit(entity: &Entity, depth: usize, out: &mut Vec<TreeItem>) {
            if entity.is_transient() {
                return;
            }
            out.push(TreeItem {
                id: u64::from(entity.id),
                label: entity.get_name(),
                depth,
                has_children: entity.get_children().iter().any(|c| !c.is_transient()),
                icon: Some(entity_icon(entity)),
            });
            for child in entity.get_children() {
                visit(child, depth + 1, out);
            }
        }
        let mut items = Vec::new();
        for entity in scene.get_entities() {
            visit(entity, 0, &mut items);
        }
        items
    }

    #[test]
    fn items_follow_the_scene_in_place() {
        ferrite::core::headless::enable();
        let mut scene = Scene::new();
        let (a, b, c) = (Entity::new("a"), Entity::new("b"), Entity::new("c"));
        let (a_id, b_id) = (a.id, b.id);
        scene.add_entity(a);
        scene.add_entity(b);
        scene.add_entity(c);

        let mut items = Vec::new();
        assert!(HierarchyPanel::sync_items(&scene, &mut items));
        assert_eq!(items, built(&scene));
        assert!(!HierarchyPanel::sync_items(&scene, &mut items), "nothing changed");

        // A child: its parent gets an arrow, the rest moves down a row.
        scene.add_entity_to(Some(a_id), Entity::new("child")).ok().unwrap();
        assert!(HierarchyPanel::sync_items(&scene, &mut items));
        assert_eq!(items, built(&scene));

        scene.get_entity_mut(&b_id).unwrap().set_name("renamed".into());
        assert!(HierarchyPanel::sync_items(&scene, &mut items));
        assert_eq!(items, built(&scene));

        // Removed: the list gets shorter.
        assert!(scene.remove_entity(&a_id));
        assert!(HierarchyPanel::sync_items(&scene, &mut items));
        assert_eq!(items, built(&scene));
        assert_eq!(items.len(), 2);
        assert!(!HierarchyPanel::sync_items(&scene, &mut items));
    }
}
