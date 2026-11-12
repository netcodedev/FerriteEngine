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
}

const ACTIONS_HEIGHT: f32 = 30.0;

/// Icon for an entity, chosen from its most telling component.
pub fn entity_icon(entity: &Entity) -> &'static str {
    let names: Vec<String> = entity.components().iter().map(|c| c.display_name()).collect();
    let has = |name: &str| names.iter().any(|n| n == name);
    if has("Player controller") {
        icons::USER
    } else if has("Camera") {
        icons::CAMERA
    } else if has("Sky light") {
        icons::SUN
    } else if has("Terrain") {
        icons::MOUNTAIN
    } else if has("Debug controller") {
        icons::BUG
    } else {
        icons::BOX
    }
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
        let mut panel = Panel::new("Hierarchy".into(), Position::default(), Size::new(260.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);

        let selection = state.borrow().selection.clone();
        let tree = TreeView::new(Position::default(), Size::new(250.0, 300.0), selection);

        let add_state = state.clone();
        let mut items: Vec<MenuItem> = EntityTemplate::ALL
            .iter()
            .map(|(name, template)| MenuItem::new(*name).icon(template_icon(*template)))
            .collect();
        items.push(MenuItem::new("Empty child of selected").icon(icons::LIST_TREE));
        let add_menu = MenuButton::with_items(
            "Add",
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
                        .set_status("Select an entity to add a child to it");
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
        duplicate.set_tooltip("Duplicate selected (Ctrl+D)");
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
        delete.set_tooltip("Delete selected (Del)");

        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            tree,
            add_menu,
            duplicate,
            delete,
            last_selection: 0,
        }
    }

    fn build_items(scene: &Scene) -> Vec<TreeItem> {
        fn visit(entity: &Entity, depth: usize, out: &mut Vec<TreeItem>) {
            if entity.is_transient() {
                return;
            }
            let has_children = entity.get_children().iter().any(|c| !c.is_transient());
            out.push(TreeItem {
                id: u64::from(entity.id),
                label: entity.get_name(),
                depth,
                has_children,
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
        self.tree.set_items(Self::build_items(scene));
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
