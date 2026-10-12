//! Boolean groups (union, subtract, intersect, exclude), computed from the children's
//! outlines like Figma: the children stay editable and the result is recomputed from them.
//!
//! Rules (children in paint order, bottom first): union = all; subtract = bottom minus the
//! rest; intersect = area common to all; exclude = area covered an odd number of times.
//! Curves are flattened to polylines (0.25 px tolerance) before combining.

use i_overlay::core::fill_rule::FillRule as IFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use ude_core::geometry::PathBuilder;
use ude_core::scene::BoolOpKind;
use ude_core::{Affine, Document, FillRule, NodeId, NodeKind, PathData, PathVerb, Point};

/// Closed polylines.
pub type Contours = Vec<Vec<[f64; 2]>>;

const TOLERANCE: f64 = 0.25;
const MAX_DEPTH: usize = 32;

/// The boolean group's result in its own coordinates, or `None` for other nodes.
pub fn boolean_path(doc: &Document, id: NodeId) -> Option<PathData> {
    let NodeKind::BooleanOp(_) = doc.node(id)?.kind else {
        return None;
    };
    Some(to_path(&outline(doc, id, Affine::IDENTITY, 0)))
}

/// Outline of `id` in the space `to` maps its local coordinates into.
fn outline(doc: &Document, id: NodeId, to: Affine, depth: usize) -> Contours {
    let Some(n) = doc.node(id) else {
        return Vec::new();
    };
    if depth > MAX_DEPTH || !n.is_visible() {
        return Vec::new();
    }
    let (w, h) = (f64::from(n.size.w), f64::from(n.size.h));
    let raw: (Contours, FillRule) = match &n.kind {
        NodeKind::Rectangle
        | NodeKind::Frame
        | NodeKind::Component(_)
        | NodeKind::Instance(_)
        | NodeKind::Raster { .. } => (
            vec![rounded_rect(w, h, n.corner_radii())],
            FillRule::NonZero,
        ),
        NodeKind::Ellipse(_) => (vec![ellipse(w, h)], FillRule::NonZero),
        NodeKind::Polygon { sides } => (
            vec![polygon(w, h, usize::from((*sides).max(3)), None)],
            FillRule::NonZero,
        ),
        NodeKind::Star {
            points,
            inner_ratio,
        } => (
            vec![polygon(
                w,
                h,
                usize::from((*points).max(3)),
                Some(f64::from(*inner_ratio)),
            )],
            FillRule::NonZero,
        ),
        NodeKind::Vector(v) => {
            let rule = v.paths.first().map_or(FillRule::NonZero, |p| p.fill_rule);
            (
                v.paths.iter().flat_map(|p| flatten(&p.path)).collect(),
                rule,
            )
        }
        NodeKind::BooleanOp(kind) => {
            let children: Vec<Contours> = n
                .children
                .iter()
                .map(|&c| child_outline(doc, c, depth))
                .collect();
            (combine(*kind, &children), FillRule::NonZero)
        }
        NodeKind::Group => {
            let children: Vec<Contours> = n
                .children
                .iter()
                .map(|&c| child_outline(doc, c, depth))
                .collect();
            (combine(BoolOpKind::Union, &children), FillRule::NonZero)
        }
        _ => return Vec::new(),
    };
    let (contours, rule) = raw;
    let normalized = normalize(&contours, rule);
    normalized
        .into_iter()
        .map(|c| c.into_iter().map(|p| apply(to, p)).collect())
        .collect()
}

/// A child's outline in its parent's space.
fn child_outline(doc: &Document, id: NodeId, depth: usize) -> Contours {
    let t = doc.node(id).map_or(Affine::IDENTITY, |n| n.transform);
    outline(doc, id, t, depth + 1)
}

fn apply(t: Affine, p: [f64; 2]) -> [f64; 2] {
    let q = t.apply(Point::new(p[0] as f32, p[1] as f32));
    [f64::from(q.x), f64::from(q.y)]
}

/// Removes self-overlaps using the shape's own fill rule, so all inputs combine as non-zero.
fn normalize(contours: &Contours, rule: FillRule) -> Contours {
    if contours.is_empty() {
        return Vec::new();
    }
    let fill = if rule == FillRule::EvenOdd {
        IFill::EvenOdd
    } else {
        IFill::NonZero
    };
    let empty: Contours = Vec::new();
    flatten_shapes(contours.overlay(&empty, OverlayRule::Subject, fill))
}

