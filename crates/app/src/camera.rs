//! Canvas camera: zoom and pan. Pure math, no window types, so it is unit-tested.

use ude_renderer::vello::kurbo::{Affine, Point, Rect, Vec2};

pub const MIN_ZOOM: f64 = 0.01;
pub const MAX_ZOOM: f64 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Document units per logical pixel (1.0 = 100%).
    pub zoom: f64,
    /// Offset in device pixels.
    pub pan: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl Camera {
    /// Document → device-pixel transform for a display with `scale_factor` (DPI).
    pub fn transform(&self, scale_factor: f64) -> Affine {
        Affine::translate(self.pan) * Affine::scale(self.zoom * scale_factor)
    }

    /// Zooms by `factor`, keeping the document point under `device_pt` fixed.
    pub fn zoom_at(&mut self, device_pt: Point, factor: f64, scale_factor: f64) {
        let doc_pt = self.transform(scale_factor).inverse() * device_pt;
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = device_pt.to_vec2() - doc_pt.to_vec2() * (self.zoom * scale_factor);
    }

    /// Sets an absolute zoom around `device_pt`.
    pub fn set_zoom_at(&mut self, device_pt: Point, zoom: f64, scale_factor: f64) {
        self.zoom_at(device_pt, zoom / self.zoom, scale_factor);
    }

    /// Frames `bounds` (document units) in a `width`×`height` device-pixel viewport.
    pub fn fit(&mut self, bounds: Rect, width: f64, height: f64, scale_factor: f64) {
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 || width <= 0.0 || height <= 0.0 {
            return;
        }
        const MARGIN: f64 = 0.92;
        let zoom = (width / bounds.width()).min(height / bounds.height()) * MARGIN / scale_factor;
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let s = self.zoom * scale_factor;
        let center = bounds.center().to_vec2() * s;
        self.pan = Vec2::new(width / 2.0, height / 2.0) - center;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut c = Camera {
            zoom: 1.0,
            pan: Vec2::new(30.0, -12.0),
        };
        let cursor = Point::new(400.0, 300.0);
        let before = c.transform(2.0).inverse() * cursor;
        c.zoom_at(cursor, 1.7, 2.0);
        let after = c.transform(2.0).inverse() * cursor;
        assert!(close(before, after));
        assert!((c.zoom - 1.7).abs() < 1e-12);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut c = Camera::default();
        c.zoom_at(Point::ORIGIN, 1e9, 1.0);
        assert_eq!(c.zoom, MAX_ZOOM);
        c.zoom_at(Point::ORIGIN, 1e-12, 1.0);
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn fit_centres_bounds() {
        let mut c = Camera::default();
        let bounds = Rect::new(1000.0, 500.0, 3000.0, 1500.0);
        c.fit(bounds, 1000.0, 800.0, 1.0);
        let center = c.transform(1.0) * bounds.center();
        assert!(close(center, Point::new(500.0, 400.0)));
        let mapped = c.transform(1.0).transform_rect_bbox(bounds);
        assert!(mapped.width() <= 1000.0 && mapped.height() <= 800.0);
    }
}
