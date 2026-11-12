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
    /// Right column (statistics or animation preview); zero width when
    /// hidden.
    pub stats: Rect,
}

pub const TITLEBAR_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 32.0;
const TOP: f32 = TITLEBAR_HEIGHT + TOOLBAR_HEIGHT;
/// Left column holding the hierarchy (top) and inspector (bottom). Wide
/// enough for the inspector's label column plus three number fields.
const SIDE_WIDTH: f32 = 330.0;
/// Share of the column height given to the hierarchy.
const HIERARCHY_SHARE: f32 = 0.4;
const MIN_HIERARCHY: f32 = 140.0;
const MIN_VIEWPORT: f32 = 64.0;
const STATS_WIDTH: f32 = 300.0;
const PREVIEW_WIDTH: f32 = 360.0;

/// What occupies the right-hand column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightColumn {
    None,
    Stats,
    Preview,
}

impl Layout {
    pub fn compute(window_w: f32, window_h: f32, right: RightColumn) -> Self {
        let body_h = (window_h - TOP).max(MIN_VIEWPORT);
        // Columns shrink before the viewport disappears on tiny windows.
        let side_w = SIDE_WIDTH.min((window_w - MIN_VIEWPORT).max(0.0)).floor();
        let wanted = match right {
            RightColumn::None => 0.0,
            RightColumn::Stats => STATS_WIDTH,
            RightColumn::Preview => PREVIEW_WIDTH,
        };
        let stats_w = wanted.min((window_w - side_w - MIN_VIEWPORT).max(0.0)).floor();
        let hierarchy_h = (body_h * HIERARCHY_SHARE)
            .max(MIN_HIERARCHY)
            .min(body_h * 0.8)
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
            stats: Rect {
                x: window_w - stats_w,
                y: TOP,
                w: stats_w,
                h: body_h,
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
