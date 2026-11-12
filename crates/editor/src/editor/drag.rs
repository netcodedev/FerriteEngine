//! Dragging assets out of the asset browser onto fields that accept them.
//!
//! The browser only notes what was pressed (`EditorState::drag`, not yet
//! active). This layer turns it into a drag once the cursor has moved a
//! little, draws the dragged asset at the cursor, highlights the target
//! under it and, on release, queues the target's command. Panels register
//! their targets (`EditorState::drop_targets`) anew every frame; this layer
//! is updated before them and empties the list for the next round.

use ferrite::core::{
    assets::AssetRef,
    entity::EntityHandle,
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, theme, UIElement};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{layout::Rect, send, Command, Shared};

/// Cursor travel (px) after which a press on an asset becomes a drag.
const DRAG_THRESHOLD: f32 = 5.0;
const GHOST_HEIGHT: f32 = 26.0;

/// An asset picked up in the browser.
#[derive(Clone, Debug)]
pub struct AssetDrag {
    pub asset: AssetRef,
    /// Shown next to the cursor.
    pub label: String,
    /// Whether the asset is a model (targets only take what fits them).
    pub is_model: bool,
    /// Where the mouse button went down.
    pub origin: (f32, f32),
    /// Where the cursor is now.
    pub position: (f32, f32),
    /// False until the cursor has moved far enough from `origin`.
    pub active: bool,
}

/// What dropping an asset on a target does.
#[derive(Clone, Debug, PartialEq)]
pub enum DropAction {
    /// Use a model asset for a scatter layer of this entity's terrain; a
    /// new layer without an index.
    ScatterLayer {
        entity: EntityHandle,
        layer: Option<usize>,
    },
}

impl DropAction {
    pub fn accepts(&self, drag: &AssetDrag) -> bool {
        match self {
            DropAction::ScatterLayer { .. } => drag.is_model,
        }
    }

    fn command(&self, asset: AssetRef) -> Command {
        match self {
            DropAction::ScatterLayer { entity, layer } => Command::SetScatterAsset {
                entity: *entity,
                layer: *layer,
                asset,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropTarget {
    pub rect: Rect,
    pub action: DropAction,
}

/// The target at this point that takes the dragged asset. Where targets
/// overlap (a row inside a panel that is a target as a whole) the smallest
/// one wins.
pub fn target_at<'a>(
    targets: &'a [DropTarget],
    drag: &AssetDrag,
    x: f32,
    y: f32,
) -> Option<&'a DropTarget> {
    targets
        .iter()
        .filter(|target| target.rect.contains(x, y) && target.action.accepts(drag))
        .min_by(|a, b| {
            let area = |t: &DropTarget| t.rect.w * t.rect.h;
            area(a).total_cmp(&area(b))
        })
}

/// End the current drag, if there is one: queue the command of the target
/// under the cursor. Returns whether an asset was actually being dragged
/// (as opposed to just pressed).
pub fn finish_drag(state: &Shared) -> bool {
    let drag = state.borrow_mut().drag.take();
    let Some(drag) = drag.filter(|drag| drag.active) else {
        return false;
    };
    let (x, y) = drag.position;
    let command = {
        let state = state.borrow();
        target_at(&state.drop_targets, &drag, x, y)
            .map(|target| target.action.command(drag.asset.clone()))
    };
    if let Some(command) = command {
        send(state, command);
    }
    true
}

pub struct DragLayer {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    ghost: Plane,
    ghost_icon: Text,
    ghost_text: Text,
    ghost_label: String,
    highlight: Plane,
    /// Whether there is a target under the cursor to highlight.
    over_target: bool,
}

impl DragLayer {
    pub fn new(state: Shared) -> Self {
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::default(),
            ghost: PlaneBuilder::new()
                .color(theme::BG_MENU)
                .border_color(theme::ACCENT)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS_SMALL)
                .build(),
            ghost_icon: theme::icon(icons::PACKAGE, theme::ICON_SIZE, theme::ACCENT_TEXT),
            ghost_text: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT),
            ghost_label: String::new(),
            highlight: PlaneBuilder::new()
                .color(theme::ACCENT_SOFT)
                .border_color(theme::ACCENT)
                .border_thickness(1.5)
                .border_radius_uniform(theme::RADIUS_SMALL)
                .build(),
            over_target: false,
        }
    }

    fn active_drag(&self) -> Option<AssetDrag> {
        self.state.borrow().drag.clone().filter(|drag| drag.active)
    }
}

