//! Renders a page of the Universal Design Model through a [`Painter`].
//!
//! The same traversal drives the GPU backend (`vello::Scene`) and the CPU backend
//! (`vello_cpu::RenderContext`, also used for headless export). See
//! `docs/rendering-strategy.md`.
//!
//! M2 scope: shapes, vectors, solid and gradient fills, centre-aligned strokes,
//! drop shadows on box-like nodes, opacity/blend/clip layers, viewport culling and
//! level-of-detail. Text is drawn greeked (as bars) until M6, images as placeholders
//! until M7, and masks are skipped until phase 2.

pub mod convert;
pub mod images;
pub mod overlay;
pub mod painter;
pub mod pdf;
pub mod text;

pub use images::{DecodedImage, ImageCache};
pub use overlay::{Outline, Overlay, paint_overlay};
pub use painter::{BrushSpec, CpuPainter, Geom, GlyphPos, Painter};
pub use text::TextEngine;

/// Per-window caches the traversal needs: text layouts and decoded images.
#[derive(Default)]
pub struct RenderCaches {
    pub text: TextEngine,
    pub images: ImageCache,
}
pub use {vello, vello_cpu};

use ude_core::scene::NodeFlags;
use ude_core::styles::{Effect, ImageScaleMode, PaintKind};
use ude_core::{Document, Node, NodeId, NodeKind};
use vello::kurbo::{Affine, BezPath, Ellipse, Point, Rect, RoundedRect, RoundedRectRadii};
use vello::peniko::{BlendMode, Fill};

/// Nodes smaller than this on screen (in device pixels) are not drawn.
const MIN_VISIBLE_PX: f64 = 0.5;
/// Text smaller than this on screen is drawn as bars (unreadable anyway, and far cheaper).
const GREEK_BELOW_PX: f64 = 4.0;

/// Document-to-device mapping and the device-pixel viewport.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Maps document coordinates to device pixels (zoom, pan, DPI scale).
    pub transform: Affine,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub visited: u32,
    pub drawn: u32,
    pub culled: u32,
    pub layers: u32,
}

/// Paints `page`'s content. Returns traversal statistics for the diagnostics panel.
pub fn paint_page(
    doc: &Document,
    page: NodeId,
    view: &View,
    painter: &mut impl Painter,
    caches: &mut RenderCaches,
) -> FrameStats {
    caches.images.next_frame();
    let mut cx = Cx {
        doc,
        painter,
        caches,
        viewport: Rect::new(0.0, 0.0, view.width, view.height),
        stats: FrameStats::default(),
    };
    for &child in doc.children(page) {
        cx.node(child, view.transform, 1.0);
    }
    cx.stats
}

/// Document-space bounds of what `id` paints: its own box, or for pages and groups the
/// union of their children (strokes and effects included).
pub fn export_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let own = |n: NodeId| {
        let node = doc.node(n)?;
        let o = f64::from(node.paint_outset());
        let local = Rect::new(
            -o,
            -o,
            f64::from(node.size.w) + o,
            f64::from(node.size.h) + o,
        );
        Some(convert::affine(&doc.world_transform(n)).transform_rect_bbox(local))
    };
    let has_box = doc
        .node(id)
        .is_some_and(|n| n.size.w > 0.0 || n.size.h > 0.0);
    if has_box {
        own(id)
    } else {
        doc.descendants(id)
            .skip(1)
            .filter_map(own)
            .filter(|r| r.area() > 0.0)
            .reduce(|a, b| a.union(b))
    }
}

/// Renders `id` (a page or any layer) offscreen at `scale` on the CPU backend.
/// Returns width, height and straight (non-premultiplied) RGBA8 pixels.
pub fn render_to_rgba(
    doc: &Document,
    id: NodeId,
    scale: f64,
    caches: &mut RenderCaches,
) -> Option<(u32, u32, Vec<u8>)> {
    let bounds = export_bounds(doc, id)?;
    let (w, h) = (
        (bounds.width() * scale).ceil(),
        (bounds.height() * scale).ceil(),
    );
    if !(1.0..=f64::from(u16::MAX)).contains(&w) || !(1.0..=f64::from(u16::MAX)).contains(&h) {
        return None;
    }
    let (w, h) = (w as u16, h as u16);
    let view = Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0));
    let mut cx = vello_cpu::RenderContext::new(w, h);
    let mut resources = vello_cpu::Resources::new();
    let mut painter = CpuPainter {
        cx: &mut cx,
        resources: &mut resources,
    };
    paint_for_export(
        doc,
        id,
        view,
        Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        &mut painter,
        caches,
    );
    cx.flush();
    let mut pixmap = vello_cpu::Pixmap::new(w, h);
    cx.render(&mut pixmap, &mut resources);
    Some((
        u32::from(w),
        u32::from(h),
        pixmap.take_rgba8(vello::peniko::ImageAlphaType::Alpha),
    ))
}

