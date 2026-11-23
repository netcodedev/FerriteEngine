//! Render statistics: collection (frame timings, GPU time, memory, scene
//! counts) and the docked "Performance" panel that shows them in sections,
//! with switches for the renderer's debug views (culling frozen, ambient
//! occlusion, the sun's shadow maps).

use std::{collections::VecDeque, time::Instant};

use ferrite::core::{
    view_frustum::{Culled, ViewFrustum},
    entity::Entity,
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        debug_views, profiler,
        text::{icons, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{panel::Panel, scroll::ScrollArea, section::Section, toggle::Toggle},
    theme, UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::{Mode, Shared};

/// Frames kept for the graph and min/max.
const HISTORY: usize = 120;
/// Text values refresh this often; per-frame text updates would cost more
/// than they tell.
const TEXT_REFRESH_SECONDS: f32 = 0.25;

/// Exponential moving average for jittery per-frame timings.
#[derive(Clone, Copy, Default)]
struct Smoothed(f32);

impl Smoothed {
    fn push(&mut self, value: f32) {
        self.0 = if self.0 == 0.0 {
            value
        } else {
            self.0 * 0.9 + value * 0.1
        };
    }
}

/// GPU time of a block of commands from two timestamps. A small ring of
/// query pairs is used so results are read frames later, without stalling
/// the pipeline.
pub struct GpuTimer {
    queries: Vec<ferrite::core::gfx::Queries>,
    issued: [bool; 4],
    next: usize,
    active: bool,
    last_ms: Option<f32>,
}

impl GpuTimer {
    pub fn new() -> Self {
        Self {
            queries: (0..4).map(|_| ferrite::core::gfx::Queries::new(2)).collect(),
            issued: [false; 4],
            next: 0,
            active: false,
            last_ms: None,
        }
    }

    pub fn begin(&mut self) {
        // Collect any finished results before reusing a query pair.
        for i in 0..self.queries.len() {
            if self.issued[i] {
                if let Some(times) = self.queries[i].read() {
                    let ns = times[1].saturating_sub(times[0]);
                    self.last_ms = Some(ns as f32 / 1_000_000.0);
                    self.issued[i] = false;
                }
            }
        }
        if self.issued[self.next] {
            // Still in flight; skip timing this frame rather than block.
            self.active = false;
            return;
        }
        self.queries[self.next].timestamp(0);
        self.active = true;
    }

    pub fn end(&mut self) {
        if !self.active {
            return;
        }
        self.queries[self.next].timestamp(1);
        self.issued[self.next] = true;
        self.next = (self.next + 1) % self.queries.len();
        self.active = false;
    }

    pub fn last_ms(&self) -> Option<f32> {
        self.last_ms
    }
}

#[derive(Default, Clone)]
pub struct SceneCounts {
    pub entities: usize,
    pub transient: usize,
    pub components: usize,
    pub rigid_bodies: usize,
    pub colliders: usize,
}

#[derive(Default, Clone)]
pub struct Memory {
    pub rss: Option<u64>,
    pub virtual_size: Option<u64>,
    pub system_total: Option<u64>,
    pub system_available: Option<u64>,
    /// `(used, total)` in bytes when the driver exposes it.
    pub gpu: Option<(u64, u64)>,
}

#[derive(Default, Clone)]
pub struct GlInfo {
    pub renderer: String,
    pub version: String,
}

/// Everything the statistics panel shows. Updated by the editor layer.
pub struct Stats {
    pub frame_ms: VecDeque<f32>,
    update_ms: Smoothed,
    render_ms: Smoothed,
    ui_ms: Smoothed,
    gpu_ms: Smoothed,
    /// Per pass, from the renderer's profiler.
    pass_ms: Option<[f32; profiler::PASSES]>,
    pub counts: SceneCounts,
    pub memory: Memory,
    pub gl: GlInfo,
    last_slow_refresh: Option<Instant>,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            frame_ms: VecDeque::with_capacity(HISTORY),
            update_ms: Smoothed::default(),
            render_ms: Smoothed::default(),
            ui_ms: Smoothed::default(),
            gpu_ms: Smoothed::default(),
            pass_ms: None,
            counts: SceneCounts::default(),
            memory: Memory::default(),
            gl: GlInfo::default(),
            last_slow_refresh: None,
        }
    }

    pub fn record_frame(&mut self, delta_seconds: f64) {
        if self.frame_ms.len() == HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(delta_seconds as f32 * 1000.0);
    }

    pub fn record_phases(&mut self, update_ms: f32, render_ms: f32, ui_ms: f32, gpu_ms: Option<f32>) {
        self.update_ms.push(update_ms);
        self.render_ms.push(render_ms);
        self.ui_ms.push(ui_ms);
        if let Some(gpu) = gpu_ms {
            self.gpu_ms.push(gpu);
        }
        self.pass_ms = profiler::pass_ms();
    }

    /// Scene counts, memory and GL strings change slowly; refresh them a
    /// couple of times per second only.
    pub fn refresh_slow(&mut self, scene: &Scene) {
        if self
            .last_slow_refresh
            .is_some_and(|t| t.elapsed().as_secs_f32() < 0.5)
        {
            return;
        }
        self.last_slow_refresh = Some(Instant::now());
        self.counts = count_scene(scene);
        self.memory = read_memory();
        if self.gl.renderer.is_empty() {
            self.gl = read_gl_info();
        }
    }

    pub fn fps(&self) -> f32 {
        let recent: Vec<f32> = self.frame_ms.iter().rev().take(30).copied().collect();
        if recent.is_empty() {
            return 0.0;
        }
        let avg = recent.iter().sum::<f32>() / recent.len() as f32;
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }

    /// The numbers the panel shows, for the MCP bridge (`editor_status`).
    pub fn to_json(&self) -> serde_json::Value {
        let (avg, min, max) = self.frame_avg_min_max();
        let passes: serde_json::Map<String, serde_json::Value> = match self.pass_ms {
            Some(ms) => profiler::Pass::ALL
                .iter()
                .zip(ms)
                .map(|(pass, ms)| (pass.label().to_string(), serde_json::json!(round(ms))))
                .collect(),
            None => serde_json::Map::new(),
        };
        serde_json::json!({
            "fps": round(self.fps()),
            "frame_ms": { "avg": round(avg), "min": round(min), "max": round(max) },
            "update_ms": round(self.update_ms.0),
            "render_cpu_ms": round(self.render_ms.0),
            "ui_ms": round(self.ui_ms.0),
            "gpu_ms": round(self.gpu_ms.0),
            "gpu_passes_ms": passes,
            "culling": Culled::ALL
                .iter()
                .zip(ViewFrustum::counts())
                .map(|(kind, count)| (format!("{kind:?}"), serde_json::json!({ "drawn": count.drawn, "total": count.total })))
                .collect::<serde_json::Map<_, _>>(),
        })
    }

    fn frame_avg_min_max(&self) -> (f32, f32, f32) {
        if self.frame_ms.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let sum: f32 = self.frame_ms.iter().sum();
        let min = self.frame_ms.iter().copied().fold(f32::MAX, f32::min);
        let max = self.frame_ms.iter().copied().fold(0.0, f32::max);
        (sum / self.frame_ms.len() as f32, min, max)
    }
}

