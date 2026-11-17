//! Cables: runs of spans from one device's port to another's, fastened to
//! walls, floors or the ground at every point between. Each span hangs
//! down by its own sag (a parabola, close to how a real cable hangs).

use ferrite::prelude::*;

/// How long a cable can be, metres (one reel), unless the scene says
/// otherwise.
pub const DEFAULT_MAX_LENGTH: f32 = 50.0;
/// How far a span sags at most, as a share of its horizontal length.
pub const MAX_SAG_SHARE: f32 = 0.35;
/// The sag share a new cable starts with.
pub const DEFAULT_SAG_SHARE: f32 = 0.03;
/// How much one notch of the mouse wheel changes the sag share.
pub const SAG_STEP: f32 = 0.01;

/// One end of a cable: a device's port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct End {
    pub device: usize,
    pub port: usize,
}

/// A straight piece of cable between two points, hanging down by `sag`
/// metres in its middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub a: Vector3<f32>,
    pub b: Vector3<f32>,
    pub sag: f32,
}

pub fn horizontal(v: Vector3<f32>) -> f32 {
    (v.x * v.x + v.z * v.z).sqrt()
}

impl Span {
    /// A span sagging by `share` of its horizontal length.
    pub fn with_share(a: Vector3<f32>, b: Vector3<f32>, share: f32) -> Span {
        Span { a, b, sag: sag_for(a, b, share) }
    }

    /// The point `t` (0..1) along it.
    pub fn point(&self, t: f32) -> Vector3<f32> {
        self.a + (self.b - self.a) * t - Vector3::unit_y() * (4.0 * self.sag * t * (1.0 - t))
    }

    /// `pieces + 1` points from `a` to `b`.
    pub fn points(&self, pieces: usize) -> Vec<Vector3<f32>> {
        (0..=pieces).map(|i| self.point(i as f32 / pieces as f32)).collect()
    }

    /// How many pieces it is drawn and measured in.
    pub fn pieces(&self) -> usize {
        let straight = (self.b - self.a).magnitude();
        ((straight / 0.4) as usize).clamp(1, 32) + if self.sag > 0.0 { 4 } else { 0 }
    }

    /// Its length along the curve.
    pub fn length(&self) -> f32 {
        let points = self.points(self.pieces().max(16));
        points.windows(2).map(|w| (w[1] - w[0]).magnitude()).sum()
    }

    /// Its lowest point.
    pub fn lowest(&self) -> Vector3<f32> {
        self.points(self.pieces()).into_iter().fold(self.a, |low, p| if p.y < low.y { p } else { low })
    }
}

/// The sag for `share` of the span's horizontal length; none for a span
/// going straight up or down (it lies along a wall).
pub fn sag_for(a: Vector3<f32>, b: Vector3<f32>, share: f32) -> f32 {
    horizontal(b - a) * share.clamp(0.0, MAX_SAG_SHARE)
}

/// How a span is held straight while Shift is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// Straight up or down from the last point.
    Vertical,
    /// Level with the last point.
    Horizontal,
}

impl Lock {
    pub fn name(self) -> &'static str {
        match self {
            Lock::Vertical => "senkrecht",
            Lock::Horizontal => "waagerecht",
        }
    }
}

/// `target` moved so the span from `from` to it is exactly vertical or
/// exactly horizontal, whichever is closer to where it points.
pub fn lock(from: Vector3<f32>, target: Vector3<f32>) -> (Vector3<f32>, Lock) {
    let d = target - from;
    if d.y.abs() > horizontal(d) {
        (Vector3::new(from.x, target.y, from.z), Lock::Vertical)
    } else {
        (Vector3::new(target.x, from.y, target.z), Lock::Horizontal)
    }
}

/// A cable in the world: its ends, the points it is fastened at (the ends
/// included) and each span's sag.
#[derive(Clone, Debug, PartialEq)]
pub struct Cable {
    pub from: End,
    pub to: End,
    pub points: Vec<Vector3<f32>>,
    pub sags: Vec<f32>,
    /// An overhead line between poles: thinner, and not bound to the reel's
    /// length.
    pub overhead: bool,
}

impl Cable {
    pub fn spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.points.windows(2).zip(&self.sags).map(|(w, sag)| Span { a: w[0], b: w[1], sag: *sag })
    }

    pub fn length(&self) -> f32 {
        self.spans().map(|s| s.length()).sum()
    }

    pub fn joins(&self, device: usize) -> bool {
        self.from.device == device || self.to.device == device
    }
}

