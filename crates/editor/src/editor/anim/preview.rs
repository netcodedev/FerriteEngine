//! Live preview of the edited animation graph: the player model driven by
//! the graph, with sliders for its inputs and an orbit camera.

use std::{collections::HashMap, time::Instant};

use cgmath::{perspective, Deg, Matrix4, Point3, SquareMatrix, Vector3};
use ferrite::{
    core::{
        model::{
            animation_graph::{graph_file::AnimGraphFile, AnimationGraph},
            Model, ModelBuilder,
        },
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            text::{icons, Text},
        },
        scene::Scene,
        utils::DataSource,
    },
    player::build_animation_graph,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
    },
    theme, UIElement, UI,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::super::{field_row::FieldRow, layout::Rect, Shared};

const MODEL_FILE: &str = "Mannequin.fbx";
/// The player's feet sit this far below the entity origin (capsule half
/// height + radius), so the floor is drawn there.
const FLOOR_Y: f32 = -1.0;

struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    dragging: Option<(f64, f64)>,
}

pub struct PreviewPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    controls: ScrollArea,
    reset: Button,
    pause: Button,
    status: Text,
    status_detail: Text,
    model: Option<Model>,
    model_error: Option<String>,
    graph: Option<AnimationGraph>,
    built_revision: Option<u64>,
    /// Content of the last built graph, without node positions (moving a
    /// node must not restart the preview).
    built_file: Option<AnimGraphFile>,
    inputs_built: Option<(u64, u64)>,
    input_values: HashMap<String, DataSource<f32>>,
    input_order: Vec<String>,
    paused: DataSource<bool>,
    reset_requested: DataSource<bool>,
    last_frame: Instant,
    orbit: Orbit,
}

fn button(label: &str, icon: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), on_click);
    button.set_style(ButtonStyle::Secondary);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(26.0);
    button
}

impl PreviewPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Preview".into(), Position::default(), Size::new(360.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut controls = ScrollArea::new(Position::default(), Size::new(340.0, 200.0));
        controls.set_gap(4.0);
        let paused = DataSource::new(false);
        let reset_requested = DataSource::new(false);
        let reset_flag = reset_requested.clone();
        let pause_flag = paused.clone();
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            controls,
            reset: button("Reset", icons::ROTATE_CW, Box::new(move |_| reset_flag.write(true))),
            pause: button(
                "Pause",
                icons::SQUARE,
                Box::new(move |_| pause_flag.write(!pause_flag.read())),
            ),
            status: theme::heading("", theme::FONT_SIZE, theme::TEXT),
            status_detail: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED),
            model: None,
            model_error: None,
            graph: None,
            built_revision: None,
            built_file: None,
            inputs_built: None,
            input_values: HashMap::new(),
            input_order: Vec::new(),
            paused,
            reset_requested,
            last_frame: Instant::now(),
            orbit: Orbit {
                yaw: 0.6,
                pitch: 0.18,
                distance: 4.2,
                dragging: None,
            },
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.stats
    }

    /// Square-ish 3D area at the top of the panel.
    fn view_rect(&self) -> Rect {
        let r = self.rect();
        let header = self.panel.header_height();
        let w = (r.w - 16.0).max(1.0);
        let h = w.min(r.h * 0.48).max(1.0);
        Rect {
            x: r.x + 8.0,
            y: r.y + header + 8.0,
            w,
            h,
        }
    }

    fn ensure_model(&mut self) {
        if self.model.is_some() || self.model_error.is_some() {
            return;
        }
        match ModelBuilder::new(MODEL_FILE) {
            Ok(builder) => {
                let mut model = builder.build();
                model.init();
                self.model = Some(model);
            }
            Err(e) => self.model_error = Some(format!("Could not load {MODEL_FILE}: {e}")),
        }
    }

    fn rebuild_graph(&mut self) {
        let (revision, mut file) = {
            let s = self.state.borrow();
            (s.anim.revision, s.anim.file.clone())
        };
        if self.built_revision == Some(revision) {
            return;
        }
        self.built_revision = Some(revision);
        for state in file.states.iter_mut() {
            state.position = (0.0, 0.0);
        }
        if self.built_file.as_ref() == Some(&file) {
            return;
        }
        let playing = self.graph.as_ref().map(|g| g.current_state().to_owned());
        match build_animation_graph(&file) {
            Ok(mut graph) => {
                for (name, value) in &self.input_values {
                    graph.set_input(name, value.read());
                }
                // Keep showing the state that was playing, if it still exists.
                if let Some(name) = playing {
                    graph.set_state(&name);
                }
                self.graph = Some(graph);
                self.state.borrow_mut().anim.preview.error = None;
            }
            Err(e) => {
                self.graph = None;
                self.state.borrow_mut().anim.preview.error = Some(e.to_string());
            }
        }
        self.built_file = Some(file);
    }

    fn rebuild_controls(&mut self) {
        let (structure, labels, inputs, width) = {
            let s = self.state.borrow();
            (
                s.anim.structure,
                s.anim.labels,
                s.anim.file.inputs.clone(),
                s.layout.stats.w,
            )
        };
        if self.inputs_built == Some((structure, labels)) {
            return;
        }
        self.inputs_built = Some((structure, labels));
        self.controls.clear();
        self.input_order.clear();
        let slider_w = (width - 16.0 - 96.0 - 64.0 - 24.0).max(60.0);
        self.controls.add_child(Box::new(Label::muted(
            "Inputs drive the graph like the game would.",
            theme::FONT_SIZE_SMALL + 0.5,
        )));
        for input in &inputs {
            let source = self
                .input_values
                .entry(input.name.clone())
                .or_insert_with(|| DataSource::new(input.default))
                .clone();
            self.controls.add_child(Box::new(FieldRow::new(
                &input.name,
                vec![
                    UI::slider_sized(source.clone(), 0.0, 1.0, Size::new(slider_w, 20.0)),
                    UI::input_sized(source, Size::new(58.0, 24.0)),
                ],
            )));
            self.input_order.push(input.name.clone());
        }
        self.controls.set_z_index(2.0);
    }

    fn view_projection(&self, view: &Rect) -> Matrix4<f32> {
        let o = &self.orbit;
        let target = Point3::new(0.0, FLOOR_Y + 0.95, 0.0);
        let eye = target
            + Vector3::new(
                o.distance * o.pitch.cos() * o.yaw.sin(),
                o.distance * o.pitch.sin(),
                o.distance * o.pitch.cos() * o.yaw.cos(),
            );
        let projection = perspective(Deg(38.0), view.w / view.h.max(1.0), 0.05, 100.0);
        projection * Matrix4::look_at_rh(eye, target, Vector3::unit_y())
    }
}

