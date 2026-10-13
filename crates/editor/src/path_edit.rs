//! Editable vector paths for the pen tool and point editing. Anchors are kept in document
//! coordinates while editing; [`commit`] writes them back into the layer's own space.

use ude_core::geometry::PathBuilder;
use ude_core::scene::{VectorData, VectorPath};
use ude_core::{
    DocError, Document, FillRule, History, NodeId, NodeKind, Op, PathData, PathVerb, Prop, Size,
    Transaction,
};
use ude_spatial::kurbo::{Affine, BezPath, CubicBez, ParamCurve, ParamCurveNearest, Point, Rect};
use ude_spatial::{from_kurbo, to_kurbo};

use crate::create::{Tool, default_node};

/// An anchor with its two handles; a handle equal to the anchor means "no handle" (sharp).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub p: Point,
    pub h_in: Point,
    pub h_out: Point,
}

impl Anchor {
    pub fn corner(p: Point) -> Self {
        Self {
            p,
            h_in: p,
            h_out: p,
        }
    }

    pub fn is_sharp(&self) -> bool {
        self.h_in == self.p && self.h_out == self.p
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubPath {
    pub anchors: Vec<Anchor>,
    pub closed: bool,
}

/// Which handle of an anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    In,
    Out,
}

/// (subpath, anchor) index.
pub type AnchorRef = (usize, usize);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditPath {
    pub subs: Vec<SubPath>,
}

impl EditPath {
    /// A vector layer's paths, in document coordinates.
    pub fn of_node(doc: &Document, id: NodeId) -> Option<Self> {
        let NodeKind::Vector(v) = &doc.node(id)?.kind else {
            return None;
        };
        let to_world = to_kurbo(&doc.world_transform(id));
        let mut out = Self::default();
        for vp in v.paths.iter() {
            out.read(&vp.path, to_world);
        }
        Some(out)
    }

    fn read(&mut self, path: &PathData, t: Affine) {
        let mut pts = path
            .points()
            .iter()
            .map(|p| t * Point::new(f64::from(p.x), f64::from(p.y)));
        let mut sub = SubPath::default();
        let flush = |sub: &mut SubPath, subs: &mut Vec<SubPath>| {
            let s = std::mem::take(sub);
            if !s.anchors.is_empty() {
                subs.push(s);
            }
        };
        for v in path.verbs() {
            match v {
                PathVerb::MoveTo => {
                    flush(&mut sub, &mut self.subs);
                    if let Some(p) = pts.next() {
                        sub.anchors.push(Anchor::corner(p));
                    }
                }
                PathVerb::LineTo => {
                    if let Some(p) = pts.next() {
                        sub.anchors.push(Anchor::corner(p));
                    }
                }
                PathVerb::QuadTo | PathVerb::CubicTo => {
                    let (c1, c2, e) = if *v == PathVerb::QuadTo {
                        let (Some(c), Some(e)) = (pts.next(), pts.next()) else {
                            break;
                        };
                        let s = sub.anchors.last().map_or(c, |a| a.p);
                        (s.lerp(c, 2.0 / 3.0), e.lerp(c, 2.0 / 3.0), e)
                    } else {
                        let (Some(c1), Some(c2), Some(e)) = (pts.next(), pts.next(), pts.next())
                        else {
                            break;
                        };
                        (c1, c2, e)
                    };
                    if let Some(last) = sub.anchors.last_mut() {
                        last.h_out = c1;
                    }
                    sub.anchors.push(Anchor {
                        p: e,
                        h_in: c2,
                        h_out: e,
                    });
                }
                PathVerb::Close => {
                    sub.closed = true;
                    // A closing segment that ends on the start point merges into the first anchor.
                    if sub.anchors.len() > 1
                        && let (Some(first), Some(last)) =
                            (sub.anchors.first().copied(), sub.anchors.last().copied())
                        && first.p.distance(last.p) < 1e-3
                    {
                        sub.anchors.pop();
                        sub.anchors[0].h_in = last.h_in;
                    }
                    flush(&mut sub, &mut self.subs);
                }
            }
        }
        flush(&mut sub, &mut self.subs);
    }

    fn segments(&self, sub: &SubPath) -> impl Iterator<Item = (usize, CubicBez)> {
        let n = sub.anchors.len();
        let count = if sub.closed { n } else { n.saturating_sub(1) };
        let a = sub.anchors.clone();
        (0..count).map(move |i| {
            let (s, e) = (a[i], a[(i + 1) % n]);
            (i, CubicBez::new(s.p, s.h_out, e.h_in, e.p))
        })
    }