/// The nearest distance between a ray (`dir` normalized) and the segment
/// `a`–`b`, and how far along the ray that is.
pub fn ray_segment(origin: Vector3<f32>, dir: Vector3<f32>, a: Vector3<f32>, b: Vector3<f32>) -> (f32, f32) {
    let ab = b - a;
    let w = origin - a;
    let (aa, ad, bb) = (ab.dot(ab), ab.dot(dir), dir.dot(dir));
    let (aw, dw) = (ab.dot(w), dir.dot(w));
    let denominator = aa * bb - ad * ad;
    let mut s = if denominator.abs() < 1e-9 { 0.0 } else { ((aw * bb - ad * dw) / denominator).clamp(0.0, 1.0) };
    let mut t = (ad * s - dw) / bb;
    if t < 0.0 {
        t = 0.0;
        s = if aa > 0.0 { (aw / aa).clamp(0.0, 1.0) } else { 0.0 };
    }
    let p = a + ab * s;
    let q = origin + dir * t;
    ((p - q).magnitude(), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vector3<f32> {
        Vector3::new(x, y, z)
    }

    #[test]
    fn a_span_hangs_lowest_in_its_middle() {
        let span = Span { a: v(0.0, 5.0, 0.0), b: v(10.0, 5.0, 0.0), sag: 1.0 };
        assert!((span.point(0.5).y - 4.0).abs() < 1e-5);
        assert_eq!(span.point(0.0), span.a);
        assert_eq!(span.point(1.0), span.b);
        assert!((span.lowest().y - 4.0).abs() < 0.01);
    }

    #[test]
    fn sagging_makes_a_cable_longer() {
        let tight = Span { a: v(0.0, 0.0, 0.0), b: v(10.0, 0.0, 0.0), sag: 0.0 };
        assert!((tight.length() - 10.0).abs() < 1e-3);
        let loose = Span { sag: 1.5, ..tight };
        // A parabola of 10 m with 1.5 m sag is about 10.58 m long.
        assert!((loose.length() - 10.58).abs() < 0.03, "{}", loose.length());
    }

    #[test]
    fn spans_up_a_wall_do_not_sag() {
        assert_eq!(sag_for(v(1.0, 0.0, 1.0), v(1.0, 3.0, 1.0), 0.2), 0.0);
        assert!((sag_for(v(0.0, 0.0, 0.0), v(3.0, 2.0, 4.0), 0.1) - 0.5).abs() < 1e-5);
        // The share is held within its bounds.
        assert!((sag_for(v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0), 2.0) - 10.0 * MAX_SAG_SHARE).abs() < 1e-5);
    }

    #[test]
    fn shift_holds_a_span_level_or_upright() {
        let from = v(0.0, 1.0, 0.0);
        let (p, lock_kind) = lock(from, v(3.0, 1.4, 2.0));
        assert_eq!((p, lock_kind), (v(3.0, 1.0, 2.0), Lock::Horizontal));
        let (p, lock_kind) = lock(from, v(0.3, 3.0, -0.2));
        assert_eq!((p, lock_kind), (v(0.0, 3.0, 0.0), Lock::Vertical));
    }

    #[test]
    fn a_cable_is_as_long_as_its_spans() {
        let cable = Cable {
            from: End { device: 0, port: 0 },
            to: End { device: 1, port: 0 },
            points: vec![v(0.0, 0.0, 0.0), v(0.0, 3.0, 0.0), v(4.0, 3.0, 0.0)],
            sags: vec![0.0, 0.0],
            overhead: false,
        };
        assert!((cable.length() - 7.0).abs() < 1e-3);
        assert!(cable.joins(1) && !cable.joins(2));
    }

    #[test]
    fn rays_find_cables() {
        let (d, t) = ray_segment(v(0.0, 0.0, -5.0), v(0.0, 0.0, 1.0), v(-1.0, 0.1, 0.0), v(1.0, 0.1, 0.0));
        assert!((d - 0.1).abs() < 1e-4 && (t - 5.0).abs() < 1e-4);
        // Past the end of the segment.
        let (d, _) = ray_segment(v(3.0, 0.0, -5.0), v(0.0, 0.0, 1.0), v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0));
        assert!((d - 2.0).abs() < 1e-4);
    }
}