struct Cx<'a, P: Painter> {
    doc: &'a Document,
    painter: &'a mut P,
    caches: &'a mut RenderCaches,
    viewport: Rect,
    stats: FrameStats,
}

impl<P: Painter> Cx<'_, P> {
    fn node(&mut self, id: NodeId, parent: Affine, alpha: f32) {
        let Some(n) = self.doc.node(id) else { return };
        self.stats.visited += 1;
        // Masks are not composited yet (phase 2); never paint the mask shape itself.
        if !n.is_visible() || n.opacity <= 0.0 || n.flags.contains(NodeFlags::IS_MASK) {
            return;
        }

        let transform = parent * convert::affine(&n.transform);
        let (w, h) = (f64::from(n.size.w), f64::from(n.size.h));
        let leaf = n.children.is_empty();
        let clips = n.flags.contains(NodeFlags::CLIPS_CONTENT);

        // Only leaves and clipping containers have reliable bounds; other containers
        // (groups) may have children outside their own box.
        if leaf || clips {
            let outset = f64::from(n.paint_outset());
            let bounds =
                transform.transform_rect_bbox(Rect::new(-outset, -outset, w + outset, h + outset));
            if !bounds.overlaps(self.viewport)
                || (bounds.width() < MIN_VISIBLE_PX && bounds.height() < MIN_VISIBLE_PX)
            {
                self.stats.culled += 1;
                return;
            }
        }

        let blend = convert::blend(n.blend);
        // A layer is needed only when it changes the result.
        let group_opacity = !leaf && n.opacity < 1.0 && n.children.len() > 1;
        // Leaves need one only for a blend mode (e.g. a Photoshop multiply layer).
        let layer = blend.is_some() || (!leaf && (clips || group_opacity));
        let (layer_alpha, inner_alpha) = if layer {
            (alpha * n.opacity, 1.0)
        } else {
            (1.0, alpha * n.opacity)
        };
        let own = local_geometry(n);

        if let Some(g) = &own {
            self.shadows(
                n,
                transform,
                g,
                inner_alpha * if layer { layer_alpha } else { 1.0 },
            );
        }
        if layer {
            self.stats.layers += 1;
            let blend = blend.unwrap_or_else(|| BlendMode::from(vello::peniko::Mix::Normal));
            match (&own, clips) {
                (Some(g), true) => {
                    self.painter
                        .push_layer(transform, &g.geom(), blend, layer_alpha)
                }
                // Non-clipping layer: clip to the viewport in device space.
                _ => self.painter.push_layer(
                    Affine::IDENTITY,
                    &Geom::Rect(self.viewport),
                    blend,
                    layer_alpha,
                ),
            }
        }

        self.fills(n, transform, own.as_ref(), inner_alpha);
        if !leaf {
            for &child in &n.children {
                self.node(child, transform, inner_alpha);
            }
        }
        if let Some(g) = &own {
            self.strokes(n, transform, g, inner_alpha);
        }
        if layer {
            self.painter.pop_layer();
        }
    }

    fn fills(&mut self, n: &Node, transform: Affine, own: Option<&Shape>, alpha: f32) {
        let size = (f64::from(n.size.w), f64::from(n.size.h));
        match (&n.kind, own) {
            (NodeKind::Text(text), _) => {
                let on_screen = text.styles.first().map_or(0.0, |s| f64::from(s.size))
                    * transform.determinant().abs().sqrt();
                if on_screen < GREEK_BELOW_PX {
                    self.greeked_text(n, text, transform, alpha);
                } else {
                    self.caches.text.draw(
                        self.painter,
                        text,
                        &self.doc.strings,
                        (n.size.w, n.size.h),
                        transform,
                        alpha,
                    );
                    self.stats.drawn += 1;
                }
            }
            (NodeKind::Raster { asset }, Some(g)) => {
                // A raster layer's pixels fill its box exactly (PSD layers, placed images).
                if !self.image(*asset, transform, size, ImageScaleMode::Fill, None, alpha) {
                    let brush = BrushSpec::Solid(convert::placeholder(alpha));
                    self.painter
                        .fill(transform, &g.geom(), Fill::NonZero, &brush, None);
                }
                self.stats.drawn += 1;
            }
            (_, Some(g)) => {
                for paint in n.fills.as_slice() {
                    if !paint.visible {
                        continue;
                    }
                    if let PaintKind::Image { asset, mode, .. } = &paint.kind {
                        let a = alpha * paint.opacity;
                        if !self.image(*asset, transform, size, *mode, Some(&g.geom()), a) {
                            let brush = BrushSpec::Solid(convert::placeholder(a));
                            self.painter
                                .fill(transform, &g.geom(), g.rule, &brush, None);
                        }
                    } else if let Some((brush, bt)) = convert::brush(paint, size, alpha) {
                        self.painter.fill(transform, &g.geom(), g.rule, &brush, bt);
                    }
                }
                if !n.fills.is_empty() {
                    self.stats.drawn += 1;
                }
            }
            _ => {}
        }
    }

    /// Draws an image into the node box (`size`, local units). `Fill` covers the box (cropping,
    /// clipped to `clip`), `Fit` fits inside it; crop/tile are drawn as fill for now.
    /// Returns false when the image is unavailable, so the caller can draw a placeholder.
    fn image(
        &mut self,
        asset: ude_core::AssetId,
        transform: Affine,
        size: (f64, f64),
        mode: ImageScaleMode,
        clip: Option<&Geom<'_>>,
        alpha: f32,
    ) -> bool {
        let src = match self.doc.assets.get(asset).map(|e| e.kind) {
            Some(ude_core::AssetKind::Image { width, height }) if width > 0 && height > 0 => {
                (width, height)
            }
            _ => return false,
        };
        let device = transform.transform_rect_bbox(Rect::new(0.0, 0.0, size.0, size.1));
        let needed = (device.width(), device.height());
        let Some(img) = self.caches.images.get(&self.doc.assets, asset, src, needed) else {
            return false;
        };
        let (sw, sh) = (f64::from(src.0), f64::from(src.1));
        let scale = match mode {
            ImageScaleMode::Fit => (size.0 / sw).min(size.1 / sh),
            _ => (size.0 / sw).max(size.1 / sh),
        };
        // Centre the scaled image in the box, then map decoded pixels to source pixels.
        let offset = ((size.0 - sw * scale) / 2.0, (size.1 - sh * scale) / 2.0);
        let t = transform
            * Affine::translate(offset)
            * Affine::scale(scale)
            * Affine::scale_non_uniform(sw / f64::from(img.width), sh / f64::from(img.height));
        let clipped = clip.is_some() && mode != ImageScaleMode::Fit;
        if let (true, Some(c)) = (clipped, clip) {
            self.painter.push_layer(
                transform,
                c,
                BlendMode::from(vello::peniko::Mix::Normal),
                1.0,
            );
        }
        self.painter.image(t, &img, alpha);
        if clipped {
            self.painter.pop_layer();
        }
        true
    }

    fn strokes(&mut self, n: &Node, transform: Affine, own: &Shape, alpha: f32) {
        let Some(stroke) = n.ext.as_ref().and_then(|e| e.strokes.as_deref()) else {
            return;
        };
        if stroke.width <= 0.0 {
            return;
        }
        let size = (f64::from(n.size.w), f64::from(n.size.h));
        // Inside/outside alignment is approximated by centre alignment until phase 2.
        for paint in stroke.paints.as_slice() {
            if let Some((brush, _)) = convert::brush(paint, size, alpha) {
                self.painter
                    .stroke(transform, &own.geom(), f64::from(stroke.width), &brush);
            }
        }
    }

    /// Drop shadows for box-like shapes, using the analytic blurred rounded rect.
    fn shadows(&mut self, n: &Node, transform: Affine, own: &Shape, alpha: f32) {
        let Some(ext) = &n.ext else { return };
        let ShapeKind::Box(radius) = own.kind else {
            return;
        };
        for fx in ext.effects.iter() {
            if let Effect::DropShadow(s) = fx
                && s.visible
            {
                let rect = Rect::new(0.0, 0.0, f64::from(n.size.w), f64::from(n.size.h))
                    .inflate(f64::from(s.spread), f64::from(s.spread));
                let t =
                    transform * Affine::translate((f64::from(s.offset.x), f64::from(s.offset.y)));
                // Design tools specify blur as a radius ≈ 2σ.
                self.painter.shadow(
                    t,
                    rect,
                    convert::color(s.color, alpha),
                    radius,
                    f64::from(s.blur) / 2.0,
                );
            }
        }
    }

    /// Greeked text: one bar per estimated line, for text too small to read.
    fn greeked_text(
        &mut self,
        n: &Node,
        text: &ude_core::typography::TextData,
        transform: Affine,
        alpha: f32,
    ) {
        let Some(style) = text.styles.first() else {
            return;
        };
        let size = f64::from(style.size.max(1.0));
        let line_h = size * 1.4;
        let width = f64::from(n.size.w).max(size);
        let text_w = text.content.chars().count() as f64 * size * 0.52;
        let lines = (text_w / width).ceil().max(1.0) as usize;
        let color = style
            .fills
            .as_slice()
            .iter()
            .find_map(|p| match &p.kind {
                ude_core::styles::PaintKind::Solid(c) if p.visible => Some(*c),
                _ => None,
            })
            .unwrap_or(ude_core::styles::Color::BLACK);
        let brush = BrushSpec::Solid(convert::color(color, alpha * 0.55));
        let mut remaining = text_w;
        for i in 0..lines {
            let w = remaining.min(width);
            remaining -= w;
            let y = i as f64 * line_h + (line_h - size * 0.55) / 2.0;
            let bar = RoundedRect::new(0.0, y, w, y + size * 0.55, size * 0.15);
            self.painter.fill(
                transform,
                &Geom::RoundedRect(bar),
                Fill::NonZero,
                &brush,
                None,
            );
        }
        self.stats.drawn += 1;
    }
}

