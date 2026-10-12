//! SVG import (docs/format-strategy.md §5) through usvg, which resolves CSS, `<use>`, units
//! and transforms. Text is imported as its outlines (exact look, editable as shapes).
//! External references are never fetched: SVG files are untrusted input.

use std::sync::Arc;

use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::PathBuilder;
use ude_core::scene::{NodeFlags, VectorData, VectorPath};
use ude_core::styles::{
    BlendMode, Color, Gradient, GradientKind, GradientStop, LineCap, LineJoin, Paint, PaintKind,
    Stroke, StrokeAlign,
};
use ude_core::{Affine, Document, FillRule, Node, NodeId, NodeKind, Paints, SourceFormat, Symbol};
use usvg::tiny_skia_path::PathSegment;

/// A document plus what could not be imported exactly (shown to the user).
pub struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
}

pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    let mut opt = usvg::Options::default();
    // System fonts are needed to turn <text> into outlines.
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| e.to_string())?;

    let mut doc = Document::new();
    doc.meta.title = name.to_owned();
    doc.meta.source = Some(SourceFormat::Svg);
    let root = doc.root();
    let page_name = doc.strings.intern("Page 1");
    let page = doc
        .build_child(root, Node::new(NodeKind::Page, page_name))
        .map_err(|e| e.to_string())?;
    let size = tree.size();
    let frame_name = doc.strings.intern(name);
    let mut frame = Node::new(NodeKind::Frame, frame_name).with_size(size.width(), size.height());
    frame.flags.set(NodeFlags::CLIPS_CONTENT, true);
    let frame = doc.build_child(page, frame).map_err(|e| e.to_string())?;

    let mut im = Importer {
        doc,
        warnings: Vec::new(),
    };
    im.group_children(tree.root(), frame);
    let mut doc = im.doc;
    let _ = doc.take_changes();
    doc.shrink_to_fit();
    let mut warnings = im.warnings;
    warnings.sort();
    warnings.dedup();
    Ok(Imported { doc, warnings })
}

struct Importer {
    doc: Document,
    warnings: Vec<String>,
}

impl Importer {
    fn warn(&mut self, what: &str) {
        self.warnings
            .push(format!("{what}: not supported yet, imported without it"));
    }

    fn name(&mut self, id: &str, fallback: &str) -> Symbol {
        self.doc
            .strings
            .intern(if id.is_empty() { fallback } else { id })
    }

    fn add(&mut self, parent: NodeId, node: Node) -> Option<NodeId> {
        self.doc.build_child(parent, node).ok()
    }

    fn group_children(&mut self, g: &usvg::Group, parent: NodeId) {
        for child in g.children() {
            match child {
                usvg::Node::Group(g) => self.group(g, parent),
                usvg::Node::Path(p) => self.path(p, parent),
                usvg::Node::Image(i) => self.image(i, parent),
                usvg::Node::Text(t) => self.group(t.flattened(), parent),
            }
        }
    }

    fn group(&mut self, g: &usvg::Group, parent: NodeId) {
        if g.clip_path().is_some() {
            self.warn("Clip paths");
        }
        if g.mask().is_some() {
            self.warn("Masks");
        }
        if !g.filters().is_empty() {
            self.warn("Filters");
        }
        let name = self.name(g.id(), "Group");
        let mut node = Node::new(NodeKind::Group, name).with_transform(affine(g.transform()));
        node.opacity = g.opacity().get();
        node.blend = blend(g.blend_mode());
        if let Some(id) = self.add(parent, node) {
            self.group_children(g, id);
        }
    }

