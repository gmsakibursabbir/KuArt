//! SVG export (docs/format-strategy.md §5): writes a page or layer as standalone SVG.
//!
//! Shapes, paths, solid and gradient fills, strokes, opacity, blend modes, drop shadows,
//! clipping frames, text and embedded images. Not exported: masks (skipped, as on canvas),
//! blurs other than drop shadow, and automatic text wrapping (explicit line breaks only).

use std::fmt::Write as _;

use ude_core::assets::{AssetSource, Codec};
use ude_core::geometry::PathVerb;
use ude_core::scene::NodeFlags;
use ude_core::styles::{
    BlendMode, Color, Effect, GradientKind, LineCap, LineJoin, Paint, PaintKind,
};
use ude_core::typography::TextData;
use ude_core::{Affine, Document, FillRule, Node, NodeId, NodeKind, Rect};

/// Text lines as (text, x offset, baseline y) in the text box; see [`export_with`].
pub type LineLayout<'a> = &'a mut dyn FnMut(&TextData, (f32, f32)) -> Vec<(String, f32, f32)>;

/// Exports with simple line handling (explicit line breaks only, estimated baselines).
/// Prefer [`export_with`] and the renderer's text layout for exact wrapping.
pub fn export(doc: &Document, id: NodeId) -> Result<String, String> {
    export_with(doc, id, &mut simple_lines)
}

/// Exports with `lines` providing each text box's laid-out lines, so wrapping and
/// alignment match the canvas exactly.
pub fn export_with(doc: &Document, id: NodeId, lines: LineLayout<'_>) -> Result<String, String> {
    let bounds = bounds(doc, id).ok_or("nothing to export (empty layer)")?;
    let mut w = Writer {
        doc,
        defs: String::new(),
        body: String::new(),
        next_id: 0,
        lines,
    };
    let is_page = doc
        .node(id)
        .is_some_and(|n| matches!(n.kind, NodeKind::Page | NodeKind::Document));
    if is_page {
        for &c in doc.children(id) {
            w.node(c);
        }
    } else {
        let parent = doc
            .node(id)
            .and_then(|n| n.parent)
            .map_or(Affine::IDENTITY, |p| doc.world_transform(p));
        let _ = write!(w.body, "<g transform=\"{}\">", matrix(&parent));
        w.node(id);
        w.body.push_str("</g>");
    }
    let (x, y, bw, bh) = (bounds.x0, bounds.y0, bounds.width(), bounds.height());
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">",
        num(bw),
        num(bh),
        num(x),
        num(y),
        num(bw),
        num(bh)
    );
    if !w.defs.is_empty() {
        let _ = write!(out, "<defs>{}</defs>", w.defs);
    }
    out.push_str(&w.body);
    out.push_str("</svg>\n");
    Ok(out)
}

/// World bounds of the layer's own box, or the union of its descendants for pages and groups.
fn bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let n = doc.node(id)?;
    if n.size.w > 0.0 || n.size.h > 0.0 {
        return doc.world_bounds(id);
    }
    doc.descendants(id)
        .skip(1)
        .filter_map(|d| doc.world_bounds(d))
        .filter(|r| !r.is_empty())
        .reduce(|a, b| a.union(&b))
}

fn simple_lines(t: &TextData, _box: (f32, f32)) -> Vec<(String, f32, f32)> {
    let Some(style) = t.styles.first() else {
        return Vec::new();
    };
    let line_h = match style.line_height {
        ude_core::typography::LineHeight::Px(px) => px,
        ude_core::typography::LineHeight::Percent(p) => style.size * p / 100.0,
        ude_core::typography::LineHeight::Auto => style.size * 1.2,
    };
    t.content
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            (
                line.to_owned(),
                0.0,
                (line_h - style.size) / 2.0 + style.size * 0.8 + i as f32 * line_h,
            )
        })
        .collect()
}

struct Writer<'a> {
    doc: &'a Document,
    defs: String,
    body: String,
    next_id: u32,
    lines: LineLayout<'a>,
}

