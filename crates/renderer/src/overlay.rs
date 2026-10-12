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
    /// Comment pins (device space): position and whether the comment is resolved.
    pub pins: Vec<(Point, bool)>,
    /// Text being edited: caret segment and selection quads (device space).
    pub caret: Option<(Point, Point)>,
    pub text_selection: Vec<[Point; 4]>,
    /// Path being drawn or edited (device space): outline, points (selected?), handle lines,
    /// the pen's preview segment, and the first point when a click would close the path.
    pub path: Option<BezPath>,
    pub path_anchors: Vec<(Point, bool)>,
    pub path_handles: Vec<(Point, Point)>,
    pub path_rubber: Option<BezPath>,
    pub path_closing: Option<Point>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.hover.is_none()
            && self.selected.is_empty()
            && self.selection_box.is_none()
            && self.marquee.is_none()
            && self.guides.is_empty()
            && self.pins.is_empty()
            && self.caret.is_none()
            && self.text_selection.is_empty()
            && self.path.is_none()
    }
}

pub fn paint_overlay(o: &Overlay, p: &mut impl Painter) {
    let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
    let line = scale;
    let accent = o.accent.unwrap_or(ACCENT);
    for q in &o.text_selection {
        let mut path = BezPath::new();
        path.move_to(q[0]);
        for &pt in &q[1..] {
            path.line_to(pt);
        }
        path.close_path();
        let fill = BrushSpec::Solid(accent.multiply_alpha(0.35));
        p.fill(
            Affine::IDENTITY,
            &Geom::Path(&path),
            Fill::NonZero,
            &fill,
            None,
        );
    }
    if let Some((a, b)) = o.caret {
        let mut path = BezPath::new();
        path.move_to(a);
        path.line_to(b);
        p.stroke(
            Affine::IDENTITY,
            &Geom::Path(&path),
            1.5 * scale,
            &BrushSpec::Solid(accent),
        );
    }
    for &(at, resolved) in &o.pins {
        // Speech-bubble pin: circle above the point with a white rim.
        let r = 9.0 * scale;
        let c = Point::new(at.x + r, at.y - r);
        let fill = if resolved {
            Color::from_rgba8(0x8E, 0x8E, 0x8E, 0xFF)
        } else {
            accent
        };
        let e = vello::kurbo::Ellipse::new(c, (r, r), 0.0);
        p.fill(
            Affine::IDENTITY,
            &Geom::Ellipse(e),
            Fill::NonZero,
            &BrushSpec::Solid(fill),
            None,
        );
        p.stroke(
            Affine::IDENTITY,
            &Geom::Ellipse(e),
            2.0 * scale,
            &BrushSpec::Solid(Color::from_rgba8(0x0A, 0x0A, 0x0A, 0xFF)),
        );
    }
    paint_path(o, p, accent, scale);
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

fn paint_path(o: &Overlay, p: &mut impl Painter, accent: Color, scale: f64) {
    let solid = BrushSpec::Solid(accent);
    if let Some(path) = &o.path {
        p.stroke(Affine::IDENTITY, &Geom::Path(path), scale, &solid);
    }
    if let Some(r) = &o.path_rubber {
        let faint = BrushSpec::Solid(accent.multiply_alpha(0.6));
        p.stroke(Affine::IDENTITY, &Geom::Path(r), scale, &faint);
    }
    let white = BrushSpec::Solid(Color::WHITE);
    for &(a, h) in &o.path_handles {
        let mut l = BezPath::new();
        l.move_to(a);
        l.line_to(h);
        p.stroke(Affine::IDENTITY, &Geom::Path(&l), scale, &solid);
        let e = vello::kurbo::Ellipse::new(h, (3.5 * scale, 3.5 * scale), 0.0);
        p.fill(
            Affine::IDENTITY,
            &Geom::Ellipse(e),
            Fill::NonZero,
            &white,
            None,
        );
        p.stroke(Affine::IDENTITY, &Geom::Ellipse(e), scale, &solid);
    }
    for &(a, selected) in &o.path_anchors {
        let r = Rect::from_center_size(a, (7.0 * scale, 7.0 * scale));
        let fill = if selected { &solid } else { &white };
        p.fill(Affine::IDENTITY, &Geom::Rect(r), Fill::NonZero, fill, None);
        p.stroke(Affine::IDENTITY, &Geom::Rect(r), scale, &solid);
    }
    if let Some(c) = o.path_closing {
        // A ring around the first point: clicking closes the shape.
        let e = vello::kurbo::Ellipse::new(c, (8.0 * scale, 8.0 * scale), 0.0);
        p.stroke(Affine::IDENTITY, &Geom::Ellipse(e), 1.5 * scale, &solid);
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
