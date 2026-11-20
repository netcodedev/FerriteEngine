//! Line shapes the viewport draws: gizmo handles and markers.

use cgmath::{InnerSpace, Matrix4, Point3, Vector3};
use ferrite::core::renderer::line::{Line, LineRenderer};

use crate::editor::layout::Rect;

use super::*;

/// A diamond (an octahedron's edges) around `center`.
pub(super) fn diamond_lines(center: Point3<f32>, size: f32) -> Vec<Line> {
    let tips: Vec<Point3<f32>> = AXES
        .iter()
        .flat_map(|a| [center + a * size, center - a * size])
        .collect();
    let mut lines = Vec::new();
    for i in 0..6 {
        for j in i + 1..6 {
            // Every pair of tips but the opposite ones.
            if j == i + 1 && i % 2 == 0 {
                continue;
            }
            let d = tips[j] - tips[i];
            lines.push(Line::new(tips[i], d.normalize(), d.magnitude()));
        }
    }
    lines
}

/// The 12 edges of a box around `center` with half sizes along `axes`
/// (each scaled to its half size).
pub(super) fn box_lines(center: Point3<f32>, axes: [Vector3<f32>; 3]) -> Vec<Line> {
    let mut lines = Vec::new();
    for i in 0..3 {
        let (a, b) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
        let along = axes[i] * 2.0;
        if along.magnitude2() < 1e-12 {
            continue;
        }
        for (sa, sb) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let start = center + a * sa + b * sb - axes[i];
            lines.push(Line::new(start, along.normalize(), along.magnitude()));
        }
    }
    lines
}

/// Three crossed lines and four diagonals around `center`.
pub(super) fn star_lines(center: Point3<f32>, size: f32) -> Vec<Line> {
    let diagonal = 1.0 / 3f32.sqrt();
    let mut dirs: Vec<Vector3<f32>> = AXES.to_vec();
    for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        dirs.push(Vector3::new(x * diagonal, diagonal, z * diagonal));
    }
    dirs.into_iter()
        .map(|d| Line::new(center - d * size, d, size * 2.0))
        .collect()
}

/// The outline of a spot light's cone: its rim at `range` and four lines
/// from the light to it.
pub(super) fn cone_lines(apex: Point3<f32>, dir: Vector3<f32>, range: f32, angle_deg: f32) -> Vec<Line> {
    let angle = angle_deg.to_radians();
    let (height, radius) = (range * angle.cos(), range * angle.sin());
    let rim = apex + dir * height;
    let (u, v) = perpendicular_basis(dir);
    let mut lines = circle_lines(rim, dir, radius);
    for side in [u, -u, v, -v] {
        let edge = rim + side * radius - apex;
        lines.push(Line::new(apex, edge.normalize(), edge.magnitude()));
    }
    lines
}

/// Draw `lines` `width` pixels thick by repeating them shifted by whole
/// pixels in screen space. Core-profile GL only guarantees 1 px wide lines,
/// so `glLineWidth` is not reliable.
pub(super) fn render_thick_lines(
    vp: &Matrix4<f32>,
    lines: &Vec<Line>,
    color: Vector3<f32>,
    rect: Rect,
    width: i32,
) {
    let half = width / 2;
    for dy in -half..=half {
        for dx in -half..=half {
            // A translation applied after the projection adds `t * w` to
            // clip space, i.e. shifts NDC by exactly `t`.
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / rect.w.max(1.0),
                dy as f32 * 2.0 / rect.h.max(1.0),
                0.0,
            ));
            LineRenderer::render_lines(&(shift * vp), lines, color, true);
        }
    }
}

/// Markers drawn 1 px wide, gathered while markers in a row share a colour
/// and drawn in one go: one upload and draw instead of one per marker.
/// Markers of another colour or drawn thicker `flush` it first, so
/// everything is drawn in the same order as marker by marker.
pub(super) struct LineBatch {
    color: Vector3<f32>,
    lines: Vec<Line>,
}

