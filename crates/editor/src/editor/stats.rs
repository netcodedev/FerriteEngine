//! Render statistics: collection (frame timings, GPU time, memory, scene
//! counts) and the docked "Statistics" panel that shows them in sections.

use std::{collections::VecDeque, ffi::CStr, time::Instant};

use ferrite::core::{
    entity::Entity,
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{panel::Panel, scroll::ScrollArea, section::Section},
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

/// GPU time of a block of GL commands via `GL_TIME_ELAPSED` queries. A
/// small ring of queries is used so results are read frames later, without
/// stalling the pipeline.
pub struct GpuTimer {
    queries: [u32; 4],
    issued: [bool; 4],
    next: usize,
    active: bool,
    last_ms: Option<f32>,
}

impl GpuTimer {
    pub fn new() -> Self {
        let mut queries = [0u32; 4];
        unsafe {
            gl::GenQueries(queries.len() as i32, queries.as_mut_ptr());
        }
        Self {
            queries,
            issued: [false; 4],
            next: 0,
            active: false,
            last_ms: None,
        }
    }

    pub fn begin(&mut self) {
        // Collect any finished results before reusing a query object.
        for i in 0..self.queries.len() {
            if self.issued[i] {
                let mut available = 0;
                unsafe {
                    gl::GetQueryObjectiv(self.queries[i], gl::QUERY_RESULT_AVAILABLE, &mut available);
                }
                if available != 0 {
                    let mut ns: u64 = 0;
                    unsafe {
                        gl::GetQueryObjectui64v(self.queries[i], gl::QUERY_RESULT, &mut ns);
                    }
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
        unsafe {
            gl::BeginQuery(gl::TIME_ELAPSED, self.queries[self.next]);
        }
        self.active = true;
    }

    pub fn end(&mut self) {
        if !self.active {
            return;
        }
        unsafe {
            gl::EndQuery(gl::TIME_ELAPSED);
        }
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

fn has_extension(name: &str) -> bool {
    let mut count = 0;
    unsafe {
        gl::GetIntegerv(gl::NUM_EXTENSIONS, &mut count);
    }
    (0..count.max(0) as u32).any(|i| unsafe {
        let ptr = gl::GetStringi(gl::EXTENSIONS, i);
        !ptr.is_null() && CStr::from_ptr(ptr as *const _).to_bytes() == name.as_bytes()
    })
}

/// VRAM `(used, total)` from NVIDIA's `GL_NVX_gpu_memory_info`, or an
/// approximation from `GL_ATI_meminfo` (AMD/Mesa), which only reports free
/// memory.
fn read_gpu_memory() -> Option<(u64, u64)> {
    const NVX_TOTAL: u32 = 0x9048;
    const NVX_AVAILABLE: u32 = 0x9049;
    const ATI_TEXTURE_FREE: u32 = 0x87FC;
    if has_extension("GL_NVX_gpu_memory_info") {
        let (mut total, mut available) = (0, 0);
        unsafe {
            gl::GetIntegerv(NVX_TOTAL, &mut total);
            gl::GetIntegerv(NVX_AVAILABLE, &mut available);
        }
        let total = total as u64 * 1024;
        let available = available as u64 * 1024;
        return Some((total.saturating_sub(available), total));
    }
    if has_extension("GL_ATI_meminfo") {
        let mut info = [0i32; 4];
        unsafe {
            gl::GetIntegerv(ATI_TEXTURE_FREE, info.as_mut_ptr());
        }
        // [free in pool, largest free block, aux free, largest aux block]
        // No total is reported; show free memory as "used = 0 of free".
        return Some((0, info[0] as u64 * 1024));
    }
    None
}

fn gl_string(name: u32) -> String {
    unsafe {
        let ptr = gl::GetString(name);
        if ptr.is_null() {
            return "unknown".into();
        }
        CStr::from_ptr(ptr as *const _).to_string_lossy().into_owned()
    }
}

fn read_gl_info() -> GlInfo {
    GlInfo {
        renderer: gl_string(gl::RENDERER),
        version: gl_string(gl::VERSION)
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn bytes(value: Option<u64>) -> String {
    match value {
        None => "n/a".into(),
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
        self.background.render();
        for bar in &self.bars {
            bar.render();
        }
        self.target_line.render();
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
}

const SECTIONS: &[(&str, &str, &[(Key, &str)])] = &[
    (
        "Performance",
        icons::GAUGE,
        &[
            (Key::Fps, "Frames per second"),
            (Key::FrameTime, "Frame time (avg)"),
            (Key::FrameMinMax, "Min / max"),
        ],
    ),
    (
        "Timings",
        icons::TIMER,
        &[
            (Key::Update, "Scene update"),
            (Key::Render, "Scene render (CPU)"),
            (Key::Gpu, "Scene render (GPU)"),
            (Key::Ui, "UI update + render"),
            (Key::CpuTotal, "Frame CPU total"),
        ],
    ),
    (
        "Memory",
        icons::MEMORY_STICK,
        &[
            (Key::Rss, "Process (RSS)"),
            (Key::Virtual, "Process (virtual)"),
            (Key::System, "System in use"),
            (Key::Vram, "GPU memory"),
        ],
    ),
    (
        "Scene",
        icons::LAYERS,
        &[
            (Key::Entities, "Entities"),
            (Key::Components, "Components"),
            (Key::Chunks, "Terrain chunks"),
            (Key::Bodies, "Rigid bodies"),
            (Key::Colliders, "Colliders"),
        ],
    ),
    (
        "Renderer",
        icons::MONITOR,
        &[
            (Key::GpuName, "GPU"),
            (Key::GlVersion, "OpenGL"),
            (Key::Window, "Window"),
            (Key::Viewport, "Viewport"),
            (Key::Mode, "Mode"),
        ],
    ),
];

/// Docked right-hand panel with collapsible statistic sections.
pub struct StatsPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    values: Vec<(Key, DataSource<String>)>,
    /// Open/closed state per section, kept across rebuilds.
    open: Vec<DataSource<bool>>,
    built_width: Option<i32>,
    last_refresh: Option<Instant>,
}

impl StatsPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Statistics".into(), Position::default(), Size::new(300.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(290.0, 300.0));
        body.set_gap(4.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            values: Vec::new(),
            open: SECTIONS.iter().map(|_| DataSource::new(true)).collect(),
            built_width: None,
            last_refresh: None,
        }
    }

    /// (Re)create the sections for the current column width.
    fn build(&mut self, width: f32) {
        self.body.clear();
        self.values.clear();
        let content = (width - 24.0).max(160.0);
        for (index, (title, icon, rows)) in SECTIONS.iter().enumerate() {
            let mut section = Section::new(title, Some(icon), content);
            section.bind_open(self.open[index].clone());
            if index == 0 {
                section.add_child(Box::new(FrameGraph::new(self.state.clone(), content - 8.0)));
            }
            for (key, label) in rows.iter() {
                let source = DataSource::new(String::from("-"));
                section.add_child(Box::new(StatRow::new(label, source.clone(), content - 8.0)));
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
                    "n/a".into()
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
                _ => "n/a".into(),
            },
            Key::Vram => match m.gpu {
                Some((0, free)) => format!("{} free", bytes(Some(free))),
                Some((used, total)) => format!("{} / {}", bytes(Some(used)), bytes(Some(total))),
                None => "n/a".into(),
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
            Key::Mode => match mode {
                Mode::Edit => "Edit".into(),
                Mode::Play => "Play".into(),
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

impl UIElement for StatsPanel {
    fn update(&mut self, scene: &mut Scene) {
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
        if self.built_width != Some(rect.w as i32) {
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
    }

    fn render(&self) {
        if self.state.borrow().layout.stats.w <= 0.0 {
            return;
        }
        self.panel.render();
        self.body.render();
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
