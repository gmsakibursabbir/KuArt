//! Editor overlays drawn in device space on top of the document: hover outline,
//! selection outlines and handles, marquee. Colours follow docs/ui-design-system.md §6.

use vello::kurbo::{Affine, BezPath, Point, Rect};
use vello::peniko::{Color, Fill};

use crate::painter::{BrushSpec, Geom, Painter};

/// `accent.primary`
pub const ACCENT: Color = Color::from_rgba8(0xA7, 0x8B, 0xFA, 0xFF);
/// `accent.component`
pub const COMPONENT: Color = Color::from_rgba8(0xF4, 0x72, 0xB6, 0xFF);

/// A node's outline in device pixels (four corners, possibly rotated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub quad: [Point; 4],
    /// Components and instances use the component colour.
    pub component: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    pub hover: Option<Outline>,
    pub selected: Vec<Outline>,
    /// Box carrying the resize handles (single selection: the node's quad; multiple: their bounds).
    pub selection_box: Option<Outline>,
    pub marquee: Option<Rect>,
    /// Snap guide segments (device space).
    pub guides: Vec<(Point, Point)>,
    /// Device pixels per logical pixel, so lines and handles keep their on-screen size.
    pub scale: f64,
    /// Theme accent for selection, hover and guides (`None`: [`ACCENT`]).
    pub accent: Option<Color>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.hover.is_none()
            && self.selected.is_empty()
            && self.selection_box.is_none()
            && self.marquee.is_none()
            && self.guides.is_empty()
    }
}

pub fn paint_overlay(o: &Overlay, p: &mut impl Painter) {
    let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
    let line = scale;
    let accent = o.accent.unwrap_or(ACCENT);
    if let Some(m) = o.marquee {
        let fill = BrushSpec::Solid(accent.multiply_alpha(0.10));
        p.fill(Affine::IDENTITY, &Geom::Rect(m), Fill::NonZero, &fill, None);
        p.stroke(
            Affine::IDENTITY,
            &Geom::Rect(m),
            line,
            &BrushSpec::Solid(accent),
        );
    }
    for &(a, b) in &o.guides {
        let mut path = BezPath::new();
        path.move_to(a);
        path.line_to(b);
        p.stroke(
            Affine::IDENTITY,
            &Geom::Path(&path),
            line,
            &BrushSpec::Solid(COMPONENT),
        );
    }
    if let Some(h) = &o.hover {
        // Hovering a selected layer would just double its outline.
        if !o.selected.contains(h) {
            stroke_quad(p, h, line * 1.5, accent);
        }
    }
    for s in &o.selected {
        stroke_quad(p, s, line, accent);
    }
    if let Some(b) = &o.selection_box {
        if o.selected.len() > 1 {
            stroke_quad(p, b, line, accent);
        }
        let size = 8.0 * scale;
        let fill = BrushSpec::Solid(Color::WHITE);
        let border = BrushSpec::Solid(color_of(b, accent));
        for pt in handle_points(&b.quad) {
            let r = Rect::from_center_size(pt, (size, size));
            p.fill(Affine::IDENTITY, &Geom::Rect(r), Fill::NonZero, &fill, None);
            p.stroke(Affine::IDENTITY, &Geom::Rect(r), line, &border);
        }
    }
}

fn color_of(o: &Outline, accent: Color) -> Color {
    if o.component { COMPONENT } else { accent }
}

fn stroke_quad(p: &mut impl Painter, o: &Outline, width: f64, accent: Color) {
    let mut path = BezPath::new();
    path.move_to(o.quad[0]);
    for &pt in &o.quad[1..] {
        path.line_to(pt);
    }
    path.close_path();
    p.stroke(
        Affine::IDENTITY,
        &Geom::Path(&path),
        width,
        &BrushSpec::Solid(color_of(o, accent)),
    );
}

/// Corners and edge midpoints: the eight resize handles (used by M4 transform tools).
pub fn handle_points(q: &[Point; 4]) -> [Point; 8] {
    let mid = |a: Point, b: Point| a.midpoint(b);
    [
        q[0],
        mid(q[0], q[1]),
        q[1],
        mid(q[1], q[2]),
        q[2],
        mid(q[2], q[3]),
        q[3],
        mid(q[3], q[0]),
    ]
}