impl UIElement for PreviewPanel {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if rect.w <= 0.0 {
            return;
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        self.ensure_model();
        self.rebuild_graph();
        self.rebuild_controls();

        // Drive the graph.
        if self.reset_requested.read() {
            self.reset_requested.write(false);
            if let Some(graph) = &mut self.graph {
                graph.reset();
            }
        }
        let paused = self.paused.read();
        if let Some(graph) = &mut self.graph {
            for name in &self.input_order {
                if let Some(value) = self.input_values.get(name) {
                    graph.set_input(name, value.read());
                }
            }
            if !paused {
                graph.update(dt);
            }
            if let (Some(pose), Some(model)) = (graph.get_pose(), &mut self.model) {
                model.apply_pose(&pose);
                // Keep the preview in place; the game turns this root motion
                // into velocity.
                model.reset_position();
            }
            let status = &mut self.state.borrow_mut().anim.preview;
            status.current = graph.current_state().to_owned();
            status.previous = graph.previous_state().map(str::to_owned);
            status.progress = graph.transition_progress();
        }

        // Status line and buttons under the 3D view.
        let view = self.view_rect();
        let preview = self.state.borrow().anim.preview.clone();
        let (headline, detail) = if let Some(error) = &self.model_error {
            ("Preview unavailable".to_owned(), error.clone())
        } else if let Some(error) = &preview.error {
            ("Graph error".to_owned(), error.clone())
        } else {
            (
                format!("State: {}", preview.current),
                match &preview.previous {
                    Some(previous) => format!(
                        "Blending from {previous}  {:.0}%",
                        preview.progress.clamp(0.0, 1.0) * 100.0
                    ),
                    None => if paused { "Paused".into() } else { "Playing".into() },
                },
            )
        };
        self.status.set_content(&headline);
        self.status_detail.set_content(&detail);
        let y = view.y + view.h + 10.0;
        self.status
            .prepare_render_at(Position::new(view.x + 2.0, y, 3.0));
        self.status_detail
            .prepare_render_at(Position::new(view.x + 2.0, y + 22.0, 3.0));
        self.pause.set_label(if paused { "Play" } else { "Pause" });
        self.pause.set_icon(if paused { icons::PLAY } else { icons::SQUARE });
        self.pause.fit(26.0);
        let mut bx = view.x + view.w;
        for b in [&mut self.pause, &mut self.reset] {
            bx -= b.get_size().width;
            b.set_offset(Offset { x: bx, y: y + 2.0 });
            bx -= 6.0;
        }