enum ShapeKind {
    /// Box-like shape with a uniform-ish corner radius (shadow fast path applies).
    Box(f64),
    Other,
}

/// A node's own outline in local coordinates.
struct Shape {
    path: Option<BezPath>,
    analytic: Option<AnalyticShape>,
    rule: Fill,
    kind: ShapeKind,
}

enum AnalyticShape {
    Rect(Rect),
    Rounded(RoundedRect),
    Ellipse(Ellipse),
}

impl Shape {
    fn geom(&self) -> Geom<'_> {
        match (&self.analytic, &self.path) {
            (Some(AnalyticShape::Rect(r)), _) => Geom::Rect(*r),
            (Some(AnalyticShape::Rounded(r)), _) => Geom::RoundedRect(*r),
            (Some(AnalyticShape::Ellipse(e)), _) => Geom::Ellipse(*e),
            (None, Some(p)) => Geom::Path(p),
            (None, None) => Geom::Rect(Rect::ZERO),
        }
    }
}

fn local_geometry(n: &Node) -> Option<Shape> {
    let (w, h) = (f64::from(n.size.w), f64::from(n.size.h));
    let rect = Rect::new(0.0, 0.0, w, h);
    let analytic = |a, kind| {
        Some(Shape {
            path: None,
            analytic: Some(a),
            rule: Fill::NonZero,
            kind,
        })
    };
    let path = |p: BezPath, rule| {
        Some(Shape {
            path: Some(p),
            analytic: None,
            rule,
            kind: ShapeKind::Other,
        })
    };
    match &n.kind {
        NodeKind::Rectangle
        | NodeKind::Frame
        | NodeKind::Component(_)
        | NodeKind::ComponentSet
        | NodeKind::Instance(_)
        | NodeKind::Raster { .. }
        | NodeKind::Preserved { .. } => {
            let [tl, tr, br, bl] = n.corner_radii().map(f64::from);
            if tl == 0.0 && tr == 0.0 && br == 0.0 && bl == 0.0 {
                analytic(AnalyticShape::Rect(rect), ShapeKind::Box(0.0))
            } else {
                let radii = RoundedRectRadii::new(tl, tr, br, bl);
                analytic(
                    AnalyticShape::Rounded(RoundedRect::from_rect(rect, radii)),
                    ShapeKind::Box(tl.max(tr).max(br).max(bl)),
                )
            }
        }
        NodeKind::Ellipse(_) => analytic(
            AnalyticShape::Ellipse(Ellipse::from_rect(rect)),
            ShapeKind::Other,
        ),
        NodeKind::Polygon { sides } => path(
            regular_polygon(w, h, usize::from((*sides).max(3)), None),
            Fill::NonZero,
        ),
        NodeKind::Star {
            points,
            inner_ratio,
        } => path(
            regular_polygon(
                w,
                h,
                usize::from((*points).max(3)),
                Some(f64::from(*inner_ratio)),
            ),
            Fill::NonZero,
        ),
        NodeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((w, 0.0));
            path(p, Fill::NonZero)
        }
        NodeKind::Vector(v) => {
            let mut p = BezPath::new();
            let mut rule = Fill::NonZero;
            for vp in v.paths.iter() {
                p.extend(convert::path(&vp.path));
                rule = convert::fill_rule(vp.fill_rule);
            }
            path(p, rule)
        }
        NodeKind::Text(_) => analytic(AnalyticShape::Rect(rect), ShapeKind::Other),
        NodeKind::Document
        | NodeKind::Page
        | NodeKind::Group
        | NodeKind::BooleanOp(_)
        | NodeKind::Slice => None,
    }
}

