//! Conversions from the Universal Design Model to kurbo/peniko types.

use ude_core::geometry::{PathData, PathVerb};
use ude_core::styles::{self, GradientKind, PaintKind};
use vello::kurbo::{Affine, BezPath, Point};
use vello::peniko::{self, BlendMode, Color, Compose, Fill, Gradient, Mix};

use crate::painter::BrushSpec;

pub fn affine(t: &ude_core::Affine) -> Affine {
    Affine::new(t.0.map(f64::from))
}

pub fn color(c: styles::Color, alpha: f32) -> Color {
    Color::new([c.r, c.g, c.b, c.a * alpha])
}

pub fn fill_rule(r: ude_core::FillRule) -> Fill {
    match r {
        ude_core::FillRule::NonZero => Fill::NonZero,
        ude_core::FillRule::EvenOdd => Fill::EvenOdd,
    }
}

/// `None` for modes that need no compositing layer (normal / pass-through).
pub fn blend(b: styles::BlendMode) -> Option<BlendMode> {
    use styles::BlendMode as B;
    let mix = match b {
        B::PassThrough | B::Normal => return None,
        B::Darken => Mix::Darken,
        B::Multiply => Mix::Multiply,
        B::ColorBurn | B::LinearBurn => Mix::ColorBurn, // LinearBurn: closest available; tracked in compat matrix
        B::Lighten => Mix::Lighten,
        B::Screen => Mix::Screen,
        B::ColorDodge => Mix::ColorDodge,
        B::LinearDodge => return Some(BlendMode::new(Mix::Normal, Compose::Plus)),
        B::Overlay => Mix::Overlay,
        B::SoftLight => Mix::SoftLight,
        B::HardLight => Mix::HardLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
    };
    Some(BlendMode::new(mix, Compose::SrcOver))
}

pub fn path(p: &PathData) -> BezPath {
    let mut out = BezPath::new();
    let pts = p.points();
    let pt = |i: usize| Point::new(f64::from(pts[i].x), f64::from(pts[i].y));
    let mut i = 0;
    for &verb in p.verbs() {
        match verb {
            PathVerb::MoveTo => out.move_to(pt(i)),
            PathVerb::LineTo => out.line_to(pt(i)),
            PathVerb::QuadTo => out.quad_to(pt(i), pt(i + 1)),
            PathVerb::CubicTo => out.curve_to(pt(i), pt(i + 1), pt(i + 2)),
            PathVerb::Close => out.close_path(),
        }
        i += verb.point_count();
    }
    out
}

/// Brush for one paint, plus the brush transform mapping gradient space onto the node box.
///
/// Gradient space convention (shared by all importers): linear runs from (0, 0.5) to
/// (1, 0.5); radial/diamond are centred at (0.5, 0.5) with radius 0.5; angular sweeps
/// around (0.5, 0.5). `Gradient::transform` maps gradient space into the unit box,
/// which is then scaled to the node size.
pub fn brush(
    paint: &styles::Paint,
    size: (f64, f64),
    alpha: f32,
) -> Option<(BrushSpec, Option<Affine>)> {
    if !paint.visible {
        return None;
    }
    let alpha = alpha * paint.opacity;
    match &paint.kind {
        PaintKind::Solid(c) => Some((BrushSpec::Solid(color(*c, alpha)), None)),
        PaintKind::Gradient(g) => {
            let stops: Vec<peniko::ColorStop> = g
                .stops
                .iter()
                .map(|s| (s.offset, color(s.color, alpha)).into())
                .collect();
            let grad = match g.kind {
                GradientKind::Linear => Gradient::new_linear((0.0, 0.5), (1.0, 0.5)),
                GradientKind::Radial | GradientKind::Diamond => {
                    Gradient::new_radial((0.5, 0.5), 0.5)
                }
                GradientKind::Angular => {
                    Gradient::new_sweep((0.5, 0.5), 0.0, std::f32::consts::TAU)
                }
            }
            .with_stops(stops.as_slice());
            let t = Affine::scale_non_uniform(size.0, size.1) * affine(&g.transform);
            Some((BrushSpec::Gradient(grad), Some(t)))
        }
        // Image paints render as a neutral placeholder until the image pipeline lands (M7).
        PaintKind::Image { .. } => Some((BrushSpec::Solid(placeholder(alpha)), None)),
    }
}

/// Placeholder for content not rendered yet (images until M7); `border.strong` token.
pub fn placeholder(alpha: f32) -> Color {
    Color::from_rgba8(0x3B, 0x41, 0x60, 255).multiply_alpha(alpha)
}
