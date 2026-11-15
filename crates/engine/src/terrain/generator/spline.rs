//! Curves that turn a climate value into metres (or a factor): points
//! joined by a smooth curve that never overshoots them (monotone cubic,
//! Fritsch-Carlson), flat beyond the first and last point. Kept as data,
//! so a world's shape can be tuned without touching code.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spline {
    /// `(input, output)`, by rising input.
    pub points: Vec<(f32, f32)>,
}

impl Spline {
    pub fn new(points: &[(f32, f32)]) -> Self {
        let mut points = points.to_vec();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { points }
    }

    /// The curve at `x`.
    pub fn at(&self, x: f32) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => return 0.0,
            1 => return p[0].1,
            _ => {}
        }
        if x <= p[0].0 {
            return p[0].1;
        }
        let last = p.len() - 1;
        if x >= p[last].0 {
            return p[last].1;
        }
        // The segment holding x.
        let i = p.partition_point(|q| q.0 <= x).saturating_sub(1).min(last - 1);
        let (x0, y0) = p[i];
        let (x1, y1) = p[i + 1];
        let h = x1 - x0;
        if h <= 0.0 {
            return y1;
        }
        let (m0, m1) = (self.tangent(i), self.tangent(i + 1));
        let t = (x - x0) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * y0
            + (t3 - 2.0 * t2 + t) * h * m0
            + (-2.0 * t3 + 3.0 * t2) * y1
            + (t3 - t2) * h * m1
    }

    fn slope(&self, i: usize) -> f32 {
        let (a, b) = (self.points[i], self.points[i + 1]);
        let h = b.0 - a.0;
        if h <= 0.0 {
            0.0
        } else {
            (b.1 - a.1) / h
        }
    }

    /// Tangent at point `i`, limited so the curve stays monotone between
    /// points (no bumps past them).
    fn tangent(&self, i: usize) -> f32 {
        let last = self.points.len() - 1;
        if i == 0 || i == last {
            // Flat at the ends, so the curve meets the flat outside.
            return 0.0;
        }
        let (a, b) = (self.slope(i - 1), self.slope(i));
        if a * b <= 0.0 {
            0.0
        } else {
            // Harmonic mean: never steeper than three times either side.
            2.0 * a * b / (a + b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spline_passes_its_points_and_is_flat_outside() {
        let s = Spline::new(&[(-1.0, -400.0), (0.0, 5.0), (0.5, 60.0), (1.0, 200.0)]);
        for &(x, y) in &s.points {
            assert!((s.at(x) - y).abs() < 1e-3);
        }
        assert_eq!(s.at(-5.0), -400.0);
        assert_eq!(s.at(7.0), 200.0);
    }

    #[test]
    fn a_spline_never_overshoots_rising_points() {
        let s = Spline::new(&[(0.0, 0.0), (0.1, 0.0), (0.2, 100.0), (0.3, 100.0), (1.0, 1000.0)]);
        let mut last = f32::MIN;
        for i in 0..=1000 {
            let y = s.at(i as f32 / 1000.0);
            assert!(y >= last - 1e-3, "falls at {i}");
            last = y;
        }
        assert!((0..100).all(|i| (-1e-3..=100.001).contains(&s.at(0.1 + i as f32 * 0.002))));
    }
}