/// Polygon (or star, with `inner` radius ratio) inscribed in the `w`×`h` box, first point at the top.
fn regular_polygon(w: f64, h: f64, n: usize, inner: Option<f64>) -> BezPath {
    let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
    let count = if inner.is_some() { n * 2 } else { n };
    let mut p = BezPath::new();
    for i in 0..count {
        let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / count as f64;
        let r = if i % 2 == 1 {
            inner.unwrap_or(1.0)
        } else {
            1.0
        };
        let pt = Point::new(cx + rx * r * a.cos(), cy + ry * r * a.sin());
        if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}

/// Draws `id` (a page's layers, or one layer as placed on the canvas) with `view` mapping
/// document space to the output. Shared by raster and PDF export.
pub(crate) fn paint_for_export(
    doc: &Document,
    id: NodeId,
    view: Affine,
    viewport: Rect,
    painter: &mut impl Painter,
    caches: &mut RenderCaches,
) {
    caches.images.next_frame();
    let mut tcx = Cx {
        doc,
        painter,
        caches,
        viewport,
        stats: FrameStats::default(),
    };
    let is_page = doc
        .node(id)
        .is_some_and(|n| matches!(n.kind, NodeKind::Page | NodeKind::Document));
    if is_page {
        for &child in doc.children(id) {
            tcx.node(child, view, 1.0);
        }
    } else {
        // Start from the parent's world transform so the layer renders exactly as on canvas.
        let parent = doc
            .node(id)
            .and_then(|n| n.parent)
            .map_or(Affine::IDENTITY, |p| {
                convert::affine(&doc.world_transform(p))
            });
        tcx.node(id, view * parent, 1.0);
    }
}

/// Raster export formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterFormat {
    Png,
    /// Flattened onto white (JPEG has no transparency), quality 90.
    Jpeg,
    /// Lossless WebP.
    WebP,
}