impl Writer<'_> {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn node(&mut self, id: NodeId) {
        let Some(n) = self.doc.node(id) else { return };
        if !n.is_visible() || n.flags.contains(NodeFlags::IS_MASK) {
            return;
        }
        let mut attrs = format!(" transform=\"{}\"", matrix(&n.transform));
        if n.opacity < 1.0 {
            let _ = write!(attrs, " opacity=\"{}\"", num(n.opacity));
        }
        if let Some(mode) = blend_css(n.blend) {
            let _ = write!(attrs, " style=\"mix-blend-mode:{mode}\"");
        }
        if let Some(filter) = self.shadow_filter(n) {
            let _ = write!(attrs, " filter=\"url(#{filter})\"");
        }
        let _ = write!(self.body, "<g{attrs}>");
        self.own_content(n);
        if !n.children.is_empty() {
            if n.flags.contains(NodeFlags::CLIPS_CONTENT) {
                let clip = self.id("clip");
                let _ = write!(
                    self.defs,
                    "<clipPath id=\"{clip}\">{}</clipPath>",
                    shape(n, "")
                );
                let _ = write!(self.body, "<g clip-path=\"url(#{clip})\">");
                for &c in &n.children {
                    self.node(c);
                }
                self.body.push_str("</g>");
            } else {
                for &c in &n.children {
                    self.node(c);
                }
            }
        }
        if let Some(s) = n
            .ext
            .as_ref()
            .and_then(|e| e.strokes.as_deref())
            .filter(|s| s.width > 0.0)
        {
            for p in s.paints.as_slice().iter().filter(|p| p.visible) {
                let Some(paint) = self.paint(p) else { continue };
                let mut a = format!(
                    " fill=\"none\" stroke=\"{paint}\" stroke-width=\"{}\"",
                    num(s.width)
                );
                if p.opacity < 1.0 {
                    let _ = write!(a, " stroke-opacity=\"{}\"", num(p.opacity));
                }
                a.push_str(match s.cap {
                    LineCap::Butt => "",
                    LineCap::Round => " stroke-linecap=\"round\"",
                    LineCap::Square => " stroke-linecap=\"square\"",
                });
                a.push_str(match s.join {
                    LineJoin::Miter => "",
                    LineJoin::Round => " stroke-linejoin=\"round\"",
                    LineJoin::Bevel => " stroke-linejoin=\"bevel\"",
                });
                if !s.dashes.is_empty() {
                    let d: Vec<String> = s.dashes.iter().map(|v| num(*v)).collect();
                    let _ = write!(a, " stroke-dasharray=\"{}\"", d.join(" "));
                }
                self.body.push_str(&shape(n, &a));
            }
        }
        self.body.push_str("</g>");
    }

    fn own_content(&mut self, n: &Node) {
        match &n.kind {
            NodeKind::Text(t) => self.text(t, (n.size.w, n.size.h)),
            NodeKind::Raster { asset } => {
                if let Some(href) = data_uri(self.doc, *asset) {
                    let _ = write!(
                        self.body,
                        "<image href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"/>",
                        num(n.size.w),
                        num(n.size.h)
                    );
                }
            }
            NodeKind::Document
            | NodeKind::Page
            | NodeKind::Group
            | NodeKind::BooleanOp(_)
            | NodeKind::Slice => {}
            _ => {
                for p in n.fills.as_slice().iter().filter(|p| p.visible) {
                    if let PaintKind::Image { asset, .. } = &p.kind {
                        // Image fill: cover the shape, clipped to it.
                        let Some(href) = data_uri(self.doc, *asset) else {
                            continue;
                        };
                        let clip = self.id("clip");
                        let _ = write!(
                            self.defs,
                            "<clipPath id=\"{clip}\">{}</clipPath>",
                            shape(n, "")
                        );
                        let _ = write!(
                            self.body,
                            "<image href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"xMidYMid slice\" clip-path=\"url(#{clip})\"{}/>",
                            num(n.size.w),
                            num(n.size.h),
                            opacity_attr("opacity", p.opacity)
                        );
                        continue;
                    }
                    let Some(paint) = self.paint(p) else { continue };
                    let a = format!(
                        " fill=\"{paint}\"{}",
                        opacity_attr("fill-opacity", p.opacity)
                    );
                    self.body.push_str(&shape(n, &a));
                }
            }
        }
    }

    /// `fill`/`stroke` value for a paint; gradients are added to `<defs>`.
    fn paint(&mut self, p: &Paint) -> Option<String> {
        match &p.kind {
            PaintKind::Solid(c) => Some(color(*c)),
            PaintKind::Gradient(g) => {
                let id = self.id("grad");
                let stops: String = g
                    .stops
                    .iter()
                    .map(|s| {
                        format!(
                            "<stop offset=\"{}\" stop-color=\"{}\"{}/>",
                            num(s.offset),
                            color(s.color),
                            opacity_attr("stop-opacity", s.color.a)
                        )
                    })
                    .collect();
                let t = matrix(&g.transform);
                let _ = match g.kind {
                    GradientKind::Linear => write!(
                        self.defs,
                        "<linearGradient id=\"{id}\" gradientUnits=\"objectBoundingBox\" x1=\"0\" y1=\"0.5\" x2=\"1\" y2=\"0.5\" gradientTransform=\"{t}\">{stops}</linearGradient>"
                    ),
                    // Angular and diamond gradients have no SVG equivalent: approximated as radial.
                    _ => write!(
                        self.defs,
                        "<radialGradient id=\"{id}\" gradientUnits=\"objectBoundingBox\" cx=\"0.5\" cy=\"0.5\" r=\"0.5\" gradientTransform=\"{t}\">{stops}</radialGradient>"
                    ),
                };
                Some(format!("url(#{id})"))
            }
            PaintKind::Image { .. } => None,
        }
    }

    fn shadow_filter(&mut self, n: &Node) -> Option<String> {
        let ext = n.ext.as_ref()?;
        let shadows: Vec<_> = ext
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::DropShadow(s) if s.visible => Some(*s),
                _ => None,
            })
            .collect();
        if shadows.is_empty() {
            return None;
        }
        let id = self.id("shadow");
        let mut f =
            format!("<filter id=\"{id}\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\">");
        for s in shadows {
            let _ = write!(
                f,
                "<feDropShadow dx=\"{}\" dy=\"{}\" stdDeviation=\"{}\" flood-color=\"{}\" flood-opacity=\"{}\"/>",
                num(s.offset.x),
                num(s.offset.y),
                num(s.blur / 2.0),
                color(s.color),
                num(s.color.a)
            );
        }
        f.push_str("</filter>");
        self.defs.push_str(&f);
        Some(id)
    }

    fn text(&mut self, t: &TextData, size: (f32, f32)) {
        let Some(style) = t.styles.first() else {
            return;
        };
        let fill = style
            .fills
            .as_slice()
            .iter()
            .find_map(|p| match &p.kind {
                PaintKind::Solid(c) if p.visible => Some(*c),
                _ => None,
            })
            .unwrap_or(Color::BLACK);
        let family = self.doc.strings.resolve(style.family);
        let _ = write!(
            self.body,
            "<text font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\"{} fill=\"{}\"{}>",
            // Same generic fallback as the canvas when the named font is not installed.
            escape(&format!("{family}, sans-serif")),
            num(style.size),
            style.weight,
            if style.italic {
                " font-style=\"italic\""
            } else {
                ""
            },
            color(fill),
            opacity_attr("fill-opacity", fill.a)
        );
        for (line, x, y) in (self.lines)(t, size) {
            let _ = write!(
                self.body,
                "<tspan x=\"{}\" y=\"{}\" xml:space=\"preserve\">{}</tspan>",
                num(x),
                num(y),
                escape(&line)
            );
        }
        self.body.push_str("</text>");
    }
}

