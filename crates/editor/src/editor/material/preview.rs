//! The material workspace's preview: the material on a sphere, cube,
//! plane or cylinder, lit by the preview sun, turned by dragging. For a
//! material it fills the main area; for a graph it sits in the right
//! column beside the canvas. The wind can be let loose on it, to see what
//! a material's World Offset does.

use std::{cell::RefCell, collections::HashMap};

use cgmath::{Matrix4, Point3, SquareMatrix, Vector3};
use ferrite::core::{
    material::{preview::Shape, wind},
    model::Model,
    primitives::{Offset, Position, Size},
    renderer::{
        line::{Line, LineRenderer},
        pbr::Lighting,
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::super::{layout::Rect, Shared};

const HEADER_HEIGHT: f32 = 36.0;
const Z: f32 = 2.0;

thread_local! {
    /// The shapes, made once (on the render thread).
    static SHAPES: RefCell<HashMap<Shape, Model>> = RefCell::new(HashMap::new());
}

/// Where the preview goes: the main area for a material, the right column
/// for a graph.
pub fn preview_rect(state: &super::super::EditorState) -> Rect {
    let graph = state.material.as_ref().is_some_and(|e| e.is_graph());
    if graph { state.layout.stats } else { state.layout.viewport }
}

pub struct MaterialPreview {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    header: Plane,
    title: Text,
    note: Text,
    shapes: Vec<Button>,
    wind: Button,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    drag: Option<(f32, f32)>,
    shown: Option<(Shape, bool)>,
    /// The material to draw, made in `update` (loading models and
    /// uploading textures must not happen while a pass draws).
    material: Option<ferrite::core::material::Material>,
}

impl MaterialPreview {
    pub fn new(state: Shared) -> Self {
        let shapes = Shape::ALL
            .iter()
            .map(|shape| {
                let shape = *shape;
                let state = state.clone();
                let mut b = Button::new(
                    Position::default(),
                    Size::new(10.0, 24.0),
                    Box::new(move |_| {
                        if let Some(e) = state.borrow_mut().material.as_mut() {
                            e.shape = shape;
                        }
                    }),
                );
                b.set_label(shape.label());
                b.fit(24.0);
                b
            })
            .collect();
        let wind_state = state.clone();
        let mut wind = Button::new(
            Position::default(),
            Size::new(10.0, 24.0),
            Box::new(move |_| {
                if let Some(e) = wind_state.borrow_mut().material.as_mut() {
                    e.wind = if e.wind > 0.0 { 0.0 } else { 1.0 };
                }
            }),
        );
        wind.set_icon(icons::ACTIVITY);
        wind.set_label("Wind");
        wind.set_tooltip("Let the wind blow (shows the material's World Offset)");
        wind.fit(24.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::default(),
            header: PlaneBuilder::new().color(theme::BG_PANEL).border_color(theme::BORDER).border_thickness(1.0).build(),
            title: theme::heading("Vorschau", theme::FONT_SIZE, theme::TEXT),
            note: theme::text("Drag to turn, scroll to zoom", theme::FONT_SIZE_SMALL, theme::TEXT_DIM),
            shapes,
            wind,
            yaw: 35.0,
            pitch: 20.0,
            zoom: 1.0,
            drag: None,
            shown: None,
            material: None,
        }
    }

    fn rect(&self) -> Rect {
        preview_rect(&self.state.borrow())
    }

    fn view_rect(&self) -> Rect {
        let r = self.rect();
        Rect { x: r.x, y: r.y + HEADER_HEIGHT, w: r.w, h: (r.h - HEADER_HEIGHT - 22.0).max(1.0) }
    }

    fn open(&self) -> bool {
        self.state.borrow().material.is_some()
    }

    fn eye(&self, center: Point3<f32>, radius: f32) -> Point3<f32> {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        let d = radius * 3.2 * self.zoom;
        center + Vector3::new(y.cos() * p.cos(), p.sin(), y.sin() * p.cos()) * d
    }
}

impl UIElement for MaterialPreview {
    fn update(&mut self, scene: &mut Scene) {
        if !self.open() {
            return;
        }
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| ferrite::core::material::on_release(release));
        let r = self.rect();
        self.size = Size::new(r.w, r.h);
        self.header.set_position(Position::new(r.x, r.y, Z));
        self.header.set_size(Size::new(r.w, HEADER_HEIGHT));
        self.title.prepare_render_at(Position::new(r.x + 10.0, r.y + theme::center_y(&self.title, HEADER_HEIGHT), Z + 1.0));
        let by = r.y + (HEADER_HEIGHT - 24.0) * 0.5;
        let mut bx = r.x + r.w - 8.0;
        for button in std::iter::once(&mut self.wind).chain(self.shapes.iter_mut().rev()) {
            bx -= button.get_size().width;
            button.set_offset(Offset { x: bx, y: by });
            bx -= 4.0;
        }
        let (shape, wind_on) = {
            let s = self.state.borrow();
            s.material.as_ref().map_or((Shape::Sphere, false), |e| (e.shape, e.wind > 0.0))
        };
        if self.shown != Some((shape, wind_on)) {
            for (button, s) in self.shapes.iter_mut().zip(Shape::ALL) {
                button.set_style(if s == shape { ButtonStyle::Primary } else { ButtonStyle::Ghost });
            }
            self.wind.set_style(if wind_on { ButtonStyle::Primary } else { ButtonStyle::Ghost });
            self.shown = Some((shape, wind_on));
        }
        let note = self.state.borrow().material.as_ref().map(|e| e.auto_note()).unwrap_or_default();
        let note = if note.is_empty() { "Drag to turn, scroll to zoom".to_string() } else { format!("Drag to turn, scroll to zoom  \u{b7}  {note}") };
        self.note.set_content(&note);
        let v = self.view_rect();
        self.note.prepare_render_at(Position::new(v.x + 8.0, v.y + v.h + 3.0, Z + 1.0));
        self.material = self.state.borrow_mut().material.as_mut().and_then(|e| e.preview_material().cloned());
        // The shape, made here too (not while drawing).
        SHAPES.with(|shapes| {
            let mut shapes = shapes.borrow_mut();
            if !shapes.contains_key(&shape) {
                match shape.model() {
                    Ok(model) => {
                        shapes.insert(shape, model);
                    }
                    Err(e) => log::warn!("material preview: {e}"),
                }
            }
        });
        for button in self.shapes.iter_mut().chain(std::iter::once(&mut self.wind)) {
            button.update(scene);
        }
    }

    fn render(&self) {
        if !self.open() {
            return;
        }
        let view = self.view_rect();
        if view.w < 8.0 || view.h < 8.0 {
            return;
        }
        let (window_w, window_h) = {
            let s = self.state.borrow();
            (s.layout.window_w, s.layout.window_h)
        };
        // The material (made again after edits) and the shape.
        let (material, shape, wind_on) = {
            let s = self.state.borrow();
            let Some(edit) = s.material.as_ref() else { return };
            (self.material.clone(), edit.shape, edit.wind > 0.0)
        };
        let Some(material) = material else { return };
        let gl = [view.x as i32, (window_h - view.y - view.h) as i32, view.w as i32, view.h as i32];
        let bg = theme::BG_FIELD;
        ferrite::core::gfx::begin_region(
            ferrite::core::gfx::Rect::new(gl[0], gl[1], gl[2].max(0) as u32, gl[3].max(0) as u32),
            [bg.0, bg.1, bg.2, 1.0],
        );
        let (c, radius) = shape.framing();
        let center = Point3::new(c[0], c[1], c[2]);
        let eye = self.eye(center, radius);
        let projection = cgmath::perspective(cgmath::Deg(40.0), view.w / view.h.max(1.0), 0.02, 100.0);
        let vp = projection * Matrix4::look_at_rh(eye, center, Vector3::unit_y());

        // A floor grid.
        let extent = 1.5;
        let mut grid = Vec::new();
        for i in 0..=12 {
            let t = -extent + 2.0 * extent * i as f32 / 12.0;
            grid.push(Line::new(Point3::new(t, 0.0, -extent), Vector3::unit_z(), extent * 2.0));
            grid.push(Line::new(Point3::new(-extent, 0.0, t), Vector3::unit_x(), extent * 2.0));
        }
        LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);

        // The wind's clock: running, or held still.
        wind::set_clock(if wind_on { ferrite::core::renderer::weather::seconds() } else { 0.0 });
        let lighting = Lighting::preview(Vector3::new(3.0, 5.0, 4.0), eye);
        SHAPES.with(|shapes| {
            let mut shapes = shapes.borrow_mut();
            if let Some(model) = shapes.get_mut(&shape) {
                let mut material = material;
                // Without wind, nothing moves.
                if !wind_on {
                    material.set_float("wind", 0.0);
                    material.set_float("wind_flutter", 0.0);
                }
                // Every slot of the shape (a generated model may have more
                // than its one part's).
                for index in 0..model.materials().len() {
                    model.set_material(index, material.clone());
                }
                model.render(&lighting, &Matrix4::identity(), &vp);
            }
        });
        ferrite::core::gfx::end_region(ferrite::core::gfx::Rect::sized(window_w as u32, window_h as u32));
    }

    fn render_overlay(&self) {
        if !self.open() {
            return;
        }
        self.header.render();
        self.title.render();
        self.note.render();
        for button in self.shapes.iter().chain(std::iter::once(&self.wind)) {
            button.render();
        }
    }

    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        if !self.open() {
            return false;
        }
        for button in self.shapes.iter_mut().chain(std::iter::once(&mut self.wind)) {
            if button.handle_events(scene, window, glfw, event) && !matches!(event, WindowEvent::CursorPos(..)) {
                return true;
            }
        }
        let view = self.view_rect();
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.drag = Some((x as f32, y as f32));
                    return true;
                }
                false
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => self.drag.take().is_some(),
            WindowEvent::CursorPos(x, y) => {
                if let Some((lx, ly)) = self.drag {
                    let (x, y) = (*x as f32, *y as f32);
                    self.yaw += (x - lx) * 0.4;
                    self.pitch = (self.pitch + (y - ly) * 0.3).clamp(-80.0, 85.0);
                    self.drag = Some((x, y));
                    return true;
                }
                false
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.zoom = (self.zoom * (1.0 - *dy as f32 * 0.1)).clamp(0.3, 4.0);
                    return true;
                }
                false
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
    fn set_z_index(&mut self, z: f32) {
        self.header.set_z_index(z);
    }
}

/// Free the shapes while the context is alive.
pub fn release() {
    let _ = SHAPES.try_with(|s| s.borrow_mut().clear());
}