        let controls_top = y + 50.0;
        self.controls
            .set_position(Position::new(rect.x + 4.0, controls_top, 0.0));
        self.controls.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.y + rect.h - controls_top - 6.0).max(10.0),
        ));

        self.panel.update(scene);
        self.reset.update(scene);
        self.pause.update(scene);
        self.controls.update(scene);
    }

    fn render(&self) {
        let rect = self.rect();
        if rect.w <= 0.0 {
            return;
        }
        self.panel.render();
        let view = self.view_rect();
        let window_h = self.state.borrow().layout.window_h;
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
        let vp = self.view_projection(&view);

        // Floor grid.
        let mut grid = Vec::new();
        let extent = 3.0;
        let steps = 12;
        for i in 0..=steps {
            let t = -extent + 2.0 * extent * i as f32 / steps as f32;
            grid.push(Line::new(Point3::new(t, FLOOR_Y, -extent), Vector3::unit_z(), extent * 2.0));
            grid.push(Line::new(Point3::new(-extent, FLOOR_Y, t), Vector3::unit_x(), extent * 2.0));
        }
        LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);
        let axes_x = vec![Line::new(Point3::new(0.0, FLOOR_Y + 0.001, 0.0), Vector3::unit_x(), 0.5)];
        let axes_z = vec![Line::new(Point3::new(0.0, FLOOR_Y + 0.001, 0.0), Vector3::unit_z(), 0.5)];
        LineRenderer::render_lines(&vp, &axes_x, Vector3::new(0.8, 0.3, 0.3), false);
        LineRenderer::render_lines(&vp, &axes_z, Vector3::new(0.3, 0.45, 0.9), false);

        if let Some(model) = &self.model {
            unsafe {
                gl::Enable(gl::DEPTH_TEST);
            }
            let light = Point3::new(6.0, 10.0, 8.0);
            model.render(&light, &Matrix4::identity(), &vp);
        }
        unsafe {
            // Leave no 3D depth behind for the UI drawn over this region.
            gl::Clear(gl::DEPTH_BUFFER_BIT);
            gl::Disable(gl::SCISSOR_TEST);
            let s = self.state.borrow();
            gl::Viewport(0, 0, s.layout.window_w as i32, s.layout.window_h as i32);
        }

        self.status.render();
        self.status_detail.render();
        self.reset.render();
        self.pause.render();
        self.controls.render();
    }

    fn render_overlay(&self) {
        self.controls.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let rect = self.rect();
        if rect.w <= 0.0 {
            return false;
        }
        for b in [&mut self.reset, &mut self.pause] {
            if b.handle_events(scene, window, glfw, event) && !matches!(event, WindowEvent::CursorPos(..)) {
                return true;
            }
        }
        if self.controls.handle_events(scene, window, glfw, event) {
            return true;
        }
        let view = self.view_rect();
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.orbit.dragging = Some((x, y));
                    return true;
                }
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.orbit.dragging.take().is_some() {
                    return true;
                }
            }
            WindowEvent::CursorPos(x, y) => {
                if let Some((lx, ly)) = self.orbit.dragging {
                    self.orbit.yaw -= (*x - lx) as f32 * 0.01;
                    self.orbit.pitch = (self.orbit.pitch + (*y - ly) as f32 * 0.01).clamp(-0.2, 1.4);
                    self.orbit.dragging = Some((*x, *y));
                    return true;
                }
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.orbit.distance = (self.orbit.distance * (1.0 - *dy as f32 * 0.1)).clamp(1.5, 12.0);
                    return true;
                }
            }
            _ => {}
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return rect.contains(x as f32, y as f32);
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
    fn set_z_index(&mut self, z: f32) {
        self.panel.set_z_index(z);
        self.reset.set_z_index(z + 3.0);
        self.pause.set_z_index(z + 3.0);
        self.status.set_z_index(z + 3.0);
        self.status_detail.set_z_index(z + 3.0);
        self.controls.set_z_index(z + 2.0);
    }
}