    /// The path through `t` (e.g. document → device), for drawing.
    pub fn bez(&self, t: Affine) -> BezPath {
        let mut b = BezPath::new();
        for sub in &self.subs {
            let Some(first) = sub.anchors.first() else {
                continue;
            };
            b.move_to(t * first.p);
            for (_, c) in self.segments(sub) {
                b.curve_to(t * c.p1, t * c.p2, t * c.p3);
            }
            if sub.closed {
                b.close_path();
            }
        }
        b
    }

    /// The path through `t` as document path data (straight segments stay lines).
    pub fn to_path(&self, t: Affine) -> PathData {
        let mut b = PathBuilder::default();
        let f = |p: Point| {
            let q = t * p;
            (q.x as f32, q.y as f32)
        };
        for sub in &self.subs {
            let Some(first) = sub.anchors.first() else {
                continue;
            };
            let (x, y) = f(first.p);
            b.move_to(x, y);
            for (_, c) in self.segments(sub) {
                let (x, y) = f(c.p3);
                if c.p1 == c.p0 && c.p2 == c.p3 {
                    b.line_to(x, y);
                } else {
                    let ((x1, y1), (x2, y2)) = (f(c.p1), f(c.p2));
                    b.cubic_to(x1, y1, x2, y2, x, y);
                }
            }
            if sub.closed {
                b.close();
            }
        }
        b.build()
    }

    pub fn anchor_count(&self) -> usize {
        self.subs.iter().map(|s| s.anchors.len()).sum()
    }

    pub fn anchor(&self, r: AnchorRef) -> Option<&Anchor> {
        self.subs.get(r.0)?.anchors.get(r.1)
    }

    pub fn anchor_mut(&mut self, r: AnchorRef) -> Option<&mut Anchor> {
        self.subs.get_mut(r.0)?.anchors.get_mut(r.1)
    }

    /// Nearest anchor within `tol`.
    pub fn hit_anchor(&self, p: Point, tol: f64) -> Option<AnchorRef> {
        self.refs()
            .filter_map(|r| Some((r, self.anchor(r)?.p.distance(p))))
            .filter(|&(_, d)| d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(r, _)| r)
    }

    /// Nearest visible handle within `tol`.
    pub fn hit_handle(&self, p: Point, tol: f64) -> Option<(AnchorRef, Side)> {
        self.refs()
            .flat_map(|r| [(r, Side::In), (r, Side::Out)])
            .filter_map(|(r, s)| {
                let a = self.anchor(r)?;
                let h = if s == Side::In { a.h_in } else { a.h_out };
                (h != a.p).then(|| ((r, s), h.distance(p)))
            })
            .filter(|&(_, d)| d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(h, _)| h)
    }

    /// Segment under `p` (within `tol`): subpath, segment index (from its start anchor) and t.
    pub fn hit_segment(&self, p: Point, tol: f64) -> Option<(usize, usize, f64)> {
        self.subs
            .iter()
            .enumerate()
            .flat_map(|(si, sub)| {
                self.segments(sub).map(move |(i, c)| {
                    let n = c.nearest(p, 1e-6);
                    (si, i, n.t, n.distance_sq.sqrt())
                })
            })
            .filter(|s| s.3 <= tol)
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(s, i, t, _)| (s, i, t))
    }

