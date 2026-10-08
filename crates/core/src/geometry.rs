//! Compact `f32` geometry: points, sizes, rects, affine transforms and packed paths.

use std::ops::Mul;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// Axis-aligned rectangle; `x0 <= x1`, `y0 <= y1` unless empty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    /// Identity for [`Rect::union`].
    pub const EMPTY: Rect = Rect {
        x0: f32::INFINITY,
        y0: f32::INFINITY,
        x1: f32::NEG_INFINITY,
        y1: f32::NEG_INFINITY,
    };

    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, origin.x + size.w, origin.y + size.h)
    }

    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }

    pub fn union_point(&self, p: Point) -> Rect {
        Rect::new(
            self.x0.min(p.x),
            self.y0.min(p.y),
            self.x1.max(p.x),
            self.y1.max(p.y),
        )
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
}

/// 2D affine transform `[a b c d e f]`: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f32; 6]);

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub const fn translate(x: f32, y: f32) -> Self {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub const fn scale(sx: f32, sy: f32) -> Self {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    pub fn translation(&self) -> Point {
        Point::new(self.0[4], self.0[5])
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    pub fn determinant(&self) -> f32 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }

    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if det.abs() < f32::EPSILON * 1e-3 || !det.is_finite() {
            return None;
        }
        let [a, b, c, d, e, f] = self.0;
        let inv = 1.0 / det;
        Some(Affine([
            d * inv,
            -b * inv,
            -c * inv,
            a * inv,
            (c * f - d * e) * inv,
            (b * e - a * f) * inv,
        ]))
    }

    /// Bounding box of a transformed rectangle.
    pub fn transform_rect_bbox(&self, r: Rect) -> Rect {
        let corners = [
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x0, r.y1),
            Point::new(r.x1, r.y1),
        ];
        corners
            .iter()
            .fold(Rect::EMPTY, |acc, &p| acc.union_point(self.apply(p)))
    }
}

impl Mul for Affine {
    type Output = Affine;

    /// `self * rhs` applies `rhs` first, then `self`.
    fn mul(self, rhs: Affine) -> Affine {
        let [a0, b0, c0, d0, e0, f0] = self.0;
        let [a1, b1, c1, d1, e1, f1] = rhs.0;
        Affine([
            a0 * a1 + c0 * b1,
            b0 * a1 + d0 * b1,
            a0 * c1 + c0 * d1,
            b0 * c1 + d0 * d1,
            a0 * e1 + c0 * f1 + e0,
            b0 * e1 + d0 * f1 + f0,
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// One byte per verb; points are stored separately so segments carry no enum padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PathVerb {
    MoveTo,
    LineTo,
    QuadTo,
    CubicTo,
    Close,
}

impl PathVerb {
    pub const fn point_count(self) -> usize {
        match self {
            PathVerb::MoveTo | PathVerb::LineTo => 1,
            PathVerb::QuadTo => 2,
            PathVerb::CubicTo => 3,
            PathVerb::Close => 0,
        }
    }
}

/// Packed, immutable path. Shared via `Arc` between nodes, instances and history.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PathData {
    verbs: Box<[PathVerb]>,
    points: Box<[Point]>,
}

impl PathData {
    pub fn verbs(&self) -> &[PathVerb] {
        &self.verbs
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Conservative bounds (control-point hull).
    pub fn bounds(&self) -> Rect {
        self.points
            .iter()
            .fold(Rect::EMPTY, |r, &p| r.union_point(p))
    }

    pub fn heap_bytes(&self) -> usize {
        self.verbs.len() + self.points.len() * size_of::<Point>()
    }
}

#[derive(Default)]
pub struct PathBuilder {
    verbs: Vec<PathVerb>,
    points: Vec<Point>,
}

impl PathBuilder {
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.push(PathVerb::MoveTo, &[Point::new(x, y)])
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.push(PathVerb::LineTo, &[Point::new(x, y)])
    }

    pub fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) -> &mut Self {
        self.push(PathVerb::QuadTo, &[Point::new(x1, y1), Point::new(x, y)])
    }

    pub fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) -> &mut Self {
        self.push(
            PathVerb::CubicTo,
            &[Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y)],
        )
    }

    pub fn close(&mut self) -> &mut Self {
        self.push(PathVerb::Close, &[])
    }

    fn push(&mut self, verb: PathVerb, pts: &[Point]) -> &mut Self {
        self.verbs.push(verb);
        self.points.extend_from_slice(pts);
        self
    }

    pub fn build(&mut self) -> PathData {
        PathData {
            verbs: std::mem::take(&mut self.verbs).into_boxed_slice(),
            points: std::mem::take(&mut self.points).into_boxed_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
    }

    #[test]
    fn affine_compose_and_inverse() {
        let t = Affine::translate(10.0, 5.0)
            * Affine::rotate(std::f32::consts::FRAC_PI_2)
            * Affine::scale(2.0, 3.0);
        let p = Point::new(1.0, 1.0);
        let q = t.apply(p);
        // scale → (2,3); rotate 90° → (-3,2); translate → (7,7)
        assert!(close(q, Point::new(7.0, 7.0)));
        let back = t.inverse().unwrap().apply(q);
        assert!(close(back, p));
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn path_bounds() {
        let path = PathBuilder::default()
            .move_to(0.0, 0.0)
            .cubic_to(10.0, -5.0, 20.0, 5.0, 30.0, 0.0)
            .close()
            .build();
        assert_eq!(path.bounds(), Rect::new(0.0, -5.0, 30.0, 5.0));
        let expected: usize = path.verbs().iter().map(|v| v.point_count()).sum();
        assert_eq!(expected, path.points().len());
    }
}