/// Two decimals are all a reading tells.
fn round(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn count_scene(scene: &Scene) -> SceneCounts {
    fn visit(entity: &Entity, counts: &mut SceneCounts) {
        if entity.is_transient() {
            counts.transient += 1;
        } else {
            counts.entities += 1;
        }
        counts.components += entity.components().len();
        for child in entity.get_children() {
            visit(child, counts);
        }
    }
    let mut counts = SceneCounts::default();
    for entity in scene.get_entities() {
        visit(entity, &mut counts);
    }
    counts.rigid_bodies = scene.physics_engine.rigid_bodies.len();
    counts.colliders = scene.physics_engine.colliders.len();
    counts
}

fn read_kib_field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|l| l.starts_with(key))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

fn read_memory() -> Memory {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    Memory {
        rss: read_kib_field(&status, "VmRSS:"),
        virtual_size: read_kib_field(&status, "VmSize:"),
        system_total: read_kib_field(&meminfo, "MemTotal:"),
        system_available: read_kib_field(&meminfo, "MemAvailable:"),
        gpu: read_gpu_memory(),
    }
}

fn read_gpu_memory() -> Option<(u64, u64)> {
    ferrite::core::gfx::device().video_memory()
}

fn read_gl_info() -> GlInfo {
    use ferrite::core::gfx;
    let described = gfx::device().describe();
    let (renderer, version) = described.split_once(", ").unwrap_or((described.as_str(), ""));
    // "4.6.0 NVIDIA 615.78" (GL) or "Vulkan 1.4.351 (driver)": the API
    // and its version.
    let name = gfx::backend().name();
    let number = version.trim_start_matches(name).split_whitespace().next().unwrap_or("");
    GlInfo { renderer: renderer.to_string(), version: format!("{name} {number}").trim().to_string() }
}

