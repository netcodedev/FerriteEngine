//! GPU time per render pass, from `GL_TIMESTAMP` queries: a timestamp is
//! written where each pass starts and one at the end of the frame, and
//! the time between two is the pass's. Results are read a few frames
//! later, when the GPU has them, so the pipeline never waits for them.
//! Off until something (the editor's statistics) turns it on.

use std::cell::RefCell;

/// The passes `Scene::render` is split into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    WeatherMaps,
    SunShadows,
    LightShadows,
    DepthPrepass,
    Sky,
    Opaque,
    Transparent,
    Post,
}

pub const PASSES: usize = 8;

impl Pass {
    pub const ALL: [Pass; PASSES] = [
        Pass::WeatherMaps,
        Pass::SunShadows,
        Pass::LightShadows,
        Pass::DepthPrepass,
        Pass::Sky,
        Pass::Opaque,
        Pass::Transparent,
        Pass::Post,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pass::WeatherMaps => "Weather maps",
            Pass::SunShadows => "Sun shadows",
            Pass::LightShadows => "Light shadows",
            Pass::DepthPrepass => "Depth prepass + AO",
            Pass::Sky => "Sky + clouds",
            Pass::Opaque => "Opaque",
            Pass::Transparent => "Transparent + weather",
            Pass::Post => "HDR + post",
        }
    }
}

/// Frames in flight before a frame's queries are reused.
const FRAMES: usize = 4;
/// Timestamps per frame: one per pass and the end.
const MARKS: usize = PASSES + 1;

struct Frame {
    queries: [u32; MARKS],
    /// Which pass each written timestamp starts (`None`: the frame's end).
    passes: Vec<Option<Pass>>,
    pending: bool,
}

struct Profiler {
    frames: Vec<Frame>,
    current: Option<usize>,
    next: usize,
    smoothed: [f32; PASSES],
}

thread_local! {
    static PROFILER: RefCell<Option<Profiler>> = const { RefCell::new(None) };
}

/// Turn the per-pass timing on or off.
pub fn set_enabled(enabled: bool) {
    PROFILER.with(|profiler| {
        let mut profiler = profiler.borrow_mut();
        match (enabled, profiler.is_some()) {
            (true, false) => *profiler = Some(Profiler::new()),
            (false, true) => {
                if let Some(old) = profiler.take() {
                    old.delete();
                }
            }
            _ => {}
        }
    });
}

pub fn is_enabled() -> bool {
    PROFILER.with(|profiler| profiler.borrow().is_some())
}

/// Start timing a frame's passes; ends with `end_frame`.
pub fn begin_frame() {
    with(|profiler| profiler.begin_frame());
}

/// The commands from here to the next mark (or the end) belong to `pass`.
pub fn mark(pass: Pass) {
    crate::core::diagnostics::mark_pass(pass.label());
    with(|profiler| profiler.mark(Some(pass)));
}

pub fn end_frame() {
    with(|profiler| {
        profiler.mark(None);
        profiler.current = None;
    });
}

/// Smoothed milliseconds of each pass, in `Pass::ALL`'s order; 0 for a
/// pass that did not run lately. `None` while timing is off.
pub fn pass_ms() -> Option<[f32; PASSES]> {
    PROFILER.with(|profiler| profiler.borrow().as_ref().map(|p| p.smoothed))
}

fn with(f: impl FnOnce(&mut Profiler)) {
    PROFILER.with(|profiler| {
        if let Some(profiler) = profiler.borrow_mut().as_mut() {
            f(profiler);
        }
    });
}

impl Profiler {
    fn new() -> Self {
        let frames = (0..FRAMES)
            .map(|_| {
                let mut queries = [0u32; MARKS];
                unsafe { gl::GenQueries(MARKS as i32, queries.as_mut_ptr()) };
                Frame {
                    queries,
                    passes: Vec::with_capacity(MARKS),
                    pending: false,
                }
            })
            .collect();
        Self {
            frames,
            current: None,
            next: 0,
            smoothed: [0.0; PASSES],
        }
    }

    fn delete(self) {
        for frame in &self.frames {
            unsafe { gl::DeleteQueries(MARKS as i32, frame.queries.as_ptr()) };
        }
    }

    fn collect(&mut self) {
        for index in 0..self.frames.len() {
            let frame = &mut self.frames[index];
            if !frame.pending || frame.passes.is_empty() {
                continue;
            }
            let last = frame.queries[frame.passes.len() - 1];
            let mut available = 0;
            unsafe { gl::GetQueryObjectiv(last, gl::QUERY_RESULT_AVAILABLE, &mut available) };
            if available == 0 {
                continue;
            }
            let mut stamps = [0u64; MARKS];
            for (i, stamp) in stamps.iter_mut().enumerate().take(frame.passes.len()) {
                unsafe { gl::GetQueryObjectui64v(frame.queries[i], gl::QUERY_RESULT, stamp) };
            }
            let mut totals = [0.0f32; PASSES];
            for i in 0..frame.passes.len() - 1 {
                if let Some(pass) = frame.passes[i] {
                    let ns = stamps[i + 1].saturating_sub(stamps[i]);
                    totals[pass as usize] += ns as f32 / 1_000_000.0;
                }
            }
            for (smoothed, total) in self.smoothed.iter_mut().zip(totals) {
                *smoothed = if *smoothed == 0.0 {
                    total
                } else {
                    *smoothed * 0.9 + total * 0.1
                };
            }
            frame.pending = false;
        }
    }

    fn begin_frame(&mut self) {
        self.collect();
        if self.frames[self.next].pending {
            // Still in flight: skip timing this frame rather than wait.
            self.current = None;
            return;
        }
        let frame = &mut self.frames[self.next];
        frame.passes.clear();
        self.current = Some(self.next);
        self.next = (self.next + 1) % FRAMES;
    }

    fn mark(&mut self, pass: Option<Pass>) {
        let Some(current) = self.current else {
            return;
        };
        let frame = &mut self.frames[current];
        // The end always gets the last one.
        if frame.passes.len() >= MARKS - pass.is_some() as usize {
            return;
        }
        // A pass marked twice in a row is one span.
        if pass.is_some() && frame.passes.last() == Some(&pass) {
            return;
        }
        unsafe { gl::QueryCounter(frame.queries[frame.passes.len()], gl::TIMESTAMP) };
        frame.passes.push(pass);
        if pass.is_none() {
            frame.pending = true;
        }
    }
}