impl LineBatch {
    pub(super) fn new() -> Self {
        Self {
            color: Vector3::new(0.0, 0.0, 0.0),
            lines: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, vp: &Matrix4<f32>, rect: Rect, lines: Vec<Line>, color: Vector3<f32>) {
        if color != self.color {
            self.flush(vp, rect);
            self.color = color;
        }
        self.lines.extend(lines);
    }

    pub(super) fn flush(&mut self, vp: &Matrix4<f32>, rect: Rect) {
        if !self.lines.is_empty() {
            render_thick_lines(vp, &self.lines, self.color, rect, 1);
            self.lines.clear();
        }
    }
}

pub(super) fn arrow_lines(center: Point3<f32>, axis: Vector3<f32>, size: f32) -> Vec<Line> {
    let tip = center + axis * size;
    let (u, v) = perpendicular_basis(axis);
    let head = size * 0.15;
    let mut lines = vec![Line::new(center, axis, size)];
    for side in [u, -u, v, -v] {
        let start = tip - axis * head + side * head * 0.4;
        let dir = tip - start;
        lines.push(Line::new(start, dir.normalize(), dir.magnitude()));
    }
    lines
}

/// The 12 edges of a cube around `center`, aligned with `axes`.
pub(super) fn cube_lines(center: Point3<f32>, axes: [Vector3<f32>; 3], half: f32) -> Vec<Line> {
    let mut lines = Vec::with_capacity(12);
    for i in 0..3 {
        let (a, b) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
        for (sa, sb) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let start = center + (a * sa + b * sb - axes[i]) * half;
            lines.push(Line::new(start, axes[i], half * 2.0));
        }
    }
    lines
}

pub(super) fn circle_lines(center: Point3<f32>, axis: Vector3<f32>, radius: f32) -> Vec<Line> {
    let (u, v) = perpendicular_basis(axis);
    let point = |i: usize| {
        let a = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
        center + (u * a.cos() + v * a.sin()) * radius
    };
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let (a, b) = (point(i), point(i + 1));
            Line::new(a, (b - a).normalize(), (b - a).magnitude())
        })
        .collect()
}

#[cfg(test)]
mod light_marker_tests {
    use super::*;

    #[test]
    fn a_spot_lights_cone_reaches_its_range_at_its_angle() {
        let apex = Point3::new(1.0, 5.0, 2.0);
        let dir = -Vector3::unit_y();
        let lines = cone_lines(apex, dir, 10.0, 30.0);
        let sides = &lines[lines.len() - 4..];
        for side in sides {
            assert_eq!(side.position, apex);
            assert!((side.length - 10.0).abs() < 1e-4);
            let cos = side.direction.dot(dir);
            assert!((cos - 30f32.to_radians().cos()).abs() < 1e-4);
        }
    }

    #[test]
    fn an_emitter_marker_is_a_diamond_and_a_box_has_twelve_edges() {
        let lines = diamond_lines(Point3::new(0.0, 0.0, 0.0), 1.0);
        assert_eq!(lines.len(), 12);
        for line in &lines {
            assert!((line.length - 2f32.sqrt()).abs() < 1e-5);
        }
        let edges = box_lines(
            Point3::new(0.0, 0.0, 0.0),
            [Vector3::unit_x(), Vector3::unit_y() * 2.0, Vector3::unit_z()],
        );
        assert_eq!(edges.len(), 12);
        assert!(edges.iter().any(|l| (l.length - 4.0).abs() < 1e-5));
    }

    #[test]
    fn a_light_marker_is_a_star_around_the_light() {
        let center = Point3::new(0.0, 2.0, 0.0);
        let lines = star_lines(center, 0.5);
        assert_eq!(lines.len(), 7);
        for line in &lines {
            let middle = line.position + line.direction * (line.length / 2.0);
            assert!((middle - center).magnitude() < 1e-5);
            assert!((line.length - 1.0).abs() < 1e-5);
        }
    }
}