fn bytes(value: Option<u64>) -> String {
    match value {
        None => tr!("common.not_available").into(),
        Some(b) if b >= 1 << 30 => format!("{:.2} GB", b as f64 / (1u64 << 30) as f64),
        Some(b) => format!("{:.0} MB", b as f64 / (1u64 << 20) as f64),
    }
}

fn ms(value: f32) -> String {
    format!("{value:.2} ms")
}

// --- widgets -----------------------------------------------------------

/// "label ......... value" row; the value text is fed through a source.
struct StatRow {
    handle: UIElementHandle,
    offset: Offset,
    size: Size,
    label: Text,
    value: Text,
    source: DataSource<String>,
    shown: String,
}

impl StatRow {
    fn new(label: &str, source: DataSource<String>, width: f32) -> Self {
        Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            size: Size::new(width, 22.0),
            label: theme::text(label, theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED),
            value: theme::mono("", theme::FONT_SIZE_SMALL, theme::TEXT),
            source,
            shown: String::new(),
        }
    }
}

impl UIElement for StatRow {
    fn update(&mut self, _scene: &mut Scene) {
        let value = self.source.read();
        if value != self.shown {
            self.value.set_content(&value);
            self.shown = value;
        }
        let y = self.offset.y;
        let ly = theme::center_y(&self.label, self.size.height);
        self.label
            .prepare_render_at(Position::new(self.offset.x + 4.0, y + ly, 3.0));
        let vx = self.offset.x + self.size.width - self.value.measure_width() - 6.0;
        let vy = theme::center_y(&self.value, self.size.height);
        self.value.prepare_render_at(Position::new(vx, y + vy, 3.0));
    }
    fn render(&self) {
        self.label.render();
        self.value.render();
    }
    fn handle_events(
        &mut self,
        _: &mut Scene,
        _: &mut glfw::Window,
        _: &mut Glfw,
        _: &WindowEvent,
    ) -> bool {
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z: f32) {
        self.label.set_z_index(z + 1.0);
        self.value.set_z_index(z + 1.0);
    }
}

/// The switches of the Performance panel: VSync and the renderer's debug
/// views (see `debug_views`, `ViewFrustum::set_frozen`). Shared by the
/// panel and the MCP bridge; `apply` hands them to the renderer.
pub struct DebugSwitches {
    pub vsync: DataSource<bool>,
    pub freeze_culling: DataSource<bool>,
    pub ambient_occlusion: DataSource<bool>,
    pub sun_shadow_maps: DataSource<bool>,
    pub lamp_shadows: DataSource<bool>,
}

