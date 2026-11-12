/// Docked editor layout in window pixels (top-left origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub window_w: f32,
    pub window_h: f32,
    pub titlebar: Rect,
    pub toolbar: Rect,
    pub hierarchy: Rect,
    pub inspector: Rect,
    pub viewport: Rect,
    /// The whole right column; zero width when nothing is docked there.
    pub right: Rect,
    /// Statistics (scene) or preview (animation) in the right column; zero
    /// size when hidden.
    pub stats: Rect,
    /// Widget showcase in the right column; zero size when hidden.
    pub widgets: Rect,
}

pub const TITLEBAR_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 32.0;
pub const TOP: f32 = TITLEBAR_HEIGHT + TOOLBAR_HEIGHT;
/// Smallest the viewport may get when columns are resized or the window
/// shrinks.
const MIN_VIEWPORT: f32 = 200.0;
/// Smallest panel width / height reachable by dragging.
pub const MIN_COLUMN: f32 = 220.0;
pub const MIN_PANEL_HEIGHT: f32 = 120.0;

/// User-adjustable panel sizes (dragged with the splitters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelSizes {
    /// Left column holding hierarchy (top) and inspector (bottom).
    pub side: f32,
    /// Share of the left column's height given to its top panel.
    pub top_share: f32,
    pub stats: f32,
    pub preview: f32,
    /// Share of the right column's height for statistics when the widget
    /// showcase is docked below it.
    pub right_share: f32,
}

impl Default for PanelSizes {
    fn default() -> Self {
        Self {
            side: 330.0,
            top_share: 0.4,
            stats: 300.0,
            preview: 360.0,
            right_share: 0.55,
        }
    }
}

/// What occupies the right-hand column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightColumn {
    /// Scene workspace: statistics and/or the widget showcase.
    Panels { stats: bool, widgets: bool },
    Preview,
}

impl Layout {
    pub fn compute(window_w: f32, window_h: f32, right: RightColumn, sizes: &PanelSizes) -> Self {
        let body_h = (window_h - TOP).max(64.0);
        // Columns shrink before the viewport disappears on small windows.
        let side_w = sizes.side.min((window_w - MIN_VIEWPORT).max(0.0)).floor();
        let wanted = match right {
            RightColumn::Panels { stats: false, widgets: false } => 0.0,
            RightColumn::Panels { .. } => sizes.stats,
            RightColumn::Preview => sizes.preview,
        };
        let stats_w = wanted.min((window_w - side_w - MIN_VIEWPORT).max(0.0)).floor();
        // Right column: top panel (stats or preview) and/or widgets below.
        let (show_top, show_widgets) = match right {
            RightColumn::Preview => (true, false),
            RightColumn::Panels { stats, widgets } => (stats, widgets),
        };
        let top_h = match (show_top, show_widgets) {
            (true, true) => (body_h * sizes.right_share)
                .clamp(MIN_PANEL_HEIGHT.min(body_h * 0.5), (body_h - MIN_PANEL_HEIGHT).max(body_h * 0.5))
                .floor(),
            (true, false) => body_h,
            _ => 0.0,
        };
        let hierarchy_h = (body_h * sizes.top_share)
            .clamp(MIN_PANEL_HEIGHT.min(body_h * 0.5), (body_h - MIN_PANEL_HEIGHT).max(body_h * 0.5))
            .floor();
        Self {
            window_w,
            window_h,
            titlebar: Rect {
                x: 0.0,
                y: 0.0,
                w: window_w,
                h: TITLEBAR_HEIGHT,
            },
            toolbar: Rect {
                x: 0.0,
                y: TITLEBAR_HEIGHT,
                w: window_w,
                h: TOOLBAR_HEIGHT,
            },
            hierarchy: Rect {
                x: 0.0,
                y: TOP,
                w: side_w,
                h: hierarchy_h,
            },
            inspector: Rect {
                x: 0.0,
                y: TOP + hierarchy_h,
                w: side_w,
                h: body_h - hierarchy_h,
            },
            viewport: Rect {
                x: side_w,
                y: TOP,
                w: (window_w - side_w - stats_w).max(1.0),
                h: body_h,
            },
            right: Rect {
                x: window_w - stats_w,
                y: TOP,
                w: stats_w,
                h: body_h,
            },
            stats: Rect {
                x: window_w - stats_w,
                y: TOP,
                w: if show_top { stats_w } else { 0.0 },
                h: top_h,
            },
            widgets: Rect {
                x: window_w - stats_w,
                y: TOP + top_h,
                w: if show_widgets { stats_w } else { 0.0 },
                h: body_h - top_h,
            },
        }
    }

    /// The viewport as a GL viewport (bottom-left origin).
    pub fn viewport_gl(&self) -> [i32; 4] {
        let v = self.viewport;
        [
            v.x as i32,
            (self.window_h - (v.y + v.h)).max(0.0) as i32,
            v.w as i32,
            v.h as i32,
        ]
    }
}