fn flatten_shapes(shapes: Vec<Vec<Vec<[f64; 2]>>>) -> Contours {
    shapes
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .collect()
}

/// Combines child outlines (bottom first) with Figma's boolean rules.
pub fn combine(kind: BoolOpKind, children: &[Contours]) -> Contours {
    let mut iter = children.iter().filter(|c| !c.is_empty());
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let rest: Vec<&Contours> = iter.collect();
    let mut acc = first.clone();
    match kind {
        BoolOpKind::Union => {
            for c in rest {
                acc = flatten_shapes(acc.overlay(c, OverlayRule::Union, IFill::NonZero));
            }
        }
        BoolOpKind::Subtract => {
            // Bottom shape minus the union of everything above it.
            let mut cut: Contours = Vec::new();
            for c in rest {
                cut = flatten_shapes(cut.overlay(c, OverlayRule::Union, IFill::NonZero));
            }
            if !cut.is_empty() {
                acc = flatten_shapes(acc.overlay(&cut, OverlayRule::Difference, IFill::NonZero));
            }
        }
        BoolOpKind::Intersect => {
            for c in rest {
                acc = flatten_shapes(acc.overlay(c, OverlayRule::Intersect, IFill::NonZero));
            }
        }
        BoolOpKind::Exclude => {
            for c in rest {
                acc = flatten_shapes(acc.overlay(c, OverlayRule::Xor, IFill::NonZero));
            }
        }
    }
    acc
}

/// Contours → closed path.
pub fn to_path(contours: &Contours) -> PathData {
    let mut b = PathBuilder::default();
    for c in contours {
        let Some(first) = c.first() else { continue };
        b.move_to(first[0] as f32, first[1] as f32);
        for p in &c[1..] {
            b.line_to(p[0] as f32, p[1] as f32);
        }
        b.close();
    }
    b.build()
}

/// Path → closed polylines (curves subdivided to the tolerance).
pub fn flatten(path: &PathData) -> Contours {
    let mut out: Contours = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    let mut pts = path
        .points()
        .iter()
        .map(|p| [f64::from(p.x), f64::from(p.y)]);
    let mut last = [0.0, 0.0];
    let segments = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let len = (b[0] - a[0]).hypot(b[1] - a[1]) + (c[0] - b[0]).hypot(c[1] - b[1]);
        ((len / TOLERANCE).sqrt().ceil() as usize).clamp(2, 64)
    };
    for v in path.verbs() {
        match v {
            PathVerb::MoveTo => {
                if cur.len() >= 3 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                last = pts.next().unwrap_or(last);
                cur.push(last);
            }
            PathVerb::LineTo => {
                last = pts.next().unwrap_or(last);
                cur.push(last);
            }
            PathVerb::QuadTo => {
                let (c, e) = (pts.next().unwrap_or(last), pts.next().unwrap_or(last));
                let n = segments(last, c, e);
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let u = 1.0 - t;
                    cur.push([
                        u * u * last[0] + 2.0 * u * t * c[0] + t * t * e[0],
                        u * u * last[1] + 2.0 * u * t * c[1] + t * t * e[1],
                    ]);
                }
                last = e;
            }
            PathVerb::CubicTo => {
                let (c1, c2, e) = (
                    pts.next().unwrap_or(last),
                    pts.next().unwrap_or(last),
                    pts.next().unwrap_or(last),
                );
                let n = segments(last, c1, e).max(segments(c1, c2, e));
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push([
                        a * last[0] + b * c1[0] + c * c2[0] + d * e[0],
                        a * last[1] + b * c1[1] + c * c2[1] + d * e[1],
                    ]);
                }
                last = e;
            }
            PathVerb::Close => {
                if cur.len() >= 3 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 3 {
        out.push(cur);
    }
    out
}