impl RasterFormat {
    /// From a file extension (`png`, `jpg`/`jpeg`, `webp`).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }
}

/// Renders `id` at `scale` and encodes it.
pub fn export_raster(
    doc: &Document,
    id: NodeId,
    scale: f64,
    format: RasterFormat,
    caches: &mut RenderCaches,
) -> Result<Vec<u8>, String> {
    export_raster_with(doc, id, scale, format, Compression::default(), caches)
}

/// Export compression (the Design tab's "Compress" and quality settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compression {
    /// PNG and WebP: reduce to an optimised palette of at most 256 colours (lossy, like
    /// TinyPNG). JPEG is always lossy.
    pub palette: bool,
    /// 10–100: JPEG quality; for palettes, higher means slower, more accurate colour search.
    pub quality: u8,
}

impl Default for Compression {
    fn default() -> Self {
        Self {
            palette: false,
            quality: 90,
        }
    }
}

/// Like [`export_raster`], with compression settings.
pub fn export_raster_with(
    doc: &Document,
    id: NodeId,
    scale: f64,
    format: RasterFormat,
    compression: Compression,
    caches: &mut RenderCaches,
) -> Result<Vec<u8>, String> {
    let (w, h, rgba) =
        render_to_rgba(doc, id, scale, caches).ok_or("nothing to export (empty or too large)")?;
    encode_rgba(w, h, rgba, format, compression)
}

/// Encodes straight-alpha RGBA8 pixels.
pub fn encode_rgba(
    w: u32,
    h: u32,
    mut rgba: Vec<u8>,
    format: RasterFormat,
    c: Compression,
) -> Result<Vec<u8>, String> {
    let quality = c.quality.clamp(10, 100);
    if c.palette && format == RasterFormat::Png {
        // A palette wins on photos and noisy art; flat or smooth artwork is often smaller as
        // full-colour PNG (its row filters predict gradients). Keep whichever is smaller.
        let palette = palette_png(w, h, &rgba, quality)?;
        let full = encode_rgba(
            w,
            h,
            rgba,
            format,
            Compression {
                palette: false,
                quality,
            },
        )?;
        return Ok(if palette.len() < full.len() {
            palette
        } else {
            full
        });
    }
    if c.palette && format == RasterFormat::WebP {
        // Fewer distinct colours compress far better in lossless WebP.
        quantize_in_place(&mut rgba, quality);
    }
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("pixel buffer size mismatch")?;
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    match format {
        RasterFormat::Png => image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new_with_quality(
                &mut cursor,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            ),
            &img,
            w,
            h,
            image::ExtendedColorType::Rgba8,
        ),
        RasterFormat::WebP => img.write_to(&mut cursor, image::ImageFormat::WebP),
        RasterFormat::Jpeg => {
            let rgb = image::RgbImage::from_fn(w, h, |x, y| {
                let [r, g, b, a] = img.get_pixel(x, y).0;
                let over = |c: u8| {
                    ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8
                };
                image::Rgb([over(r), over(g), over(b)])
            });
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality)
                .encode_image(&rgb)
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(out)
}