    fn refs(&self) -> impl Iterator<Item = AnchorRef> + '_ {
        self.subs
            .iter()
            .enumerate()
            .flat_map(|(s, sub)| (0..sub.anchors.len()).map(move |i| (s, i)))
    }

    /// Splits segment `seg` of subpath `s` at `t` without changing its shape; returns the new anchor.
    pub fn insert(&mut self, s: usize, seg: usize, t: f64) -> AnchorRef {
        let sub = &mut self.subs[s];
        let n = sub.anchors.len();
        let (a, b) = (sub.anchors[seg], sub.anchors[(seg + 1) % n]);
        let line = a.h_out == a.p && b.h_in == b.p;
        let c = CubicBez::new(a.p, a.h_out, b.h_in, b.p);
        let (l, r) = (c.subsegment(0.0..t), c.subsegment(t..1.0));
        let new = if line {
            Anchor::corner(l.p3)
        } else {
            sub.anchors[seg].h_out = l.p1;
            sub.anchors[(seg + 1) % n].h_in = r.p2;
            Anchor {
                p: l.p3,
                h_in: l.p2,
                h_out: r.p1,
            }
        };
        sub.anchors.insert(seg + 1, new);
        (s, seg + 1)
    }

    /// Removes an anchor; a subpath left with fewer than two anchors is removed too.
    pub fn remove(&mut self, r: AnchorRef) {
        let Some(sub) = self.subs.get_mut(r.0) else {
            return;
        };
        if r.1 < sub.anchors.len() {
            sub.anchors.remove(r.1);
        }
        if sub.anchors.len() < 2 {
            self.subs.remove(r.0);
        }
    }

    /// Sharp ↔ smooth. Smooth handles follow the neighbours' direction, a third of the way.
    pub fn toggle_smooth(&mut self, r: AnchorRef) {
        let Some(sub) = self.subs.get(r.0) else {
            return;
        };
        let n = sub.anchors.len();
        let Some(&a) = sub.anchors.get(r.1) else {
            return;
        };
        if !a.is_sharp() {
            if let Some(a) = self.anchor_mut(r) {
                *a = Anchor::corner(a.p);
            }
            return;
        }
        let prev = (r.1 > 0 || sub.closed).then(|| sub.anchors[(r.1 + n - 1) % n].p);
        let next = (r.1 + 1 < n || sub.closed).then(|| sub.anchors[(r.1 + 1) % n].p);
        let (pa, na) = (prev.unwrap_or(a.p), next.unwrap_or(a.p));
        let dir = na - pa;
        let len = dir.hypot();
        if len == 0.0 {
            return;
        }
        let u = dir / len;
        let (li, lo) = (pa.distance(a.p) / 3.0, na.distance(a.p) / 3.0);
        if let Some(a) = self.anchor_mut(r) {
            a.h_in = a.p - u * li;
            a.h_out = a.p + u * lo;
        }
    }

    /// Moves a handle; unless `independent`, the opposite handle keeps its length and mirrors
    /// the angle (as in mainstream editors).
    pub fn set_handle(&mut self, r: AnchorRef, side: Side, to: Point, independent: bool) {
        let Some(a) = self.anchor_mut(r) else { return };
        let (h, other) = match side {
            Side::In => (&mut a.h_in, a.h_out),
            Side::Out => (&mut a.h_out, a.h_in),
        };
        *h = to;
        if independent {
            return;
        }
        let d = to - a.p;
        let len = if other == a.p {
            d.hypot()
        } else {
            other.distance(a.p)
        };
        let mirrored = if d.hypot() > 0.0 {
            a.p - d / d.hypot() * len
        } else {
            a.p
        };
        match side {
            Side::In => a.h_out = mirrored,
            Side::Out => a.h_in = mirrored,
        }
    }

    pub fn move_anchor(&mut self, r: AnchorRef, to: Point) {
        if let Some(a) = self.anchor_mut(r) {
            let d = to - a.p;
            a.p = to;
            a.h_in += d;
            a.h_out += d;
        }
    }
}

/// A smooth path through a freehand stroke: points closer than `tolerance` to the simplified
/// line are dropped (Ramer–Douglas–Peucker), and the rest get Catmull–Rom handles.
pub fn freehand(points: &[Point], tolerance: f64) -> SubPath {
    fn simplify(p: &[Point], tol: f64, out: &mut Vec<Point>) {
        let (Some(&a), Some(&b)) = (p.first(), p.last()) else {
            return;
        };
        let ab = b - a;
        let len = ab.hypot();
        let far = p
            .iter()
            .enumerate()
            .skip(1)
            .take(p.len().saturating_sub(2))
            .map(|(i, &q)| {
                let d = if len > 0.0 {
                    (ab.cross(q - a) / len).abs()
                } else {
                    q.distance(a)
                };
                (i, d)
            })
            .max_by(|x, y| x.1.total_cmp(&y.1));
        match far {
            Some((i, d)) if d > tol => {
                simplify(&p[..=i], tol, out);
                out.pop();
                simplify(&p[i..], tol, out);
            }
            _ => out.extend([a, b]),
        }
    }
    let mut kept = Vec::new();
    simplify(points, tolerance, &mut kept);
    kept.dedup_by(|a, b| a.distance(*b) < 1e-6);
    let n = kept.len();
    let anchors = (0..n)
        .map(|i| {
            let p = kept[i];
            if i == 0 || i + 1 == n {
                return Anchor::corner(p);
            }
            // Catmull–Rom tangent: a sixth of the way between the neighbours.
            let d = (kept[i + 1] - kept[i - 1]) / 6.0;
            Anchor {
                p,
                h_in: p - d,
                h_out: p + d,
            }
        })
        .collect();
    SubPath {
        anchors,
        closed: false,
    }
}

