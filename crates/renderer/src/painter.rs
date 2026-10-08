//! Backend-neutral drawing interface. Both the GPU (`vello::Scene`) and CPU
//! (`vello_cpu::RenderContext`) backends implement it, so the document traversal is
//! written once and both backends render the same thing.

use vello::kurbo::{Affine, BezPath, Ellipse, Rect, RoundedRect, Shape, Stroke};
use vello::peniko::{BlendMode, Color, Fill, Gradient};

/// Geometry in local coordinates. Analytic shapes stay analytic on the GPU path.
pub enum Geom<'a> {
    Rect(Rect),
    RoundedRect(RoundedRect),
    Ellipse(Ellipse),
    Path(&'a BezPath),
}

impl Geom<'_> {
    pub fn to_path(&self) -> BezPath {
        const TOLERANCE: f64 = 0.1;
        match self {
            Geom::Rect(r) => r.to_path(TOLERANCE),
            Geom::RoundedRect(r) => r.to_path(TOLERANCE),
            Geom::Ellipse(e) => e.to_path(TOLERANCE),
            Geom::Path(p) => (*p).clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BrushSpec {
    Solid(Color),
    Gradient(Gradient),
}

pub trait Painter {
    fn fill(
        &mut self,
        transform: Affine,
        geom: &Geom<'_>,
        rule: Fill,
        brush: &BrushSpec,
        brush_transform: Option<Affine>,
    );
    fn stroke(&mut self, transform: Affine, geom: &Geom<'_>, width: f64, brush: &BrushSpec);
    /// Blurred rounded-rect shadow (analytic on both backends).
    fn shadow(&mut self, transform: Affine, rect: Rect, color: Color, radius: f64, std_dev: f64);
    fn push_layer(&mut self, transform: Affine, clip: &Geom<'_>, blend: BlendMode, alpha: f32);
    fn pop_layer(&mut self);
}

macro_rules! with_shape {
    ($geom:expr, |$s:ident| $body:expr) => {
        match $geom {
            Geom::Rect($s) => $body,
            Geom::RoundedRect($s) => $body,
            Geom::Ellipse($s) => $body,
            Geom::Path($s) => {
                let $s: &BezPath = $s;
                $body
            }
        }
    };
}

impl Painter for vello::Scene {
    fn fill(
        &mut self,
        transform: Affine,
        geom: &Geom<'_>,
        rule: Fill,
        brush: &BrushSpec,
        brush_transform: Option<Affine>,
    ) {
        with_shape!(geom, |s| match brush {
            BrushSpec::Solid(c) =>
                vello::Scene::fill(self, rule, transform, *c, brush_transform, s),
            BrushSpec::Gradient(g) =>
                vello::Scene::fill(self, rule, transform, g, brush_transform, s),
        })
    }

    fn stroke(&mut self, transform: Affine, geom: &Geom<'_>, width: f64, brush: &BrushSpec) {
        let style = Stroke::new(width);
        with_shape!(geom, |s| match brush {
            BrushSpec::Solid(c) => vello::Scene::stroke(self, &style, transform, *c, None, s),
            BrushSpec::Gradient(g) => vello::Scene::stroke(self, &style, transform, g, None, s),
        })
    }

    fn shadow(&mut self, transform: Affine, rect: Rect, color: Color, radius: f64, std_dev: f64) {
        self.draw_blurred_rounded_rect(transform, rect, color, radius, std_dev);
    }

    fn push_layer(&mut self, transform: Affine, clip: &Geom<'_>, blend: BlendMode, alpha: f32) {
        with_shape!(clip, |s| vello::Scene::push_layer(
            self,
            Fill::NonZero,
            blend,
            alpha,
            transform,
            s
        ))
    }

    fn pop_layer(&mut self) {
        vello::Scene::pop_layer(self);
    }
}

impl Painter for vello_cpu::RenderContext {
    fn fill(
        &mut self,
        transform: Affine,
        geom: &Geom<'_>,
        rule: Fill,
        brush: &BrushSpec,
        brush_transform: Option<Affine>,
    ) {
        self.set_transform(transform);
        self.set_fill_rule(rule);
        set_paint(self, brush, brush_transform);
        match geom {
            Geom::Rect(r) => self.fill_rect(r),
            Geom::Path(p) => self.fill_path(p),
            other => self.fill_path(&other.to_path()),
        }
    }

    fn stroke(&mut self, transform: Affine, geom: &Geom<'_>, width: f64, brush: &BrushSpec) {
        self.set_transform(transform);
        self.set_stroke(Stroke::new(width));
        set_paint(self, brush, None);
        match geom {
            Geom::Rect(r) => self.stroke_rect(r),
            Geom::Path(p) => self.stroke_path(p),
            other => self.stroke_path(&other.to_path()),
        }
    }

    fn shadow(&mut self, transform: Affine, rect: Rect, color: Color, radius: f64, std_dev: f64) {
        self.set_transform(transform);
        self.set_paint(color);
        self.reset_paint_transform();
        self.fill_blurred_rounded_rect(&rect, radius as f32, std_dev as f32, false);
    }

    fn push_layer(&mut self, transform: Affine, clip: &Geom<'_>, blend: BlendMode, alpha: f32) {
        // vello_cpu applies the current transform to the clip path.
        self.set_transform(transform);
        let path = clip.to_path();
        vello_cpu::RenderContext::push_layer(
            self,
            Some(&path),
            Some(blend),
            Some(alpha),
            None,
            None,
        );
    }

    fn pop_layer(&mut self) {
        vello_cpu::RenderContext::pop_layer(self);
    }
}

fn set_paint(
    cx: &mut vello_cpu::RenderContext,
    brush: &BrushSpec,
    brush_transform: Option<Affine>,
) {
    match brush {
        BrushSpec::Solid(c) => cx.set_paint(*c),
        BrushSpec::Gradient(g) => cx.set_paint(g.clone()),
    }
    match brush_transform {
        Some(t) => cx.set_paint_transform(t),
        None => cx.reset_paint_transform(),
    }
}
