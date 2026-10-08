//! Basic object snapping for moves: the moving bounds' left/centre/right and
//! top/middle/bottom lines snap to the same lines of other visible layers.

use ude_core::{Document, NodeId};
use ude_spatial::SpatialIndex;
use ude_spatial::kurbo::{Point, Rect, Vec2};

/// Upper bound on candidate layers, keeping each pointer move cheap at any zoom.
const MAX_CANDIDATES: usize = 4000;

/// A snap line: its coordinate and the extent (along the other axis) of the layer it came from.
#[derive(Clone, Copy, Debug)]
struct Line {
    at: f64,
    from: f64,
    to: f64,
}

pub struct Snapper {
    xs: Vec<Line>,
    ys: Vec<Line>,
    threshold: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapResult {
    pub offset: Vec2,
    /// Guide segments in document coordinates.
    pub guides: Vec<(Point, Point)>,
}

impl Snapper {
    /// Collects snap lines from layers intersecting `region` (usually the visible area),
    /// excluding `moving` layers and their descendants. `threshold` is in document units.
    pub fn new(
        doc: &Document,
        index: &SpatialIndex,
        moving: &[NodeId],
        region: Rect,
        threshold: f64,
    ) -> Self {
        let query = ude_core::Rect::new(
            region.x0 as f32,
            region.y0 as f32,
            region.x1 as f32,
            region.y1 as f32,
        );
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for id in index.query_rect(query).take(MAX_CANDIDATES) {
            if moving.contains(&id) || doc.ancestors(id).any(|a| moving.contains(&a)) {
                continue;
            }
            let Some(b) = doc.world_bounds(id) else {
                continue;
            };
            let b = Rect::new(b.x0.into(), b.y0.into(), b.x1.into(), b.y1.into());
            for x in [b.x0, b.center().x, b.x1] {
                xs.push(Line {
                    at: x,
                    from: b.y0,
                    to: b.y1,
                });
            }
            for y in [b.y0, b.center().y, b.y1] {
                ys.push(Line {
                    at: y,
                    from: b.x0,
                    to: b.x1,
                });
            }
        }
        Self { xs, ys, threshold }
    }

    pub fn snap(&self, moving: Rect, snap_x: bool, snap_y: bool) -> SnapResult {
        let best = |lines: &[Line], values: [f64; 3]| -> Option<(f64, Line)> {
            let mut best: Option<(f64, Line)> = None;
            for l in lines {
                for v in values {
                    let d = l.at - v;
                    if d.abs() <= self.threshold && best.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                        best = Some((d, *l));
                    }
                }
            }
            best
        };
        let bx = if snap_x {
            best(&self.xs, [moving.x0, moving.center().x, moving.x1])
        } else {
            None
        };
        let by = if snap_y {
            best(&self.ys, [moving.y0, moving.center().y, moving.y1])
        } else {
            None
        };
        let offset = Vec2::new(bx.map_or(0.0, |b| b.0), by.map_or(0.0, |b| b.0));
        let snapped = moving + offset;
        let mut guides = Vec::new();
        if let Some((_, l)) = bx {
            guides.push((
                Point::new(l.at, l.from.min(snapped.y0)),
                Point::new(l.at, l.to.max(snapped.y1)),
            ));
        }
        if let Some((_, l)) = by {
            guides.push((
                Point::new(l.from.min(snapped.x0), l.at),
                Point::new(l.to.max(snapped.x1), l.at),
            ));
        }
        SnapResult { offset, guides }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Affine, Node, NodeKind, Symbol};

    #[test]
    fn snaps_edges_and_centres_within_threshold() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let fixed = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::translate(100.0, 100.0))
                    .with_size(100.0, 100.0),
            )
            .unwrap();
        let moving = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(50.0, 50.0),
            )
            .unwrap();
        let index = SpatialIndex::build(&doc, page);
        let snapper = Snapper::new(
            &doc,
            &index,
            &[moving],
            Rect::new(-1e4, -1e4, 1e4, 1e4),
            5.0,
        );
        assert!(snapper.xs.len() == 3, "the moving layer itself is excluded");
        let _ = fixed;

        // Left edge at 97 snaps to 100; top at 300 is too far from any line.
        let r = snapper.snap(Rect::new(97.0, 300.0, 147.0, 350.0), true, true);
        assert_eq!(r.offset, Vec2::new(3.0, 0.0));
        assert_eq!(r.guides.len(), 1);
        // Centre 152 snaps to centre 150.
        let r = snapper.snap(Rect::new(127.0, 0.0, 177.0, 50.0), true, false);
        assert_eq!(r.offset.x, -2.0);
    }
}