impl Clone for DebugSwitches {
    /// The same switches (a `DataSource` clone shares its value).
    fn clone(&self) -> Self {
        Self {
            vsync: self.vsync.clone(),
            freeze_culling: self.freeze_culling.clone(),
            ambient_occlusion: self.ambient_occlusion.clone(),
            sun_shadow_maps: self.sun_shadow_maps.clone(),
            lamp_shadows: self.lamp_shadows.clone(),
        }
    }
}

impl DebugSwitches {
    pub fn new() -> Self {
        Self {
            // The window starts with it on.
            vsync: DataSource::new(true),
            freeze_culling: DataSource::new(false),
            ambient_occlusion: DataSource::new(false),
            sun_shadow_maps: DataSource::new(false),
            lamp_shadows: DataSource::new(false),
        }
    }

    /// Hand the switches as they are to the renderer and the window (whose
    /// GL context must be current).
    pub fn apply(&self) {
        set_vsync(self.vsync.read());
        ViewFrustum::set_frozen(self.freeze_culling.read());
        debug_views::set_ambient_occlusion(self.ambient_occlusion.read());
        debug_views::set_sun_shadows(self.sun_shadow_maps.read());
        debug_views::set_lamp_shadows(self.lamp_shadows.read());
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "vsync": self.vsync.read(),
            "freeze_culling": self.freeze_culling.read(),
            "ambient_occlusion": self.ambient_occlusion.read(),
            "sun_shadow_maps": self.sun_shadow_maps.read(),
            "lamp_shadows": self.lamp_shadows.read(),
        })
    }
}

/// The ids of the panel's sections, in order (see `section_title`).
pub fn section_titles() -> impl Iterator<Item = &'static str> {
    SECTIONS.iter().map(|(title, _, _)| *title)
}

/// What a `ToggleRow` does when switched: given the new state.
type OnToggle = Box<dyn FnMut(bool, &mut Glfw)>;

/// A row with a label and a switch (VSync, the debug views).
struct ToggleRow {
    handle: UIElementHandle,
    offset: Offset,
    size: Size,
    label: Text,
    toggle: Toggle,
    on: DataSource<bool>,
    changed: OnToggle,
}

impl ToggleRow {
    fn new(label: &str, on: DataSource<bool>, width: f32, changed: OnToggle) -> Self {
        Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            size: Size::new(width, 24.0),
            label: theme::text(label, theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED),
            toggle: Toggle::new(Position::default(), on.clone()),
            on,
            changed,
        }
    }
}

impl UIElement for ToggleRow {
    fn update(&mut self, scene: &mut Scene) {
        let y = self.offset.y;
        let ly = theme::center_y(&self.label, self.size.height);
        self.label.prepare_render_at(Position::new(self.offset.x + 4.0, y + ly, 3.0));
        self.toggle.set_offset(Offset { x: self.offset.x + self.size.width - 42.0, y: y + 3.0 });
        self.toggle.update(scene);
    }
    fn render(&self) {
        self.label.render();
        self.toggle.render();
    }
    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        let before = self.on.read();
        let used = self.toggle.handle_events(scene, window, glfw, event);
        if self.on.read() != before {
            (self.changed)(self.on.read(), glfw);
        }
        used
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
        self.label.set_z_index(z + 1.0);
        self.toggle.set_z_index(z + 1.0);
    }
}

/// Bar graph of recent frame times with a 60 fps reference line.
struct FrameGraph {
    handle: UIElementHandle,
    offset: Offset,
    size: Size,
    z: f32,
    state: Shared,
    background: Plane,
    target_line: Plane,
    bars: Vec<Plane>,
}

impl FrameGraph {
    fn new(state: Shared, width: f32) -> Self {
        Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            size: Size::new(width, 56.0),
            z: 0.0,
            state,
            background: PlaneBuilder::new()
                .color(theme::BG_FIELD)
                .border_color(theme::BORDER)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS_SMALL)
                .build(),
            target_line: PlaneBuilder::new()
                .color((1.0, 1.0, 1.0, 0.18))
                .build(),
            bars: (0..HISTORY)
                .map(|_| PlaneBuilder::new().color(theme::SUCCESS).build())
                .collect(),
        }
    }
}