    fn path(&mut self, p: &usvg::Path, parent: NodeId) {
        if !p.is_visible() {
            return;
        }
        // Re-base the path on its bounding box so the node box matches what it draws.
        let bbox = p.data().bounds();
        let (ox, oy) = (bbox.left(), bbox.top());
        let mut b = PathBuilder::default();
        for seg in p.data().segments() {
            match seg {
                PathSegment::MoveTo(a) => b.move_to(a.x - ox, a.y - oy),
                PathSegment::LineTo(a) => b.line_to(a.x - ox, a.y - oy),
                PathSegment::QuadTo(c, a) => b.quad_to(c.x - ox, c.y - oy, a.x - ox, a.y - oy),
                PathSegment::CubicTo(c1, c2, a) => b.cubic_to(
                    c1.x - ox,
                    c1.y - oy,
                    c2.x - ox,
                    c2.y - oy,
                    a.x - ox,
                    a.y - oy,
                ),
                PathSegment::Close => b.close(),
            };
        }
        let (w, h) = (bbox.width().max(0.0), bbox.height().max(0.0));
        let origin = Affine::translate(ox, oy);
        let fill_rule = match p.fill().map(usvg::Fill::rule) {
            Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };
        let data = Arc::new(VectorData {
            paths: Box::new([VectorPath {
                path: b.build(),
                fill_rule,
            }]),
        });
        let name = self.name(p.id(), "Vector");
        let mut node = Node::new(NodeKind::Vector(data), name)
            .with_transform(origin)
            .with_size(w, h);
        if let Some(f) = p.fill()
            && let Some(paint) = self.paint(f.paint(), f.opacity().get(), origin, (w, h))
        {
            node.fills = Paints::new(vec![paint]);
        }
        if let Some(s) = p.stroke()
            && let Some(paint) = self.paint(s.paint(), s.opacity().get(), origin, (w, h))
        {
            node.ext_mut().strokes = Some(Box::new(Stroke {
                paints: Paints::new(vec![paint]),
                width: s.width().get(),
                align: StrokeAlign::Center,
                cap: match s.linecap() {
                    usvg::LineCap::Butt => LineCap::Butt,
                    usvg::LineCap::Round => LineCap::Round,
                    usvg::LineCap::Square => LineCap::Square,
                },
                join: match s.linejoin() {
                    usvg::LineJoin::Round => LineJoin::Round,
                    usvg::LineJoin::Bevel => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                },
                miter_limit: s.miterlimit().get(),
                dashes: s.dasharray().map(Box::from).unwrap_or_default(),
            }));
        }
        self.add(parent, node);
    }

    /// Converts an SVG paint (gradients are in user space) into our gradient space:
    /// the unit box mapped onto the node box. `origin` is the node's offset in user space.
    fn paint(
        &mut self,
        p: &usvg::Paint,
        opacity: f32,
        origin: Affine,
        size: (f32, f32),
    ) -> Option<Paint> {
        let kind = match p {
            usvg::Paint::Color(c) => {
                PaintKind::Solid(Color::from_rgba8(c.red, c.green, c.blue, 255))
            }
            usvg::Paint::LinearGradient(g) => {
                let (p1, p2) = ((g.x1(), g.y1()), (g.x2(), g.y2()));
                let (dx, dy) = (p2.0 - p1.0, p2.1 - p1.1);
                let len = dx.hypot(dy).max(1e-6);
                // Unit space: (0, 0.5) → p1 and (1, 0.5) → p2.
                let unit = Affine::translate(p1.0, p1.1)
                    * Affine::rotate(dy.atan2(dx))
                    * Affine::scale(len, len)
                    * Affine::translate(0.0, -0.5);
                PaintKind::Gradient(Arc::new(gradient(
                    GradientKind::Linear,
                    g.stops(),
                    to_box(origin, affine(g.transform()) * unit, size)?,
                )))
            }
            usvg::Paint::RadialGradient(g) => {
                let r = g.r().get();
                // Unit space: centre (0.5, 0.5), radius 0.5. The focal point is approximated by the centre.
                let unit = Affine::translate(g.cx(), g.cy())
                    * Affine::scale(2.0 * r, 2.0 * r)
                    * Affine::translate(-0.5, -0.5);
                PaintKind::Gradient(Arc::new(gradient(
                    GradientKind::Radial,
                    g.stops(),
                    to_box(origin, affine(g.transform()) * unit, size)?,
                )))
            }
            usvg::Paint::Pattern(_) => {
                self.warn("Pattern fills");
                return None;
            }
        };
        Some(Paint {
            kind,
            opacity,
            blend: BlendMode::Normal,
            visible: true,
        })
    }