impl UIElement for DragLayer {
    fn update(&mut self, _scene: &mut Scene) {
        // The targets seen here are the ones the panels registered during
        // the previous frame. They are taken out so the panels, which are
        // updated after this layer, start from an empty list.
        let targets = std::mem::take(&mut self.state.borrow_mut().drop_targets);
        self.over_target = false;
        let Some(drag) = self.active_drag() else {
            return;
        };
        let (x, y) = drag.position;
        if drag.label != self.ghost_label {
            self.ghost_text.set_content(&drag.label);
            self.ghost_label = drag.label.clone();
        }
        let width = 30.0 + self.ghost_text.measure_width() + 10.0;
        // Below and right of the cursor, so the target stays visible.
        let (gx, gy) = (x + 14.0, y + 12.0);
        self.ghost.set_position(Position::new(gx, gy, 90.0));
        self.ghost.set_size(Size::new(width, GHOST_HEIGHT));
        self.ghost_icon.prepare_render_at(Position::new(
            gx + 7.0,
            gy + theme::center_y(&self.ghost_icon, GHOST_HEIGHT),
            91.0,
        ));
        self.ghost_text.prepare_render_at(Position::new(
            gx + 30.0,
            gy + theme::center_y(&self.ghost_text, GHOST_HEIGHT),
            91.0,
        ));

        if let Some(target) = target_at(&targets, &drag, x, y) {
            let r = target.rect;
            self.highlight.set_position(Position::new(r.x, r.y, 89.0));
            self.highlight.set_size(Size::new(r.w, r.h));
            self.over_target = true;
        }
    }

    fn render(&self) {}

    fn render_overlay(&self) {
        if self.active_drag().is_none() {
            return;
        }
        if self.over_target {
            self.highlight.render();
        }
        self.ghost.render();
        self.ghost_icon.render();
        self.ghost_text.render();
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        _window: &mut glfw::Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            WindowEvent::CursorPos(x, y) => {
                let mut state = self.state.borrow_mut();
                if let Some(drag) = state.drag.as_mut() {
                    drag.position = (*x as f32, *y as f32);
                    let (dx, dy) = (drag.position.0 - drag.origin.0, drag.position.1 - drag.origin.1);
                    if !drag.active && dx * dx + dy * dy > DRAG_THRESHOLD * DRAG_THRESHOLD {
                        drag.active = true;
                    }
                }
                // Everything below still gets the move, for hover states.
                false
            }
            // A plain click is left to whoever was clicked; the release that
            // ends a drag is not passed on.
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                finish_drag(&self.state)
            }
            _ => false,
        }
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, _z: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite::core::assets::AssetId;

    fn drag(is_model: bool) -> AssetDrag {
        AssetDrag {
            asset: AssetRef::whole(AssetId::from(1)),
            label: "pack".into(),
            is_model,
            origin: (0.0, 0.0),
            position: (0.0, 0.0),
            active: true,
        }
    }

    fn target(x: f32, y: f32, w: f32, h: f32, layer: Option<usize>) -> DropTarget {
        DropTarget {
            rect: Rect { x, y, w, h },
            action: DropAction::ScatterLayer {
                entity: EntityHandle::from(9),
                layer,
            },
        }
    }

    #[test]
    fn the_smallest_target_under_the_cursor_wins() {
        // The whole panel adds a layer; a row inside it replaces one.
        let targets = vec![
            target(0.0, 0.0, 300.0, 400.0, None),
            target(10.0, 100.0, 280.0, 26.0, Some(1)),
        ];
        let on_row = target_at(&targets, &drag(true), 50.0, 110.0).unwrap();
        assert_eq!(on_row, &targets[1]);
        let beside = target_at(&targets, &drag(true), 50.0, 300.0).unwrap();
        assert_eq!(beside, &targets[0]);
        assert!(target_at(&targets, &drag(true), 500.0, 110.0).is_none());
    }

    #[test]
    fn a_texture_cannot_be_dropped_on_a_scatter_layer() {
        let targets = vec![target(0.0, 0.0, 300.0, 400.0, None)];
        assert!(target_at(&targets, &drag(false), 50.0, 50.0).is_none());
    }

    #[test]
    fn dropping_on_a_layer_sets_that_layer() {
        let asset = AssetRef::part(AssetId::from(1), "Oak");
        let command = target(0.0, 0.0, 1.0, 1.0, Some(2)).action.command(asset.clone());
        match command {
            Command::SetScatterAsset {
                entity,
                layer,
                asset: dropped,
            } => {
                assert_eq!(entity, EntityHandle::from(9));
                assert_eq!(layer, Some(2));
                assert_eq!(dropped, asset);
            }
            other => panic!("unexpected command {other:?}"),
        }
    }
}