impl UIElement for FrameGraph {
    fn update(&mut self, _scene: &mut Scene) {
        let state = self.state.borrow();
        let frames = &state.stats.frame_ms;
        let (x0, y0, z) = (self.offset.x + 4.0, self.offset.y, self.z);
        let (w, h) = (self.size.width - 8.0, self.size.height);
        self.background.set_position(Position::new(x0, y0, z));
        self.background.set_size(Size::new(w, h));
        // Scale: at least 33 ms (30 fps) so a smooth 60 fps sits mid-height.
        let peak = frames.iter().copied().fold(33.3f32, f32::max);
        let inner_h = h - 6.0;
        let target_y = y0 + 3.0 + inner_h * (1.0 - 16.67 / peak);
        self.target_line
            .set_position(Position::new(x0 + 1.0, target_y, z + 2.0));
        self.target_line.set_size(Size::new(w - 2.0, 1.0));
        let bar_w = (w - 6.0) / HISTORY as f32;
        let start = HISTORY - frames.len();
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let Some(ms) = i.checked_sub(start).and_then(|j| frames.get(j)) else {
                bar.set_size(Size::new(0.0, 0.0));
                continue;
            };
            let bh = (inner_h * (ms / peak)).clamp(1.0, inner_h);
            bar.set_position(Position::new(
                x0 + 3.0 + i as f32 * bar_w,
                y0 + 3.0 + inner_h - bh,
                z + 1.0,
            ));
            bar.set_size(Size::new((bar_w - 0.5).max(0.5), bh));
            bar.set_color(if *ms <= 17.5 {
                theme::SUCCESS
            } else if *ms <= 34.0 {
                theme::WARNING
            } else {
                theme::DANGER
            });
        }
    }
    fn render(&self) {
        // One draw call for the graph.
        Plane::render_all(
            std::iter::once(&self.background)
                .chain(&self.bars)
                .chain(std::iter::once(&self.target_line)),
        );
    }
    fn handle_events(
        &mut self,
        _: &mut Scene,
        _: &mut glfw::Window,
        _: &mut Glfw,
        _: &WindowEvent,
    ) -> bool {
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z: f32) {
        self.z = z;
    }
}

/// Every value row of the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    Fps,
    FrameTime,
    FrameMinMax,
    Update,
    Render,
    Gpu,
    Ui,
    CpuTotal,
    Rss,
    Virtual,
    System,
    Vram,
    Entities,
    Components,
    Chunks,
    Bodies,
    Colliders,
    GpuName,
    GlVersion,
    Window,
    Viewport,
    Mode,
    /// GPU time of one of the scene's passes (index into `Pass::ALL`).
    Pass(usize),
    /// What the camera's view culled of a kind (index into `Culled::ALL`).
    Culled(usize),
}

/// The sections: their ids (saved in the project when folded; shown
/// through `section_title`), icons and rows.
const SECTIONS: &[(&str, &str, &[Key])] = &[
    (
        "Frame rate",
        icons::GAUGE,
        &[
            Key::Fps,
            Key::FrameTime,
            Key::FrameMinMax,
        ],
    ),
    // The graphics settings, under the frame time they change (see
    // `graphics_panel`).
    (GRAPHICS_SECTION, icons::SLIDERS, &[]),
    (
        "Timings",
        icons::TIMER,
        &[
            Key::Update,
            Key::Render,
            Key::Gpu,
            Key::Ui,
            Key::CpuTotal,
        ],
    ),
    (
        "GPU passes",
        icons::TIMER,
        &[
            Key::Pass(0),
            Key::Pass(1),
            Key::Pass(2),
            Key::Pass(3),
            Key::Pass(4),
            Key::Pass(5),
            Key::Pass(6),
            Key::Pass(7),
        ],
    ),
    (
        "Culling (drawn / in reach)",
        icons::SCAN,
        &[
            Key::Culled(0),
            Key::Culled(1),
            Key::Culled(2),
        ],
    ),
    (
        "Debug views",
        icons::BUG,
        &[],
    ),
    (
        "Memory",
        icons::MEMORY_STICK,
        &[
            Key::Rss,
            Key::Virtual,
            Key::System,
            Key::Vram,
        ],
    ),
    (
        "Scene",
        icons::LAYERS,
        &[
            Key::Entities,
            Key::Components,
            Key::Chunks,
            Key::Bodies,
            Key::Colliders,
        ],
    ),
    (
        "Renderer",
        icons::MONITOR,
        &[
            Key::GpuName,
            Key::GlVersion,
            Key::Window,
            Key::Viewport,
            Key::Mode,
        ],
    ),
];