    fn image(&mut self, i: &usvg::Image, parent: NodeId) {
        if !i.is_visible() {
            return;
        }
        let (bytes, codec) = match i.kind() {
            usvg::ImageKind::PNG(b) => (b, Codec::Png),
            usvg::ImageKind::JPEG(b) => (b, Codec::Jpeg),
            usvg::ImageKind::WEBP(b) => (b, Codec::WebP),
            usvg::ImageKind::GIF(_) => return self.warn("GIF images"),
            usvg::ImageKind::SVG(_) => return self.warn("Nested SVG images"),
        };
        let size = i.size();
        let asset = self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(bytes).as_bytes()),
            // Pixel size is read by the renderer's decoder; use the placed size as the hint.
            kind: AssetKind::Image {
                width: size.width().round().max(1.0) as u32,
                height: size.height().round().max(1.0) as u32,
            },
            source: AssetSource::Memory {
                bytes: bytes.as_slice().into(),
                codec,
            },
        });
        let name = self.name(i.id(), "Image");
        self.add(
            parent,
            Node::new(NodeKind::Raster { asset }, name).with_size(size.width(), size.height()),
        );
    }
}

/// Gradient transform in our convention: maps unit gradient space into the unit node box.
fn to_box(origin: Affine, user: Affine, size: (f32, f32)) -> Option<Affine> {
    if size.0 <= 0.0 || size.1 <= 0.0 {
        return None;
    }
    let inv_box = Affine::scale(1.0 / size.0, 1.0 / size.1);
    Some(inv_box * origin.inverse()? * user)
}

fn gradient(kind: GradientKind, stops: &[usvg::Stop], transform: Affine) -> Gradient {
    let stops = stops
        .iter()
        .map(|s| {
            let c = s.color();
            let a = (s.opacity().get() * 255.0).round() as u8;
            GradientStop {
                offset: s.offset().get(),
                color: Color::from_rgba8(c.red, c.green, c.blue, a),
            }
        })
        .collect();
    Gradient {
        kind,
        stops,
        transform,
    }
}

fn affine(t: usvg::Transform) -> Affine {
    Affine([t.sx, t.ky, t.kx, t.sy, t.tx, t.ty])
}

fn blend(b: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match b {
        B::Normal => BlendMode::PassThrough,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
      <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
      <g id="card" opacity="0.5">
        <rect id="bg" x="10" y="20" width="80" height="40" fill="url(#g)" stroke="#000" stroke-width="2"/>
        <circle cx="150" cy="50" r="20" fill="#00ff00"/>
      </g>
    </svg>"##;

    #[test]
    fn imports_structure_paths_and_paints() {
        let im = import(SVG.as_bytes(), "card.svg").unwrap();
        let doc = &im.doc;
        let page = doc.children(doc.root())[0];
        let frame = doc.children(page)[0];
        assert_eq!(
            doc.node(frame).unwrap().size,
            ude_core::Size::new(200.0, 100.0)
        );
        let group = doc.children(frame)[0];
        let g = doc.node(group).unwrap();
        assert_eq!(doc.strings.resolve(g.name), "card");
        assert!((g.opacity - 0.5).abs() < 1e-6);

        let rect = doc.node(doc.children(group)[0]).unwrap();
        assert_eq!(doc.strings.resolve(rect.name), "bg");
        assert_eq!(
            rect.transform.translation(),
            ude_core::Point::new(10.0, 20.0),
            "re-based on its bounds"
        );
        assert_eq!(rect.size, ude_core::Size::new(80.0, 40.0));
        let PaintKind::Gradient(grad) = &rect.fills.as_slice()[0].kind else {
            panic!("expected gradient")
        };
        // objectBoundingBox x1=0..x2=1 across the rect: gradient space maps straight onto the box.
        let start = grad.transform.apply(ude_core::Point::new(0.0, 0.5));
        let end = grad.transform.apply(ude_core::Point::new(1.0, 0.5));
        assert!(
            start.x.abs() < 1e-3 && (end.x - 1.0).abs() < 1e-3,
            "{start:?} {end:?}"
        );
        assert!(rect.ext.as_ref().unwrap().strokes.is_some());
        assert!(im.warnings.is_empty(), "{:?}", im.warnings);
    }

    #[test]
    fn reports_unsupported_and_rejects_garbage() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><clipPath id="c"><rect width="5" height="5"/></clipPath><g clip-path="url(#c)"><rect width="10" height="10"/></g></svg>"#;
        let im = import(svg.as_bytes(), "c.svg").unwrap();
        assert!(im.warnings.iter().any(|w| w.starts_with("Clip paths")));
        assert!(import(b"<not svg", "x").is_err());
    }
}