/// SVG element for the node's outline in local coordinates, with extra attributes.
fn shape(n: &Node, attrs: &str) -> String {
    let (w, h) = (n.size.w, n.size.h);
    match &n.kind {
        NodeKind::Ellipse(_) => format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{attrs}/>",
            num(w / 2.0),
            num(h / 2.0),
            num(w / 2.0),
            num(h / 2.0)
        ),
        NodeKind::Line => format!(
            "<line x1=\"0\" y1=\"0\" x2=\"{}\" y2=\"0\"{attrs}/>",
            num(w)
        ),
        NodeKind::Polygon { sides } => format!(
            "<polygon points=\"{}\"{attrs}/>",
            polygon(w, h, usize::from((*sides).max(3)), None)
        ),
        NodeKind::Star {
            points,
            inner_ratio,
        } => format!(
            "<polygon points=\"{}\"{attrs}/>",
            polygon(w, h, usize::from((*points).max(3)), Some(*inner_ratio))
        ),
        NodeKind::Vector(v) => {
            let mut d = String::new();
            let mut rule = FillRule::NonZero;
            for p in v.paths.iter() {
                d.push_str(&path_data(&p.path));
                rule = p.fill_rule;
            }
            let rule = if rule == FillRule::EvenOdd {
                " fill-rule=\"evenodd\""
            } else {
                ""
            };
            format!("<path d=\"{d}\"{rule}{attrs}/>")
        }
        _ => {
            let [tl, tr, br, bl] = n.corner_radii();
            if tl == tr && tr == br && br == bl {
                let rx = if tl > 0.0 {
                    format!(" rx=\"{}\"", num(tl))
                } else {
                    String::new()
                };
                format!(
                    "<rect width=\"{}\" height=\"{}\"{rx}{attrs}/>",
                    num(w),
                    num(h)
                )
            } else {
                format!(
                    "<path d=\"{}\"{attrs}/>",
                    rounded_rect(w, h, [tl, tr, br, bl])
                )
            }
        }
    }
}