/// A section's title in the user's language.
fn section_title(id: &str) -> &'static str {
    match id {
        "Frame rate" => tr!("stats.section.frame_rate"),
        "Grafik" => tr!("stats.section.graphics"),
        "Timings" => tr!("stats.section.timings"),
        "GPU passes" => tr!("stats.section.gpu_passes"),
        "Culling (drawn / in reach)" => tr!("stats.section.culling"),
        "Debug views" => tr!("stats.section.debug_views"),
        "Memory" => tr!("stats.section.memory"),
        "Scene" => tr!("stats.section.scene"),
        "Renderer" => tr!("stats.section.renderer"),
        _ => "?",
    }
}

/// A row's label in the user's language.
fn row_label(key: Key) -> &'static str {
    match key {
        Key::Fps => tr!("stats.row.fps"),
        Key::FrameTime => tr!("stats.row.frame_time"),
        Key::FrameMinMax => tr!("stats.row.frame_min_max"),
        Key::Update => tr!("stats.row.update"),
        Key::Render => tr!("stats.row.render"),
        Key::Gpu => tr!("stats.row.gpu"),
        Key::Ui => tr!("stats.row.ui"),
        Key::CpuTotal => tr!("stats.row.cpu_total"),
        Key::Pass(0) => tr!("stats.row.pass_0"),
        Key::Pass(1) => tr!("stats.row.pass_1"),
        Key::Pass(2) => tr!("stats.row.pass_2"),
        Key::Pass(3) => tr!("stats.row.pass_3"),
        Key::Pass(4) => tr!("stats.row.pass_4"),
        Key::Pass(5) => tr!("stats.row.pass_5"),
        Key::Pass(6) => tr!("stats.row.pass_6"),
        Key::Pass(7) => tr!("stats.row.pass_7"),
        Key::Culled(0) => tr!("stats.row.culled_0"),
        Key::Culled(1) => tr!("stats.row.culled_1"),
        Key::Culled(2) => tr!("stats.row.culled_2"),
        Key::Rss => tr!("stats.row.rss"),
        Key::Virtual => tr!("stats.row.virtual"),
        Key::System => tr!("stats.row.system"),
        Key::Vram => tr!("stats.row.vram"),
        Key::Entities => tr!("stats.row.entities"),
        Key::Components => tr!("stats.row.components"),
        Key::Chunks => tr!("stats.row.chunks"),
        Key::Bodies => tr!("stats.row.bodies"),
        Key::Colliders => tr!("stats.row.colliders"),
        Key::GpuName => tr!("stats.row.gpu_name"),
        Key::GlVersion => tr!("stats.row.gl_version"),
        Key::Window => tr!("stats.row.window"),
        Key::Viewport => tr!("stats.row.viewport"),
        Key::Mode => tr!("stats.row.mode"),
        _ => "?",
    }
}

const GRAPHICS_SECTION: &str = "Grafik";

