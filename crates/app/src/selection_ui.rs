//! Turns selection state into device-space overlay outlines.

use ude_core::{Document, NodeId, NodeKind};
use ude_editor::Selection;
use ude_renderer::convert::affine;
use ude_renderer::vello::kurbo::{Affine, Point, Rect};
use ude_renderer::{Outline, Overlay};
use ude_spatial::SpatialIndex;

/// Marquee drag in progress: start point in document units and the selection it extends.
pub struct Marquee {
    pub start: Point,
    pub current: Point,
    pub base: Vec<NodeId>,
}

impl Marquee {
    pub fn doc_rect(&self) -> ude_core::Rect {
        let r = Rect::from_points(self.start, self.current);
        ude_core::Rect::new(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32)
    }
}

pub fn build_overlay(
    doc: &Document,
    index: &SpatialIndex,
    view: Affine,
    scale: f64,
    selection: &Selection,
    hover: Option<NodeId>,
    marquee: Option<&Marquee>,
) -> Overlay {
    let selected: Vec<Outline> = selection
        .ids()
        .iter()
        .filter_map(|&id| outline(doc, index, id, view))
        .collect();
    let selection_box = match selected.as_slice() {
        [] => None,
        [one] => Some(*one),
        many => {
            let bounds = many
                .iter()
                .flat_map(|o| o.quad)
                .fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| {
                    r.union_pt(p)
                });
            Some(Outline {
                quad: corners(bounds),
                component: false,
            })
        }
    };
    Overlay {
        hover: hover.and_then(|id| outline(doc, index, id, view)),
        selected,
        selection_box,
        marquee: marquee.map(|m| Rect::from_points(view * m.start, view * m.current)),
        scale,
    }
}

/// Device-space outline of a node: its (possibly rotated) box, or for box-less
/// containers (groups) the bounds of their indexed descendants.
fn outline(doc: &Document, index: &SpatialIndex, id: NodeId, view: Affine) -> Option<Outline> {
    let n = doc.node(id)?;
    let component = matches!(
        n.kind,
        NodeKind::Component(_) | NodeKind::ComponentSet | NodeKind::Instance(_)
    );
    let has_box = n.size.w > 0.0 || n.size.h > 0.0;
    let quad = if has_box && !matches!(n.kind, NodeKind::Group | NodeKind::BooleanOp(_)) {
        let t = view * affine(&doc.world_transform(id));
        let (w, h) = (f64::from(n.size.w), f64::from(n.size.h));
        [
            Point::ORIGIN,
            Point::new(w, 0.0),
            Point::new(w, h),
            Point::new(0.0, h),
        ]
        .map(|p| t * p)
    } else {
        let b = doc
            .descendants(id)
            .filter_map(|d| index.bounds(d))
            .reduce(|a, b| a.union(&b))?;
        let r = view.transform_rect_bbox(Rect::new(
            b.x0.into(),
            b.y0.into(),
            b.x1.into(),
            b.y1.into(),
        ));
        corners(r)
    };
    Some(Outline { quad, component })
}

fn corners(r: Rect) -> [Point; 4] {
    [
        Point::new(r.x0, r.y0),
        Point::new(r.x1, r.y0),
        Point::new(r.x1, r.y1),
        Point::new(r.x0, r.y1),
    ]
}