fn rounded_rect(w: f32, h: f32, r: [f32; 4]) -> String {
    let m = (w.min(h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = r.map(|v| v.clamp(0.0, m));
    format!(
        "M{} 0H{}A{tr} {tr} 0 0 1 {} {}V{}A{br} {br} 0 0 1 {} {}H{}A{bl} {bl} 0 0 1 0 {}V{}A{tl} {tl} 0 0 1 {} 0Z",
        num(tl),
        num(w - tr),
        num(w),
        num(tr),
        num(h - br),
        num(w - br),
        num(h),
        num(bl),
        num(h - bl),
        num(tl),
        num(tl)
    )
}

fn polygon(w: f32, h: f32, n: usize, inner: Option<f32>) -> String {
    let count = if inner.is_some() { n * 2 } else { n };
    (0..count)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / count as f32;
            let r = if i % 2 == 1 {
                inner.unwrap_or(1.0)
            } else {
                1.0
            };
            format!(
                "{},{}",
                num(w / 2.0 + w / 2.0 * r * a.cos()),
                num(h / 2.0 + h / 2.0 * r * a.sin())
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn path_data(p: &ude_core::PathData) -> String {
    let pts = p.points();
    let mut d = String::new();
    let mut i = 0;
    for &v in p.verbs() {
        let pt = |k: usize| format!("{} {}", num(pts[i + k].x), num(pts[i + k].y));
        match v {
            PathVerb::MoveTo => d += &format!("M{}", pt(0)),
            PathVerb::LineTo => d += &format!("L{}", pt(0)),
            PathVerb::QuadTo => d += &format!("Q{} {}", pt(0), pt(1)),
            PathVerb::CubicTo => d += &format!("C{} {} {}", pt(0), pt(1), pt(2)),
            PathVerb::Close => d.push('Z'),
        }
        i += v.point_count();
    }
    d
}

fn data_uri(doc: &Document, asset: ude_core::AssetId) -> Option<String> {
    let entry = doc.assets.get(asset)?;
    let AssetSource::Memory { bytes, codec } = &entry.source else {
        return None;
    };
    let mime = match codec {
        Codec::Png => "image/png",
        Codec::Jpeg => "image/jpeg",
        Codec::WebP => "image/webp",
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", base64(bytes)))
}

/// Standard base64 (RFC 4648), avoiding a dependency for ~15 lines.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (k, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if k <= chunk.len() {
                T[(n >> shift & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn matrix(t: &Affine) -> String {
    let [a, b, c, d, e, f] = t.0;
    format!(
        "matrix({} {} {} {} {} {})",
        num(a),
        num(b),
        num(c),
        num(d),
        num(e),
        num(f)
    )
}

fn color(c: Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn opacity_attr(name: &str, v: f32) -> String {
    if v < 1.0 {
        format!(" {name}=\"{}\"", num(v))
    } else {
        String::new()
    }
}

/// Compact number: up to 3 decimals, no trailing zeros.
fn num(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_owned() }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn blend_css(b: BlendMode) -> Option<&'static str> {
    Some(match b {
        BlendMode::PassThrough | BlendMode::Normal => return None,
        BlendMode::Darken => "darken",
        BlendMode::Multiply => "multiply",
        BlendMode::ColorBurn | BlendMode::LinearBurn => "color-burn",
        BlendMode::Lighten => "lighten",
        BlendMode::Screen => "screen",
        BlendMode::ColorDodge | BlendMode::LinearDodge => "color-dodge",
        BlendMode::Overlay => "overlay",
        BlendMode::SoftLight => "soft-light",
        BlendMode::HardLight => "hard-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Paints, Symbol};

    #[test]
    fn exports_shapes_fills_and_text() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let mut frame = Node::new(NodeKind::Frame, Symbol::EMPTY)
            .with_size(200.0, 100.0)
            .with_fills(Paints::new(vec![Paint::solid(Color::from_rgba8(
                0x25, 0x63, 0xEB, 255,
            ))]));
        frame.corner_radius = 8.0;
        frame.flags.set(NodeFlags::CLIPS_CONTENT, true);
        let frame = doc.build_child(page, frame).unwrap();
        let family = doc.strings.intern("Inter");
        let style = ude_core::typography::TextStyle {
            family,
            weight: 600,
            italic: false,
            size: 16.0,
            line_height: ude_core::typography::LineHeight::Auto,
            letter_spacing: ude_core::typography::LetterSpacing::Normal,
            decoration: ude_core::typography::TextDecoration::None,
            case: ude_core::typography::TextCase::Original,
            fills: Paints::new(vec![Paint::solid(Color::WHITE)]),
        };
        let text = TextData::plain("A < B", style);
        doc.build_child(
            frame,
            Node::new(NodeKind::Text(Box::new(text)), Symbol::EMPTY).with_size(50.0, 20.0),
        )
        .unwrap();
        doc.build_child(
            frame,
            Node::new(NodeKind::Ellipse(None), Symbol::EMPTY)
                .with_size(10.0, 10.0)
                .with_fills(Paints::new(vec![Paint::solid(Color::from_rgba8(
                    255, 0, 0, 255,
                ))])),
        )
        .unwrap();

        let svg = export(&doc, page).unwrap();
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\" viewBox=\"0 0 200 100\">"));
        assert!(svg.contains("<rect width=\"200\" height=\"100\" rx=\"8\" fill=\"#2563eb\"/>"));
        assert!(
            svg.contains("clip-path=\"url(#clip"),
            "frame clips its children"
        );
        assert!(svg.contains(">A &lt; B</tspan>"), "text is escaped");
        assert!(
            svg.contains("font-family=\"Inter, sans-serif\""),
            "generic fallback"
        );
        assert!(svg.contains("font-weight=\"600\""));
        assert!(svg.contains("<ellipse cx=\"5\" cy=\"5\" rx=\"5\" ry=\"5\" fill=\"#ff0000\"/>"));
        assert!(svg.ends_with("</svg>\n"));
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