/// Docked right-hand panel with collapsible statistic sections.
pub struct StatsPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    values: Vec<(Key, DataSource<String>)>,
    /// Open/closed state per section (the editor's, kept in the project).
    open: Vec<DataSource<bool>>,
    /// VSync and the debug views (the editor's, shared with the bridge).
    switches: DebugSwitches,
    built_width: Option<i32>,
    last_refresh: Option<Instant>,
    graphics: super::graphics_panel::GraphicsRows,
}

impl StatsPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new(tr!("toolbar.window.performance").into(), Position::default(), Size::new(300.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(290.0, 300.0));
        body.set_gap(4.0);
        let (open, switches) = {
            let state = state.borrow();
            (state.stats_open.iter().map(DataSource::clone).collect(), state.debug.clone())
        };
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            values: Vec::new(),
            open,
            switches,
            built_width: None,
            last_refresh: None,
            graphics: Default::default(),
        }
    }

    /// (Re)create the sections for the current column width.
    fn build(&mut self, width: f32) {
        self.body.clear();
        self.values.clear();
        let content = (width - 24.0).max(160.0);
        for (index, (id, icon, rows)) in SECTIONS.iter().enumerate() {
            let mut section = Section::new(section_title(id), Some(icon), content);
            section.bind_open(self.open[index].clone());
            if index == 0 {
                section.add_child(Box::new(FrameGraph::new(self.state.clone(), content - 8.0)));
            }
            if *id == GRAPHICS_SECTION {
                let mut groups = std::mem::take(&mut self.state.borrow_mut().graphics_groups_open);
                let rows = self.graphics.build(content - 8.0, &mut groups);
                self.state.borrow_mut().graphics_groups_open = groups;
                for row in rows {
                    section.add_child(row);
                }
            }
            if *id == "Debug views" {
                let rows: [(&str, &DataSource<bool>, fn(bool)); 4] = [
                    (tr!("stats.debug.freeze_culling"), &self.switches.freeze_culling, ViewFrustum::set_frozen),
                    (tr!("stats.debug.ambient_occlusion"), &self.switches.ambient_occlusion, debug_views::set_ambient_occlusion),
                    (tr!("stats.debug.sun_shadow_maps"), &self.switches.sun_shadow_maps, debug_views::set_sun_shadows),
                    (tr!("stats.debug.lamp_shadows"), &self.switches.lamp_shadows, debug_views::set_lamp_shadows),
                ];
                for (label, on, set) in rows {
                    section.add_child(Box::new(ToggleRow::new(
                        label,
                        on.clone(),
                        content - 8.0,
                        Box::new(move |on, _| set(on)),
                    )));
                }
            }
            for key in rows.iter() {
                let source = DataSource::new(String::from("-"));
                section.add_child(Box::new(StatRow::new(row_label(*key), source.clone(), content - 8.0)));
                self.values.push((*key, source));
            }
            self.body.add_child(Box::new(section));
        }
        self.body.set_z_index(2.0);
        // Show values right away instead of "-" until the next refresh.
        self.last_refresh = None;
    }

    fn value_text(key: Key, stats: &Stats, layout: &super::layout::Layout, mode: Mode) -> String {
        let (avg, min, max) = stats.frame_avg_min_max();
        let cpu_total = stats.update_ms.0 + stats.render_ms.0 + stats.ui_ms.0;
        let c = &stats.counts;
        let m = &stats.memory;
        match key {
            Key::Fps => format!("{:.0}", stats.fps()),
            Key::FrameTime => ms(avg),
            Key::FrameMinMax => format!("{min:.1} / {max:.1} ms"),
            Key::Update => ms(stats.update_ms.0),
            Key::Render => ms(stats.render_ms.0),
            Key::Gpu => {
                if stats.gpu_ms.0 > 0.0 {
                    ms(stats.gpu_ms.0)
                } else {
                    tr!("common.not_available").into()
                }
            }
            Key::Ui => ms(stats.ui_ms.0),
            Key::CpuTotal => ms(cpu_total),
            Key::Rss => bytes(m.rss),
            Key::Virtual => bytes(m.virtual_size),
            Key::System => match (m.system_total, m.system_available) {
                (Some(total), Some(available)) => format!(
                    "{} / {}",
                    bytes(Some(total.saturating_sub(available))),
                    bytes(Some(total))
                ),
                _ => tr!("common.not_available").into(),
            },
            Key::Vram => match m.gpu {
                Some((0, free)) => tr!("stats.vram_free", amount = bytes(Some(free))),
                Some((used, total)) => format!("{} / {}", bytes(Some(used)), bytes(Some(total))),
                None => tr!("common.not_available").into(),
            },
            Key::Entities => c.entities.to_string(),
            Key::Components => c.components.to_string(),
            Key::Chunks => c.transient.to_string(),
            Key::Bodies => c.rigid_bodies.to_string(),
            Key::Colliders => c.colliders.to_string(),
            Key::GpuName => shorten(&stats.gl.renderer, 26),
            Key::GlVersion => shorten(&stats.gl.version, 26),
            Key::Window => format!("{:.0} x {:.0}", layout.window_w, layout.window_h),
            Key::Viewport => format!("{:.0} x {:.0}", layout.viewport.w, layout.viewport.h),
            Key::Culled(index) => {
                let count = ViewFrustum::counts()[index];
                format!("{} / {}", count.drawn, count.total)
            }
            Key::Pass(index) => match stats.pass_ms {
                Some(passes) => ms(passes[index]),
                None => tr!("common.not_available").into(),
            },
            Key::Mode => match mode {
                Mode::Edit => tr!("stats.mode.edit").into(),
                Mode::Play => tr!("stats.mode.play").into(),
            },
        }
    }
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max - 1).collect();
        format!("{cut}\u{2026}")
    }
}