/// NeuQuant sampling factor: 1 (best) … 30 (fastest), from a 10–100 quality.
fn sample_factor(quality: u8) -> i32 {
    (31 - i32::from(quality) * 3 / 10).clamp(1, 30)
}

fn quantize_in_place(rgba: &mut [u8], quality: u8) {
    let nq = color_quant::NeuQuant::new(sample_factor(quality), 256, rgba);
    for px in rgba.chunks_exact_mut(4) {
        let i = nq.index_of(px);
        if let Some(c) = nq.lookup(i) {
            px.copy_from_slice(&c);
        }
    }
}

/// Indexed PNG with an optimised palette (with alpha) and maximum compression.
fn palette_png(w: u32, h: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    let nq = color_quant::NeuQuant::new(sample_factor(quality), 256, rgba);
    let map = nq.color_map_rgba();
    let indices: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|px| nq.index_of(px) as u8)
        .collect();
    let palette: Vec<u8> = map
        .chunks_exact(4)
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect();
    let trns: Vec<u8> = map.chunks_exact(4).map(|c| c[3]).collect();
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(palette);
        if trns.iter().any(|&a| a < 255) {
            e.set_trns(trns);
        }
        e.set_compression(png::Compression::High);
        let mut writer = e.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&indices)
            .map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Encodes `id` as a PNG at `scale` (see [`render_to_rgba`]).
pub fn export_png(
    doc: &Document,
    id: NodeId,
    scale: f64,
    caches: &mut RenderCaches,
) -> Result<Vec<u8>, String> {
    export_raster(doc, id, scale, RasterFormat::Png, caches)
}

