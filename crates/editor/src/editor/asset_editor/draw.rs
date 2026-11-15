use cgmath::{InnerSpace, Matrix4, Point3, Vector3};
use ferrite::core::renderer::line::{Line, LineRenderer};

use crate::editor::layout::Rect;

use super::*;

/// Window position of a world point inside `view`; `None` behind the eye.
pub(super) fn project(vp: &Matrix4<f32>, view: &Rect, p: Point3<f32>) -> Option<(f32, f32)> {
    let clip = vp * p.to_homogeneous();
    if clip.w <= 1e-6 {
        return None;
    }
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    Some((
        view.x + (x * 0.5 + 0.5) * view.w,
        view.y + (0.5 - y * 0.5) * view.h,
    ))
}

pub(super) fn ray_plane(
    origin: Point3<f32>,
    dir: Vector3<f32>,
    point: Point3<f32>,
    normal: Vector3<f32>,
) -> Option<Point3<f32>> {
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - origin).dot(normal) / denom;
    (t > 0.0).then(|| origin + dir * t)
}

/// Lines drawn on top of everything, `width` pixels wide.
pub(super) fn render_thick_lines(vp: &Matrix4<f32>, lines: &Vec<Line>, color: (f32, f32, f32), view: &Rect, width: i32) {
    let color = Vector3::new(color.0, color.1, color.2);
    let half = width / 2;
    for dy in -half..=half {
        for dx in -half..=half {
            // A translation after the projection shifts NDC by exactly it.
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / view.w.max(1.0),
                dy as f32 * 2.0 / view.h.max(1.0),
                0.0,
            ));
            LineRenderer::render_lines(&(shift * vp), lines, color, true);
        }
    }
}

pub(super) fn segment(from: Point3<f32>, to: Point3<f32>) -> Option<Line> {
    let d = to - from;
    let length = d.magnitude();
    (length > 1e-6).then(|| Line::new(from, d / length, length))
}

/// A wireframe diamond around `center`, `size` from its middle to a tip.
pub(super) fn diamond(center: Point3<f32>, size: f32) -> Vec<Line> {
    let tips = [
        Vector3::unit_x(),
        -Vector3::unit_x(),
        Vector3::unit_z(),
        -Vector3::unit_z(),
    ];
    let mut lines = Vec::new();
    for (i, a) in tips.iter().enumerate() {
        let b = tips[[2, 3, 1, 0][i]];
        lines.extend(segment(center + a * size, center + b * size));
        lines.extend(segment(center + a * size, center + Vector3::unit_y() * size));
        lines.extend(segment(center + a * size, center - Vector3::unit_y() * size));
    }
    lines
}

/// Draw the bones of a rigged model and the handles of its chains.
pub(super) fn render_skeleton(rigged: &Rigged, vp: &Matrix4<f32>, view: &Rect, hot: Option<usize>) {
    let positions = rigged.skeleton.world_positions(&rigged.root());
    let bones: Vec<Line> = rigged
        .skeleton
        .joints
        .iter()
        .enumerate()
        .filter_map(|(i, joint)| segment(positions[joint.parent?], positions[i]))
        .collect();
    render_thick_lines(vp, &bones, BONE_COLOR, view, 2);
    let size = rigged.radius * 0.035;
    for (i, chain) in rigged.chains.iter().enumerate() {
        let color = if Some(i) == hot { HANDLE_HOT_COLOR } else { HANDLE_COLOR };
        render_thick_lines(vp, &diamond(positions[chain.end], size), color, view, 2);
    }
}

/// Pixels from `p` to the segment `a`–`b`.
pub(super) fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 1e-9 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}