/// Rounded rectangle (radii: top-left, top-right, bottom-right, bottom-left), arcs flattened.
fn rounded_rect(w: f64, h: f64, radii: [f32; 4]) -> Vec<[f64; 2]> {
    let max = (w.min(h) / 2.0).max(0.0);
    let r = radii.map(|v| f64::from(v).clamp(0.0, max));
    let mut out = Vec::new();
    let mut arc = |cx: f64, cy: f64, r: f64, start: f64| {
        if r <= 0.0 {
            out.push([cx, cy]);
            return;
        }
        let n = ((r / TOLERANCE).sqrt().ceil() as usize).clamp(2, 32);
        for i in 0..=n {
            let a = start + std::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            out.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    };
    use std::f64::consts::PI;
    arc(r[0], r[0], r[0], PI);
    arc(w - r[1], r[1], r[1], 1.5 * PI);
    arc(w - r[2], h - r[2], r[2], 0.0);
    arc(r[3], h - r[3], r[3], 0.5 * PI);
    out
}

fn ellipse(w: f64, h: f64) -> Vec<[f64; 2]> {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let n = ((rx.max(ry) / TOLERANCE).sqrt().ceil() as usize * 4).clamp(16, 256);
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [rx + rx * a.cos(), ry + ry * a.sin()]
        })
        .collect()
}

/// Same construction as the renderer: first point at the top.
fn polygon(w: f64, h: f64, n: usize, inner: Option<f64>) -> Vec<[f64; 2]> {
    let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
    let count = if inner.is_some() { n * 2 } else { n };
    (0..count)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / count as f64;
            let r = if i % 2 == 1 {
                inner.unwrap_or(1.0)
            } else {
                1.0
            };
            [cx + rx * r * a.cos(), cy + ry * r * a.sin()]
        })
        .collect()
}

/// Area enclosed by contours (non-zero), for tests and diagnostics.
pub fn area(contours: &Contours) -> f64 {
    contours
        .iter()
        .map(|c| {
            let mut a = 0.0;
            for i in 0..c.len() {
                let (p, q) = (c[i], c[(i + 1) % c.len()]);
                a += p[0] * q[1] - q[0] * p[1];
            }
            a / 2.0
        })
        .sum::<f64>()
        .abs()
}

/// The boolean group as a plain vector layer with the same fills, strokes and transform and
/// no children: what exporters draw, and what Flatten replaces it with.
pub fn as_vector(doc: &Document, id: NodeId) -> Option<ude_core::Node> {
    let path = boolean_path(doc, id)?;
    let mut n = doc.node(id)?.clone();
    n.children = Default::default();
    n.kind = NodeKind::Vector(std::sync::Arc::new(ude_core::scene::VectorData {
        paths: vec![ude_core::scene::VectorPath {
            path,
            fill_rule: FillRule::NonZero,
        }]
        .into(),
    }));
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Node, Symbol};

    fn square(x: f64, y: f64, s: f64) -> Contours {
        vec![vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]]
    }

    #[test]
    fn figma_rules_on_two_squares() {
        // Two 10×10 squares overlapping by 5×10.
        let (a, b) = (square(0.0, 0.0, 10.0), square(5.0, 0.0, 10.0));
        let near = |x: f64, y: f64| (x - y).abs() < 1e-6;
        assert!(near(
            area(&combine(BoolOpKind::Union, &[a.clone(), b.clone()])),
            150.0
        ));
        assert!(near(
            area(&combine(BoolOpKind::Subtract, &[a.clone(), b.clone()])),
            50.0
        ));
        assert!(near(
            area(&combine(BoolOpKind::Intersect, &[a.clone(), b.clone()])),
            50.0
        ));
        assert!(near(area(&combine(BoolOpKind::Exclude, &[a, b])), 100.0));
    }

    #[test]
    fn boolean_group_of_document_shapes() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let group = doc
            .build_child(
                page,
                Node::new(NodeKind::BooleanOp(BoolOpKind::Subtract), Symbol::EMPTY),
            )
            .unwrap();
        doc.build_child(
            group,
            Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(100.0, 100.0),
        )
        .unwrap();
        doc.build_child(
            group,
            Node::new(NodeKind::Ellipse(None), Symbol::EMPTY)
                .with_transform(Affine::translate(25.0, 25.0))
                .with_size(50.0, 50.0),
        )
        .unwrap();
        let path = boolean_path(&doc, group).unwrap();
        let a = area(&flatten(&path));
        let expected = 100.0 * 100.0 - std::f64::consts::PI * 25.0 * 25.0;
        assert!((a - expected).abs() < expected * 0.01, "{a} vs {expected}");
        // The hole is a separate contour.
        assert_eq!(flatten(&path).len(), 2);
    }
}