pub use pdf::export_pdf;

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::styles::{Color, Paint};
    use ude_core::{Paints, Symbol};

    /// Renders with the CPU backend and returns premultiplied RGBA8 pixels.
    fn render_cpu(doc: &Document, page: NodeId, w: u16, h: u16) -> (Vec<u8>, FrameStats) {
        let mut cx = vello_cpu::RenderContext::new(w, h);
        let view = View {
            transform: Affine::IDENTITY,
            width: f64::from(w),
            height: f64::from(h),
        };
        let mut resources = vello_cpu::Resources::new();
        let stats = paint_page(
            doc,
            page,
            &view,
            &mut CpuPainter {
                cx: &mut cx,
                resources: &mut resources,
            },
            &mut RenderCaches::default(),
        );
        let mut pixmap = vello_cpu::Pixmap::new(w, h);
        cx.flush();
        cx.render(&mut pixmap, &mut resources);
        (pixmap.data_as_u8_slice().to_vec(), stats)
    }

    fn page_with(nodes: Vec<Node>) -> (Document, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        for n in nodes {
            doc.build_child(page, n).unwrap();
        }
        (doc, page)
    }

    fn pixel(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    #[test]
    fn draws_rect_where_it_is_and_culls_offscreen() {
        let red = Paints::new(vec![Paint::solid(Color::from_rgba8(255, 0, 0, 255))]);
        let on = Node::new(NodeKind::Rectangle, Symbol::EMPTY)
            .with_transform(ude_core::Affine::translate(10.0, 10.0))
            .with_size(20.0, 20.0)
            .with_fills(red.clone());
        let off = Node::new(NodeKind::Rectangle, Symbol::EMPTY)
            .with_transform(ude_core::Affine::translate(500.0, 500.0))
            .with_size(20.0, 20.0)
            .with_fills(red);
        let (doc, page) = page_with(vec![on, off]);
        let (px, stats) = render_cpu(&doc, page, 64, 64);
        assert_eq!(pixel(&px, 64, 20, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&px, 64, 5, 5), [0, 0, 0, 0]);
        assert_eq!(stats.drawn, 1);
        assert_eq!(stats.culled, 1);
    }

    #[test]
    fn frame_clips_children_and_opacity_applies() {
        let mut frame = Node::new(NodeKind::Frame, Symbol::EMPTY).with_size(32.0, 32.0);
        frame.flags.set(NodeFlags::CLIPS_CONTENT, true);
        let (mut doc, page) = page_with(vec![frame]);
        let frame = doc.children(page)[0];
        let mut child = Node::new(NodeKind::Rectangle, Symbol::EMPTY)
            .with_transform(ude_core::Affine::translate(16.0, 0.0))
            .with_size(64.0, 32.0)
            .with_fills(Paints::new(vec![Paint::solid(Color::from_rgba8(
                0, 0, 255, 255,
            ))]));
        child.opacity = 0.5;
        doc.build_child(frame, child).unwrap();
        let (px, stats) = render_cpu(&doc, page, 64, 64);
        let inside = pixel(&px, 64, 24, 16);
        assert!(
            (120..=135).contains(&inside[3]),
            "half-transparent blue, got {inside:?}"
        );
        assert_eq!(pixel(&px, 64, 40, 16), [0, 0, 0, 0], "clipped by the frame");
        assert_eq!(stats.layers, 1);
    }

    #[test]
    fn hidden_and_mask_nodes_are_not_drawn() {
        let fill = Paints::new(vec![Paint::solid(Color::WHITE)]);
        let mut hidden = Node::new(NodeKind::Rectangle, Symbol::EMPTY)
            .with_size(10.0, 10.0)
            .with_fills(fill.clone());
        hidden.flags.set(NodeFlags::VISIBLE, false);
        let mut mask = Node::new(NodeKind::Rectangle, Symbol::EMPTY)
            .with_size(10.0, 10.0)
            .with_fills(fill);
        mask.flags.set(NodeFlags::IS_MASK, true);
        let (doc, page) = page_with(vec![hidden, mask]);
        let (px, stats) = render_cpu(&doc, page, 16, 16);
        assert!(px.iter().all(|&b| b == 0));
        assert_eq!(stats.drawn, 0);
    }

    #[test]
    fn text_draws_glyphs_and_reports_missing_fonts() {
        let mut doc = Document::new();
        let family = doc.strings.intern("NoSuchFont KuArt Test");
        let style = ude_core::typography::TextStyle {
            family,
            weight: 400,
            italic: false,
            size: 24.0,
            line_height: ude_core::typography::LineHeight::Auto,
            letter_spacing: ude_core::typography::LetterSpacing::Normal,
            decoration: ude_core::typography::TextDecoration::None,
            case: ude_core::typography::TextCase::Original,
            fills: Paints::new(vec![Paint::solid(Color::BLACK)]),
        };
        let text = ude_core::typography::TextData::plain("Hello", style);
        let mut engine = TextEngine::default();
        let (w, h) = engine.measure(&text, &doc.strings, 1000.0);
        assert!(
            w > 20.0 && h > 10.0,
            "laid out with a fallback font: {w}x{h}"
        );
        assert_eq!(engine.missing_fonts(), vec!["NoSuchFont KuArt Test"]);
        assert_eq!(engine.missing_families(), vec!["NoSuchFont KuArt Test"]);
        assert_eq!(
            crate::text::face_name("Inter", 600, true),
            "Inter SemiBold Italic"
        );
        assert_eq!(crate::text::face_name("Inter", 400, false), "Inter");

        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let node = Node::new(NodeKind::Text(Box::new(text)), Symbol::EMPTY)
            .with_transform(ude_core::Affine::translate(4.0, 4.0))
            .with_size(w, h);
        doc.build_child(page, node).unwrap();
        let (px, stats) = render_cpu(&doc, page, 160, 48);
        let inked = px.chunks_exact(4).filter(|p| p[3] > 128).count();
        assert!(inked > 50, "glyph pixels drawn: {inked}");
        assert_eq!(stats.drawn, 1);
    }

    #[test]
    fn export_renders_layer_at_scale() {
        let fill = Paints::new(vec![Paint::solid(Color::from_rgba8(10, 20, 30, 128))]);
        let (doc, page) = page_with(vec![
            Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                .with_transform(ude_core::Affine::translate(100.0, 50.0))
                .with_size(20.0, 10.0)
                .with_fills(fill),
        ]);
        let rect = doc.children(page)[0];
        let (w, h, px) = render_to_rgba(&doc, rect, 2.0, &mut RenderCaches::default()).unwrap();
        assert_eq!(
            (w, h),
            (40, 20),
            "layer bounds at 2x, positioned at the origin"
        );
        let p = &px[(10 * 40 + 20) * 4..][..4];
        assert!(
            p[3].abs_diff(128) <= 1 && p[0].abs_diff(10) <= 2,
            "straight (not premultiplied) RGBA: {p:?}"
        );
        let png = export_png(&doc, page, 1.0, &mut RenderCaches::default()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn raster_layer_draws_decoded_pixels() {
        use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([0, 200, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut doc = Document::new();
        let asset = doc.assets.insert(AssetEntry {
            hash: ContentHash([9; 32]),
            kind: AssetKind::Image {
                width: 64,
                height: 32,
            },
            source: AssetSource::Memory {
                bytes: png.into(),
                codec: Codec::Png,
            },
        });
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        doc.build_child(
            page,
            Node::new(NodeKind::Raster { asset }, Symbol::EMPTY).with_size(32.0, 16.0),
        )
        .unwrap();
        let (px, _) = render_cpu(&doc, page, 40, 20);
        assert_eq!(
            pixel(&px, 40, 16, 8),
            [0, 200, 0, 255],
            "image pixels, not the placeholder"
        );
        assert_eq!(
            pixel(&px, 40, 36, 18),
            [0, 0, 0, 0],
            "outside the layer box"
        );
    }

    #[test]
    fn overlay_draws_outline_and_handles() {
        let quad = [
            Point::new(10.0, 10.0),
            Point::new(50.0, 10.0),
            Point::new(50.0, 40.0),
            Point::new(10.0, 40.0),
        ];
        let outline = Outline {
            quad,
            component: false,
        };
        let overlay = Overlay {
            selected: vec![outline],
            selection_box: Some(outline),
            scale: 1.0,
            ..Default::default()
        };
        let mut cx = vello_cpu::RenderContext::new(64, 64);
        let mut resources = vello_cpu::Resources::new();
        paint_overlay(
            &overlay,
            &mut CpuPainter {
                cx: &mut cx,
                resources: &mut resources,
            },
        );
        let mut pixmap = vello_cpu::Pixmap::new(64, 64);
        cx.flush();
        cx.render(&mut pixmap, &mut resources);
        let px = pixmap.data_as_u8_slice();
        // Handle at the top-left corner is white; the box interior stays empty.
        assert_eq!(pixel(px, 64, 10, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(px, 64, 30, 25), [0, 0, 0, 0]);
        // Edge between handles carries the accent colour.
        let edge = pixel(px, 64, 20, 10);
        assert!(
            edge[3] > 0 && edge[2] > edge[1],
            "violet outline, got {edge:?}"
        );
    }

    #[test]
    fn synthetic_document_renders_on_cpu() {
        let doc = ude_bench_like_doc();
        let page = doc.children(doc.root())[0];
        let (px, stats) = render_cpu(&doc, page, 320, 240);
        assert!(stats.drawn > 0);
        assert!(px.chunks_exact(4).any(|p| p[3] > 0));
    }

    /// Small mixed document covering every node kind the renderer handles.
    fn ude_bench_like_doc() -> Document {
        let fill = Paints::new(vec![Paint::solid(Color::from_rgba8(37, 99, 235, 255))]);
        let mut nodes = Vec::new();
        for (i, kind) in [
            NodeKind::Rectangle,
            NodeKind::Ellipse(None),
            NodeKind::Polygon { sides: 6 },
            NodeKind::Star {
                points: 5,
                inner_ratio: 0.5,
            },
            NodeKind::Line,
        ]
        .into_iter()
        .enumerate()
        {
            let mut n = Node::new(kind, Symbol::EMPTY)
                .with_transform(ude_core::Affine::translate(10.0 + 60.0 * i as f32, 10.0))
                .with_size(50.0, 50.0)
                .with_fills(fill.clone());
            n.corner_radius = 6.0;
            nodes.push(n);
        }
        page_with(nodes).0
    }

    #[test]
    fn compressed_exports_decode_and_shrink() {
        // Noisy pixels (like a photo): the palette pass is what makes it smaller.
        let (w, h) = (256u32, 128u32);
        let mut seed = 7u32;
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let [a, b, c, _] = seed.to_le_bytes();
                [a, b, c, 255]
            })
            .collect();
        let plain = encode_rgba(
            w,
            h,
            rgba.clone(),
            RasterFormat::Png,
            Compression::default(),
        )
        .unwrap();
        let small = encode_rgba(
            w,
            h,
            rgba.clone(),
            RasterFormat::Png,
            Compression {
                palette: true,
                quality: 80,
            },
        )
        .unwrap();
        assert!(
            small.len() < plain.len(),
            "{} vs {}",
            small.len(),
            plain.len()
        );
        let back = image::load_from_memory(&small).unwrap();
        assert_eq!((back.width(), back.height()), (w, h));
        // Smooth artwork: compression never makes the file larger.
        let smooth: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % w) as u8, (i / w) as u8, 0, 255])
            .collect();
        let a = encode_rgba(
            w,
            h,
            smooth.clone(),
            RasterFormat::Png,
            Compression::default(),
        )
        .unwrap();
        let b = encode_rgba(
            w,
            h,
            smooth,
            RasterFormat::Png,
            Compression {
                palette: true,
                quality: 80,
            },
        )
        .unwrap();
        assert!(b.len() <= a.len());
        let jpg = encode_rgba(
            w,
            h,
            rgba.clone(),
            RasterFormat::Jpeg,
            Compression {
                palette: false,
                quality: 40,
            },
        )
        .unwrap();
        assert_eq!(image::guess_format(&jpg).unwrap(), image::ImageFormat::Jpeg);
        let webp = encode_rgba(
            w,
            h,
            rgba,
            RasterFormat::WebP,
            Compression {
                palette: true,
                quality: 60,
            },
        )
        .unwrap();
        assert_eq!(
            image::guess_format(&webp).unwrap(),
            image::ImageFormat::WebP
        );
    }
}