/// VSync is a graphics setting (`display.vsync`): the switch sets it.
fn set_vsync(on: bool) {
    use ferrite::core::graphics;
    if graphics::engine().vsync != on {
        if let Err(e) = graphics::set("display.vsync", graphics::SettingValue::Toggle(on)) {
            log::warn!("{e}");
        }
    }
}

impl UIElement for StatsPanel {
    fn update(&mut self, scene: &mut Scene) {
        // The switch shows the setting, which the graphics panel changes too.
        let vsync = ferrite::core::graphics::engine().vsync;
        if self.switches.vsync.read() != vsync {
            self.switches.vsync.write(vsync);
        }
        let rect = self.state.borrow().layout.stats;
        if rect.w <= 0.0 {
            return;
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));
        let header = self.panel.header_height();
        self.body
            .set_position(Position::new(rect.x + 4.0, rect.y + header + 4.0, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.h - header - 8.0).max(10.0),
        ));
        if self.built_width != Some(rect.w as i32) || self.graphics.outdated() {
            self.build(rect.w);
            self.built_width = Some(rect.w as i32);
        }

        let due = self
            .last_refresh
            .map_or(true, |t| t.elapsed().as_secs_f32() >= TEXT_REFRESH_SECONDS);
        if due {
            self.last_refresh = Some(Instant::now());
            let state = self.state.borrow();
            for (key, source) in &self.values {
                let text = Self::value_text(*key, &state.stats, &state.layout, state.mode);
                if source.read() != text {
                    source.write(text);
                }
            }
        }

        self.panel.update(scene);
        self.body.update(scene);
        self.graphics.sync();
    }

    fn render(&self) {
        if self.state.borrow().layout.stats.w <= 0.0 {
            return;
        }
        self.panel.render();
        self.body.render();
    }

    /// Open dropdown lists (the graphics settings') are drawn here, over
    /// everything else.
    fn render_overlay(&self) {
        if self.state.borrow().layout.stats.w <= 0.0 {
            return;
        }
        self.body.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let rect = self.state.borrow().layout.stats;
        if rect.w <= 0.0 {
            return false;
        }
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
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
        self.body.set_z_index(z + 2.0);
    }
}
