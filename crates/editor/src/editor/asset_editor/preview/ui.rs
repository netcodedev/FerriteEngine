//! The preview as a UI element: drawing it and handling its input.

use std::time::Instant;

use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Transform, Vector3};
use ferrite::{
    core::{
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            pbr::Lighting,
            text::icons,
        },
        scene::Scene,
    },
    terrain::scatter::LodChoice,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, theme, UIElement};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use crate::editor::clip_editor::{ring_points, TimelineView, AXES, RING_COLORS};

use super::*;

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
            let since = settled_since(&mut self.pending_since, wanted_key, Instant::now());
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

        self.clip_mode = matches!(self.shown, Shown::Rigged(_))
            && self
                .state
                .borrow()
                .asset_edit
                .as_ref()
                .is_some_and(|edit| edit.clip.is_some());
        if self.clip_mode {
            self.update_clip();
            let view = {
                let state = self.state.borrow();
                state
                    .asset_edit
                    .as_ref()
                    .and_then(|edit| edit.clip.as_ref())
                    .map(|clip| TimelineView::of(clip, &clip.bones))
                    .unwrap_or_default()
            };
            let rect = self.timeline_rect();
            self.timeline.update(scene, rect, view, self.z + 2.0);
        }
        if let Shown::Held(held) = &mut self.shown {
            let state = self.state.borrow();
            if let Some(edit) = &state.asset_edit {
                held.set_pose(edit.hold_pose, &edit.hold_poses);
                if let Some(grip) = edit.grip() {
                    held.animate(grip);
                }
            }
        }
        let view = self.view_rect();
        let message = match &self.shown {
            Shown::Nothing => "No asset is open.".to_string(),
            Shown::Failed(error) => format!("The asset could not be loaded: {error}"),
            Shown::Note(note) => note.clone(),
            Shown::Texture(image) => image
                .natural_size()
                .map(|(w, h)| format!("{w} x {h} px"))
                .unwrap_or_default(),
            Shown::Model { .. } | Shown::Held(_) => String::new(),
            Shown::Rigged(rigged) if rigged.chains.is_empty() => {
                "No feet, hands or head found among the bones to pose.".to_string()
            }
            Shown::Rigged(_) => String::new(),
        };
        if self.reset_pose.read() {
            self.reset_pose.write(false);
            self.ik_drag = None;
            self.ring_drag = None;
            if let Shown::Rigged(rigged) = &mut self.shown {
                let start = rigged.skeleton.locals();
                rigged.skeleton.reset();
                rigged.model.apply_skeleton(&rigged.skeleton);
                // In a clip, going back to the rest pose is posing too.
                self.finish_pose(&start);
            }
        }
        if matches!(self.shown, Shown::Rigged(_)) {
            let shown = self.show_skeleton.read();
            self.skeleton_button.set_active(shown);
            self.skeleton_button
                .set_icon(if shown { icons::EYE } else { icons::EYE_OFF });
            let mut x = view.x + 8.0;
            for button in [&mut self.skeleton_button, &mut self.reset_button] {
                button.set_offset(Offset { x, y: view.y + 8.0 });
                x += button.get_size().width + 4.0;
                button.update(scene);
            }
        }
        self.message.set_content(&message);
        self.message
            .prepare_render_at(Position::new(view.x + 16.0, view.y + 16.0, 3.0));
        self.hint.prepare_render_at(Position::new(
            view.x + 12.0,
            view.y + view.h - 24.0,
            3.0,
        ));
        if let Shown::Texture(image) = &mut self.shown {
            // As large as fits, keeping the image's proportions.
            let (w, h) = image.natural_size().unwrap_or((1, 1));
            let (w, h) = (w.max(1) as f32, h.max(1) as f32);
            let room = ((view.w - 48.0).max(16.0), (view.h - 72.0).max(16.0));
            let scale = (TEXTURE_SIDE / w.max(h)).min(room.0 / w).min(room.1 / h);
            let (w, h) = ((w * scale).round(), (h * scale).round());
            image.set_size(Size::new(w, h));
            image.set_offset(Offset {
                x: (view.x + (view.w - w) * 0.5).round(),
                y: (view.y + (view.h - h) * 0.5).round(),
            });
        }
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

                let lighting = Lighting::preview(
                    Vector3::new(3.0, 5.0, 4.0),
                    self.eye(*center, *radius),
                );
                let lod = match lod {
                    Some(tier) => LodChoice::Fixed(tier),
                    // Close up, as the camera never gets far in model terms.
                    None => LodChoice::Distance(0.0),
                };
                instances.draw(&vp, &Matrix4::identity(), &lighting, lod);
                unsafe {
                    // Leave no 3D depth behind for the UI drawn over this
                    // region.
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                self.hint.render();
            }
            Shown::Rigged(rigged) => {
                let (window_w, window_h) = {
                    let state = self.state.borrow();
                    (state.layout.window_w, state.layout.window_h)
                };
                let (center, radius) = (rigged.center, rigged.radius);
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    gl::Viewport(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, center, radius);
                let extent = radius * 1.6;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(
                        Point3::new(center.x + t, rigged.floor, center.z - extent),
                        Vector3::unit_z(),
                        extent * 2.0,
                    ));
                    grid.push(Line::new(
                        Point3::new(center.x - extent, rigged.floor, center.z + t),
                        Vector3::unit_x(),
                        extent * 2.0,
                    ));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);
                unsafe { gl::Enable(gl::DEPTH_TEST) };
                let lighting =
                    Lighting::preview(Vector3::new(3.0, 5.0, 4.0), self.eye(center, radius));
                rigged.model.render(&lighting, &Matrix4::identity(), &vp);
                if self.show_skeleton.read() {
                    let hot = self.ik_drag.as_ref().map(|d| d.chain).or(self.hovered);
                    render_skeleton(rigged, &vp, &view, hot);
                    let positions = rigged.skeleton.world_positions(&rigged.root());
                    let size = rigged.radius * 0.012;
                    if let Some(&at) = self.hovered_joint.and_then(|joint| positions.get(joint)) {
                        render_thick_lines(&vp, &diamond(at, size * 1.6), HANDLE_HOT_COLOR, &view, 2);
                    }
                    if let Some(joint) = self.selected_joint().filter(|joint| *joint < positions.len()) {
                        render_thick_lines(&vp, &diamond(positions[joint], size * 2.0), SELECTED_COLOR, &view, 2);
                        let radius = Self::ring_radius(rigged);
                        let hot = self.ring_drag.as_ref().map(|d| d.axis).or(self.hovered_ring);
                        for (axis, dir) in AXES.iter().enumerate() {
                            let points = ring_points(positions[joint], *dir, radius);
                            let lines: Vec<Line> =
                                points.windows(2).filter_map(|w| segment(w[0], w[1])).collect();
                            let (r, g, b) = RING_COLORS[axis];
                            let color = if Some(axis) == hot {
                                (1.0, 0.9, 0.4)
                            } else {
                                (r, g, b)
                            };
                            render_thick_lines(&vp, &lines, color, &view, 2);
                        }
                    }
                }
                unsafe {
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                if self.clip_mode {
                    self.timeline.render((window_w, window_h));
                }
                self.hint.render();
                self.message.render();
                self.skeleton_button.render();
                self.reset_button.render();
            }
            Shown::Held(held) => {
                let (window_w, window_h) = {
                    let state = self.state.borrow();
                    (state.layout.window_w, state.layout.window_h)
                };
                let (center, radius) = (held.center, held.radius);
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    gl::Viewport(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, center, radius);
                let floor = held.player.floor;
                let extent = held.player.radius * 1.2;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(Point3::new(t, floor, -extent), Vector3::unit_z(), extent * 2.0));
                    grid.push(Line::new(Point3::new(-extent, floor, t), Vector3::unit_x(), extent * 2.0));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);
                unsafe { gl::Enable(gl::DEPTH_TEST) };
                let lighting = Lighting::preview(Vector3::new(3.0, 5.0, 4.0), self.eye(center, radius));
                held.player.model.render(&lighting, &Matrix4::identity(), &vp);
                if let Some(grip) = self.grip() {
                    if let Some(placed) = held.placed(&grip) {
                        held.item.render(&lighting, &placed, &vp);
                        let size = 0.012;
                        let hot = match &self.grip_drag {
                            Some(GripDrag::Item { .. }) => Some(0),
                            Some(GripDrag::OtherHand { .. }) => Some(1),
                            None => self.hovered_grip,
                        };
                        if let Some((point, other)) = Self::grip_handles(held, &grip) {
                            let color = if hot == Some(0) { HANDLE_HOT_COLOR } else { HANDLE_COLOR };
                            render_thick_lines(&vp, &diamond(point, size * 1.4), color, &view, 2);
                            // Towards the head and the edge.
                            let head = placed.transform_vector(Vector3::from(grip.along).normalize());
                            if let Some(line) = segment(point, point + head * 0.15) {
                                render_thick_lines(&vp, &vec![line], (1.0, 0.55, 0.2), &view, 2);
                            }
                            if let Some(other) = other {
                                let color = if hot == Some(1) { HANDLE_HOT_COLOR } else { BONE_COLOR };
                                render_thick_lines(&vp, &diamond(other, size * 1.4), color, &view, 2);
                            }
                        }
                    }
                }
                unsafe {
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                self.hint.render();
                self.message.render();
            }
            Shown::Texture(image) => {
                image.render();
                self.message.render();
            }
            Shown::Nothing | Shown::Note(_) | Shown::Failed(_) => {
                self.message.render();
            }
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let view = self.view_rect();
        if self.clip_mode {
            let (used, action) = self.timeline.handle_events(scene, window, glfw, event);
            if let Some(action) = action {
                self.timeline_action(action);
            }
            if used {
                return true;
            }
        }
        if matches!(self.shown, Shown::Rigged(_)) {
            for button in [&mut self.skeleton_button, &mut self.reset_button] {
                if button.handle_events(scene, window, glfw, event)
                    && !matches!(event, WindowEvent::CursorPos(..))
                {
                    return true;
                }
            }
        }
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    let (fx, fy) = (x as f32, y as f32);
                    if self.begin_grip_drag(fx, fy)
                        || self.begin_ring_drag(fx, fy)
                        || self.begin_ik_drag(fx, fy)
                    {
                        return true;
                    }
                    let joint = match &self.shown {
                        Shown::Rigged(rigged) if self.show_skeleton.read() => {
                            self.joint_at(rigged, &view, fx, fy)
                        }
                        _ => None,
                    };
                    match joint {
                        Some(joint) => self.select_joint(Some(joint)),
                        None => self.orbit.dragging = Some((x, y)),
                    }
                    return true;
                }
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.grip_drag.take().is_some() {
                    // The grip's fields show the new values.
                    if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                        edit.structure += 1;
                    }
                    return true;
                }
                if let Some(drag) = self.ik_drag.take() {
                    if drag.moved {
                        self.finish_pose(&drag.start);
                    } else if let Shown::Rigged(rigged) = &self.shown {
                        // A click on a handle selects its bone.
                        let end = rigged.chains[drag.chain].end;
                        self.select_joint(Some(end));
                    }
                    return true;
                }
                if let Some(drag) = self.ring_drag.take() {
                    self.finish_pose(&drag.start);
                    return true;
                }
                if self.orbit.dragging.take().is_some() {
                    return true;
                }
            }
            WindowEvent::CursorPos(x, y) => {
                if self.grip_drag.is_some() {
                    self.continue_grip_drag(*x as f32, *y as f32);
                    return true;
                }
                self.hovered_grip = match &self.shown {
                    Shown::Held(held)
                        if self.orbit.dragging.is_none() && view.contains(*x as f32, *y as f32) =>
                    {
                        self.grip_handle_at(held, &view, *x as f32, *y as f32)
                    }
                    _ => None,
                };
                if self.ik_drag.is_some() {
                    self.continue_ik_drag(*x as f32, *y as f32);
                    return true;
                }
                if self.ring_drag.is_some() {
                    self.continue_ring_drag(*x as f32, *y as f32);
                    return true;
                }
                let free = self.orbit.dragging.is_none() && view.contains(*x as f32, *y as f32);
                (self.hovered_ring, self.hovered_joint) = match &self.shown {
                    Shown::Rigged(rigged) if self.show_skeleton.read() && free => {
                        let ring = self.ring_at(rigged, &view, *x as f32, *y as f32);
                        let joint = if ring.is_none() {
                            self.joint_at(rigged, &view, *x as f32, *y as f32)
                        } else {
                            None
                        };
                        (ring, joint)
                    }
                    _ => (None, None),
                };
                self.hovered = match &self.shown {
                    Shown::Rigged(rigged)
                        if self.show_skeleton.read()
                            && self.orbit.dragging.is_none()
                            && view.contains(*x as f32, *y as f32) =>
                    {
                        self.handle_at(rigged, &view, *x as f32, *y as f32)
                    }
                    _ => None,
                };
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
    fn render_overlay(&self) {
        if self.clip_mode {
            self.timeline.render_overlay();
        }
    }

    fn set_z_index(&mut self, z: f32) {
        self.z = z;
        self.panel.set_z_index(z);
        self.message.set_z_index(z + 3.0);
        self.hint.set_z_index(z + 3.0);
        self.skeleton_button.set_z_index(z + 3.0);
        self.reset_button.set_z_index(z + 3.0);
    }
}
