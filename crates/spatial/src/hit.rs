//! Exact point-in-shape tests and paint-order comparison.

use std::cmp::Ordering;

use kurbo::{
    BezPath, Ellipse, Line, ParamCurveNearest, Point, Rect, RoundedRect, RoundedRectRadii, Shape,
};
use ude_core::geometry::PathVerb;
use ude_core::{Document, FillRule, NodeId, NodeKind};

/// Is the document point `(x, y)` inside `id`'s outline (within `tolerance` document units)?
pub fn contains_point(doc: &Document, id: NodeId, x: f64, y: f64, tolerance: f64) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let Some(inv) = doc.world_transform(id).inverse() else {
        return false;
    };
    let local = inv.apply(ude_core::Point::new(x as f32, y as f32));
    let p = Point::new(f64::from(local.x), f64::from(local.y));
    // Tolerance in local units (approximate under non-uniform scale).
    let scale = f64::from(doc.world_transform(id).determinant().abs().sqrt()).max(1e-6);
    let tol = tolerance / scale;
    let (w, h) = (f64::from(n.size.w), f64::from(n.size.h));
    let rect = Rect::new(0.0, 0.0, w, h);

    match &n.kind {
        NodeKind::Ellipse(_) => Ellipse::from_rect(rect.inflate(tol, tol)).contains(p),
        NodeKind::Line => {
            Line::new((0.0, 0.0), (w, 0.0)).nearest(p, 0.1).distance_sq
                <= (tol.max(2.0 / scale)).powi(2)
        }
        NodeKind::Vector(v) => v.paths.iter().any(|vp| {
            let path = to_bezpath(&vp.path);
            let winding = path.winding(p);
            let inside = match vp.fill_rule {
                FillRule::NonZero => winding != 0,
                FillRule::EvenOdd => winding % 2 != 0,
            };
            // Thin or open paths: also accept points near the outline.
            inside || near_path(&path, p, tol)
        }),
        NodeKind::Polygon { .. } | NodeKind::Star { .. } => rect.inflate(tol, tol).contains(p),
        _ => {
            let [tl, tr, br, bl] = n.corner_radii().map(f64::from);
            if tl == 0.0 && tr == 0.0 && br == 0.0 && bl == 0.0 {
                rect.inflate(tol, tol).contains(p)
            } else {
                RoundedRect::from_rect(
                    rect.inflate(tol, tol),
                    RoundedRectRadii::new(tl, tr, br, bl),
                )
                .contains(p)
            }
        }
    }
}

fn near_path(path: &BezPath, p: Point, tol: f64) -> bool {
    tol > 0.0
        && path
            .segments()
            .any(|seg| seg.nearest(p, 0.1).distance_sq <= tol * tol)
}

fn to_bezpath(p: &ude_core::PathData) -> BezPath {
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

/// Orders nodes by paint order: `Greater` means `a` is painted after (above) `b`.
/// Ancestors paint before their descendants; later siblings paint above earlier ones.
pub fn paint_order_cmp(doc: &Document, a: NodeId, b: NodeId) -> Ordering {
    path_from_root(doc, a).cmp(&path_from_root(doc, b))
}

/// Child indices from the root down to `id`; lexicographic order = pre-order = paint order.
fn path_from_root(doc: &Document, id: NodeId) -> Vec<u32> {
    let mut path = Vec::new();
    let mut cur = id;
    while let Some(parent) = doc.node(cur).and_then(|n| n.parent) {
        let index = doc
            .children(parent)
            .iter()
            .position(|&c| c == cur)
            .unwrap_or(0);
        path.push(index as u32);
        cur = parent;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Affine, Node, Symbol};

    #[test]
    fn shapes_and_rotation() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let ellipse = doc
            .build_child(
                page,
                Node::new(NodeKind::Ellipse(None), Symbol::EMPTY).with_size(100.0, 100.0),
            )
            .unwrap();
        assert!(contains_point(&doc, ellipse, 50.0, 50.0, 0.0));
        assert!(
            !contains_point(&doc, ellipse, 3.0, 3.0, 0.0),
            "corner of the box is outside the ellipse"
        );

        // 45° rotated square around the origin: (0, 50) is inside, (60, 10) is not.
        let rotated = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::rotate(std::f32::consts::FRAC_PI_4))
                    .with_size(100.0, 100.0),
            )
            .unwrap();
        assert!(contains_point(&doc, rotated, 0.0, 50.0, 0.0));
        assert!(!contains_point(&doc, rotated, 60.0, 10.0, 0.0));
    }

    #[test]
    fn paint_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let a = doc
            .build_child(page, Node::new(NodeKind::Frame, Symbol::EMPTY))
            .unwrap();
        let a_child = doc
            .build_child(a, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        let b = doc
            .build_child(page, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        assert_eq!(
            paint_order_cmp(&doc, a_child, a),
            Ordering::Greater,
            "child above parent"
        );
        assert_eq!(
            paint_order_cmp(&doc, b, a_child),
            Ordering::Greater,
            "later sibling above"
        );
    }
}