/// Writes `path` (document coordinates) into the vector layer `id`, or creates one inside
/// `parent` when `id` is `None` (pen default: 1 px black stroke, no fill). The layer keeps its
/// rotation and scale; its box is refitted to the path. One undo step per `merge_key`.
pub fn commit(
    doc: &mut Document,
    history: &mut History,
    id: &mut Option<NodeId>,
    parent: NodeId,
    path: &EditPath,
    merge_key: u64,
) -> Result<(), DocError> {
    let mut tx = Transaction::new("Edit path").with_merge_key(merge_key);
    let target = match *id {
        Some(t) if doc.contains(t) => t,
        Some(_) => return Ok(()),
        None => {
            let pw = to_kurbo(&doc.world_transform(parent));
            let mut node = default_node(doc, Tool::Pen);
            let origin = path
                .subs
                .first()
                .and_then(|s| s.anchors.first())
                .map_or(Point::ZERO, |a| a.p);
            node.transform = from_kurbo(pw.inverse() * Affine::translate(origin.to_vec2()));
            let new = doc.create(node);
            let index = doc.children(parent).len() as u32;
            tx.apply(
                doc,
                Op::Attach {
                    id: new,
                    parent,
                    index,
                },
            )?;
            *id = Some(new);
            new
        }
    };
    let (Some(n), parent) = (doc.node(target), doc.node(target).and_then(|n| n.parent)) else {
        return Ok(());
    };
    let local_t = to_kurbo(&n.transform);
    let parent_world = parent.map_or(Affine::IDENTITY, |p| to_kurbo(&doc.world_transform(p)));
    let to_local = (parent_world * local_t).inverse();
    let local = path.bez(to_local);
    let b = if local.elements().is_empty() {
        Rect::ZERO
    } else {
        ude_spatial::kurbo::Shape::bounding_box(&local)
    };
    let shift = Affine::translate(-b.origin().to_vec2());
    let data = path.to_path(shift * to_local);
    let fill_rule = match &n.kind {
        NodeKind::Vector(v) => v.paths.first().map_or(FillRule::NonZero, |p| p.fill_rule),
        _ => FillRule::NonZero,
    };
    let ops = [
        Prop::Kind(NodeKind::Vector(std::sync::Arc::new(VectorData {
            paths: vec![VectorPath {
                path: data,
                fill_rule,
            }]
            .into(),
        }))),
        Prop::Transform(from_kurbo(
            local_t * Affine::translate(b.origin().to_vec2()),
        )),
        Prop::Size(Size::new(b.width() as f32, b.height() as f32)),
    ];
    for value in ops {
        tx.apply(doc, Op::Set { id: target, value })?;
    }
    history.commit(doc, tx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Node, Symbol};

    #[test]
    fn pen_path_round_trips_and_edits() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let mut h = History::default();
        let mut p = EditPath {
            subs: vec![SubPath {
                anchors: vec![
                    Anchor::corner(Point::new(10.0, 10.0)),
                    Anchor::corner(Point::new(110.0, 10.0)),
                    Anchor::corner(Point::new(110.0, 60.0)),
                ],
                closed: true,
            }],
        };
        let mut id = None;
        commit(&mut doc, &mut h, &mut id, page, &p, 1).unwrap();
        let id = id.unwrap();
        let n = doc.node(id).unwrap();
        assert_eq!((n.size.w, n.size.h), (100.0, 50.0));
        assert_eq!(
            EditPath::of_node(&doc, id).unwrap(),
            p,
            "read back unchanged"
        );

        // Split the top edge, drag the new point up, smooth it.
        let (s, seg, t) = p.hit_segment(Point::new(60.0, 11.0), 2.0).unwrap();
        assert_eq!((s, seg), (0, 0));
        let r = p.insert(s, seg, t);
        p.move_anchor(r, Point::new(60.0, -20.0));
        p.toggle_smooth(r);
        assert!(!p.anchor(r).unwrap().is_sharp());
        let mut some = Some(id);
        commit(&mut doc, &mut h, &mut some, page, &p, 1).unwrap();
        let n = doc.node(id).unwrap();
        assert!(n.size.h > 70.0, "box grew upwards: {}", n.size.h);
        assert_eq!(doc.world_transform(id).translation().y, -20.0);
        let back = EditPath::of_node(&doc, id).unwrap();
        assert_eq!(back.anchor_count(), 4);
        assert!(back.anchor(r).unwrap().p.distance(Point::new(60.0, -20.0)) < 1e-3);

        // One undo step removes the whole session.
        h.undo(&mut doc).unwrap();
        assert!(doc.children(page).is_empty());
    }

    #[test]
    fn freehand_keeps_corners_and_drops_jitter() {
        // A wobbly line to (100, 0), then up to (100, 100).
        let mut pts: Vec<Point> = (0..=100)
            .map(|x| Point::new(f64::from(x), if x % 2 == 0 { 0.2 } else { -0.2 }))
            .collect();
        pts.extend((1..=100).map(|y| Point::new(100.0, f64::from(y))));
        let sub = freehand(&pts, 1.0);
        assert_eq!(sub.anchors.len(), 3, "start, corner, end");
        assert!(sub.anchors[1].p.distance(Point::new(100.0, 0.0)) < 1.0);
    }
}
