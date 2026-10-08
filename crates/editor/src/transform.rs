//! Move, resize and rotate gestures.
//!
//! A gesture captures the original transforms at pointer-down. Every pointer move
//! computes *absolute* new values from those originals and commits them as one
//! transaction sharing the gesture's merge key, so the whole drag becomes a single
//! undo step that restores the pre-drag state (see `History` coalescing).
//!
//! Resizing changes node sizes, not scale factors, so stroke widths, corner radii
//! and text sizes are preserved, matching mainstream design tools.

use ude_core::{DocError, Document, History, NodeId, NodeKind, Op, Prop, Size, Transaction};
use ude_spatial::kurbo::{Affine, Point, Rect, Vec2};
use ude_spatial::{from_kurbo, to_kurbo};

use crate::snap::{SnapResult, Snapper};

/// Resize handle index, in `handle_points` order: TL, T, TR, R, BR, B, BL, L.
pub type HandleIndex = u8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Move: lock to an axis. Resize: keep aspect ratio. Rotate: 15° steps.
    pub shift: bool,
    /// Resize from the centre.
    pub alt: bool,
}

#[derive(Clone, Debug)]
struct Atom {
    id: NodeId,
    world: Affine,
    parent_world: Affine,
    size: (f64, f64),
}

#[derive(Clone, Debug)]
enum Kind {
    Move,
    /// Single box node resized in its own (possibly rotated) frame.
    ResizeLocal {
        handle: HandleIndex,
    },
    /// Selection bounds resized in document space (multi-selection or groups).
    ResizeBounds {
        handle: HandleIndex,
        bounds: Rect,
    },
    Rotate {
        center: Point,
    },
}

pub struct Gesture {
    kind: Kind,
    atoms: Vec<Atom>,
    start: Point,
    merge_key: u64,
    label: &'static str,
}

/// What the UI should show while the gesture runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    /// Snap guide lines in document coordinates.
    pub guides: Vec<(Point, Point)>,
}

const MIN_SIZE: f64 = 0.01;

impl Gesture {
    pub fn begin_move(doc: &Document, ids: &[NodeId], start: Point, merge_key: u64) -> Self {
        Self {
            kind: Kind::Move,
            atoms: atoms(doc, &top_level(doc, ids)),
            start,
            merge_key,
            label: "Move",
        }
    }

    pub fn begin_resize(
        doc: &Document,
        ids: &[NodeId],
        handle: HandleIndex,
        start: Point,
        merge_key: u64,
    ) -> Self {
        let ids = top_level(doc, ids);
        let single_box = match ids.as_slice() {
            [id] => doc.node(*id).is_some_and(|n| has_box(&n.kind)),
            _ => false,
        };
        if single_box {
            return Self {
                kind: Kind::ResizeLocal { handle },
                atoms: atoms(doc, &ids),
                start,
                merge_key,
                label: "Resize",
            };
        }
        // Groups have no box of their own: resize the box-bearing layers inside them.
        let mut leaves = Vec::new();
        for &id in &ids {
            box_atoms(doc, id, &mut leaves);
        }
        let atoms = atoms(doc, &leaves);
        let bounds = atoms
            .iter()
            .map(world_box)
            .reduce(|a, b| a.union(b))
            .unwrap_or(Rect::ZERO);
        Self {
            kind: Kind::ResizeBounds { handle, bounds },
            atoms,
            start,
            merge_key,
            label: "Resize",
        }
    }

    pub fn begin_rotate(doc: &Document, ids: &[NodeId], start: Point, merge_key: u64) -> Self {
        let atoms = atoms(doc, &top_level(doc, ids));
        let mut leaves = Vec::new();
        for a in &atoms {
            box_atoms(doc, a.id, &mut leaves);
        }
        let center = self::atoms(doc, &leaves)
            .iter()
            .map(world_box)
            .reduce(|a, b| a.union(b))
            .map_or(start, |r| r.center());
        Self {
            kind: Kind::Rotate { center },
            atoms,
            start,
            merge_key,
            label: "Rotate",
        }
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Applies the gesture for the current pointer position (document units).
    pub fn update(
        &self,
        doc: &mut Document,
        history: &mut History,
        pointer: Point,
        mods: Modifiers,
        snapper: Option<&Snapper>,
    ) -> Result<Feedback, DocError> {
        let mut feedback = Feedback::default();
        let mut ops = Vec::with_capacity(self.atoms.len() * 2);
        match &self.kind {
            Kind::Move => {
                let mut delta = pointer - self.start;
                if mods.shift {
                    if delta.x.abs() >= delta.y.abs() {
                        delta.y = 0.0
                    } else {
                        delta.x = 0.0
                    }
                }
                if let Some(snapper) = snapper {
                    let moving = self
                        .atoms
                        .iter()
                        .map(world_box)
                        .reduce(|a, b| a.union(b))
                        .unwrap_or(Rect::ZERO);
                    let SnapResult { offset, guides } = snapper.snap(
                        moving + delta,
                        !mods.shift || delta.x != 0.0,
                        !mods.shift || delta.y != 0.0,
                    );
                    delta += offset;
                    feedback.guides = guides;
                }
                for a in &self.atoms {
                    ops.push(set_world(a, Affine::translate(delta) * a.world));
                }
            }
            Kind::Rotate { center } => {
                let angle = |p: Point| (p - *center).atan2();
                let mut theta = angle(pointer) - angle(self.start);
                if mods.shift {
                    let step = 15f64.to_radians();
                    theta = (theta / step).round() * step;
                }
                let rot = Affine::translate(center.to_vec2())
                    * Affine::rotate(theta)
                    * Affine::translate(-center.to_vec2());
                for a in &self.atoms {
                    ops.push(set_world(a, rot * a.world));
                }
            }
            Kind::ResizeLocal { handle } => {
                let a = &self.atoms[0];
                let p = a.world.inverse() * pointer;
                let r = resize_rect(Rect::new(0.0, 0.0, a.size.0, a.size.1), *handle, p, mods);
                let local = a.parent_world.inverse() * a.world * Affine::translate((r.x0, r.y0));
                ops.push(Op::Set {
                    id: a.id,
                    value: Prop::Transform(from_kurbo(local)),
                });
                ops.push(Op::Set {
                    id: a.id,
                    value: Prop::Size(Size::new(r.width() as f32, r.height() as f32)),
                });
            }
            Kind::ResizeBounds { handle, bounds } => {
                let r = resize_rect(*bounds, *handle, pointer, mods);
                let sx = if bounds.width() > 0.0 {
                    r.width() / bounds.width()
                } else {
                    1.0
                };
                let sy = if bounds.height() > 0.0 {
                    r.height() / bounds.height()
                } else {
                    1.0
                };
                let map = |p: Point| {
                    Point::new(r.x0 + (p.x - bounds.x0) * sx, r.y0 + (p.y - bounds.y0) * sy)
                };
                for a in &self.atoms {
                    let [m00, m10, m01, m11, ..] = a.world.as_coeffs();
                    let axis_aligned =
                        m10.abs() < 1e-9 && m01.abs() < 1e-9 && m00 > 0.0 && m11 > 0.0;
                    if axis_aligned {
                        // Scale position and size; keep the node's own scale factors.
                        let origin = map(a.world * Point::ORIGIN);
                        let world = Affine::translate(origin.to_vec2())
                            * Affine::new([m00, 0.0, 0.0, m11, 0.0, 0.0]);
                        ops.push(set_world(a, world));
                        let size = Size::new(
                            (a.size.0 * sx).max(MIN_SIZE) as f32,
                            (a.size.1 * sy).max(MIN_SIZE) as f32,
                        );
                        ops.push(Op::Set {
                            id: a.id,
                            value: Prop::Size(size),
                        });
                    } else {
                        // Rotated layers keep their size; their centre follows the scaling.
                        let c = a.world * Point::new(a.size.0 / 2.0, a.size.1 / 2.0);
                        ops.push(set_world(a, Affine::translate(map(c) - c) * a.world));
                    }
                }
            }
        }

        let mut tx = Transaction::new(self.label).with_merge_key(self.merge_key);
        for op in ops {
            if let Err(e) = tx.apply(doc, op) {
                tx.rollback(doc)?;
                return Err(e);
            }
        }
        history.commit(doc, tx);
        Ok(feedback)
    }
}

/// New rectangle when dragging `handle` of `r` to `p`.
fn resize_rect(r: Rect, handle: HandleIndex, p: Point, mods: Modifiers) -> Rect {
    let (left, right) = (matches!(handle, 0 | 6 | 7), matches!(handle, 2..=4));
    let (top, bottom) = (matches!(handle, 0..=2), matches!(handle, 4..=6));
    let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
    let c = r.center();
    if left {
        x0 = p.x;
        if mods.alt {
            x1 = 2.0 * c.x - p.x
        }
    }
    if right {
        x1 = p.x;
        if mods.alt {
            x0 = 2.0 * c.x - p.x
        }
    }
    if top {
        y0 = p.y;
        if mods.alt {
            y1 = 2.0 * c.y - p.y
        }
    }
    if bottom {
        y1 = p.y;
        if mods.alt {
            y0 = 2.0 * c.y - p.y
        }
    }
    let corner = (left || right) && (top || bottom);
    if mods.shift && corner && r.width() > 0.0 && r.height() > 0.0 {
        // Uniform scale by the larger change, anchored like the unconstrained drag.
        let s = ((x1 - x0) / r.width())
            .abs()
            .max(((y1 - y0) / r.height()).abs());
        let (w, h) = (r.width() * s, r.height() * s);
        if mods.alt {
            (x0, x1, y0, y1) = (c.x - w / 2.0, c.x + w / 2.0, c.y - h / 2.0, c.y + h / 2.0);
        } else {
            if left {
                x0 = x1 - w * (x1 - x0).signum()
            } else {
                x1 = x0 + w * (x1 - x0).signum()
            }
            if top {
                y0 = y1 - h * (y1 - y0).signum()
            } else {
                y1 = y0 + h * (y1 - y0).signum()
            }
        }
    }
    // Dragging past the opposite edge flips the rectangle around rather than inverting it.
    let mut out = Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    if out.width() < MIN_SIZE {
        out.x1 = out.x0 + MIN_SIZE;
    }
    if out.height() < MIN_SIZE {
        out.y1 = out.y0 + MIN_SIZE;
    }
    out
}

fn set_world(a: &Atom, world: Affine) -> Op {
    Op::Set {
        id: a.id,
        value: Prop::Transform(from_kurbo(a.parent_world.inverse() * world)),
    }
}

fn has_box(kind: &NodeKind) -> bool {
    !matches!(
        kind,
        NodeKind::Group | NodeKind::BooleanOp(_) | NodeKind::Page | NodeKind::Document
    )
}

/// Removes ids whose ancestor is also selected (moving both would move the child twice).
fn top_level(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter()
        .copied()
        .filter(|&id| doc.contains(id) && !doc.ancestors(id).any(|a| ids.contains(&a)))
        .collect()
}

/// Box-bearing layers that represent `id` for resizing: itself, or for groups their contents.
fn box_atoms(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
    let Some(n) = doc.node(id) else { return };
    if has_box(&n.kind) {
        out.push(id);
    } else {
        for &c in &n.children {
            box_atoms(doc, c, out);
        }
    }
}

fn atoms(doc: &Document, ids: &[NodeId]) -> Vec<Atom> {
    ids.iter()
        .filter_map(|&id| {
            let n = doc.node(id)?;
            let parent_world = n
                .parent
                .map_or(Affine::IDENTITY, |p| to_kurbo(&doc.world_transform(p)));
            Some(Atom {
                id,
                world: parent_world * to_kurbo(&n.transform),
                parent_world,
                size: (f64::from(n.size.w), f64::from(n.size.h)),
            })
        })
        .collect()
}

fn world_box(a: &Atom) -> Rect {
    a.world
        .transform_rect_bbox(Rect::new(0.0, 0.0, a.size.0, a.size.1))
}

/// Translation for keyboard nudges (document units), as one coalescable transaction.
pub fn nudge(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
    delta: Vec2,
    merge_key: u64,
) -> Result<(), DocError> {
    let atoms = atoms(doc, &top_level(doc, ids));
    let mut tx = Transaction::new("Nudge").with_merge_key(merge_key);
    for a in &atoms {
        tx.apply(doc, set_world(a, Affine::translate(delta) * a.world))?;
    }
    history.commit(doc, tx);
    Ok(())
}

/// Deletes the selection (detaches; undo restores the same nodes at the same positions).
pub fn delete(doc: &mut Document, history: &mut History, ids: &[NodeId]) -> Result<(), DocError> {
    let mut tx = Transaction::new("Delete");
    for id in top_level(doc, ids) {
        if let Err(e) = tx.apply(doc, Op::Detach { id }) {
            tx.rollback(doc)?;
            return Err(e);
        }
    }
    history.commit(doc, tx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Affine as CoreAffine, Node, Symbol};

    fn setup() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let a = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(CoreAffine::translate(0.0, 0.0))
                    .with_size(100.0, 50.0),
            )
            .unwrap();
        let b = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(CoreAffine::translate(200.0, 0.0))
                    .with_size(100.0, 50.0),
            )
            .unwrap();
        (doc, page, a, b)
    }

    fn pos(doc: &Document, id: NodeId) -> (f32, f32) {
        let t = doc.node(id).unwrap().transform.translation();
        (t.x, t.y)
    }

    fn size(doc: &Document, id: NodeId) -> (f32, f32) {
        let s = doc.node(id).unwrap().size;
        (s.w, s.h)
    }

    #[test]
    fn drag_is_one_undo_step() {
        let (mut doc, _, a, _) = setup();
        let mut h = History::default();
        let g = Gesture::begin_move(&doc, &[a], Point::new(10.0, 10.0), 1);
        for i in 1..=50 {
            g.update(
                &mut doc,
                &mut h,
                Point::new(10.0 + f64::from(i), 10.0 + f64::from(i) / 2.0),
                Modifiers::default(),
                None,
            )
            .unwrap();
        }
        assert_eq!(pos(&doc, a), (50.0, 25.0));
        assert_eq!(h.len(), 1, "50 pointer moves coalesce into one entry");
        h.undo(&mut doc).unwrap();
        assert_eq!(pos(&doc, a), (0.0, 0.0));
    }

    #[test]
    fn shift_locks_axis() {
        let (mut doc, _, a, _) = setup();
        let mut h = History::default();
        let g = Gesture::begin_move(&doc, &[a], Point::ORIGIN, 1);
        g.update(
            &mut doc,
            &mut h,
            Point::new(30.0, 5.0),
            Modifiers {
                shift: true,
                alt: false,
            },
            None,
        )
        .unwrap();
        assert_eq!(pos(&doc, a), (30.0, 0.0));
    }

    #[test]
    fn resize_handles_and_modifiers() {
        let (mut doc, _, a, _) = setup();
        let mut h = History::default();
        // Bottom-right corner to (150, 100): 150×100.
        let g = Gesture::begin_resize(&doc, &[a], 4, Point::new(100.0, 50.0), 1);
        g.update(
            &mut doc,
            &mut h,
            Point::new(150.0, 100.0),
            Modifiers::default(),
            None,
        )
        .unwrap();
        assert_eq!((pos(&doc, a), size(&doc, a)), ((0.0, 0.0), (150.0, 100.0)));
        // Shift keeps the 2:1 aspect ratio.
        g.update(
            &mut doc,
            &mut h,
            Point::new(200.0, 60.0),
            Modifiers {
                shift: true,
                alt: false,
            },
            None,
        )
        .unwrap();
        assert_eq!(size(&doc, a), (200.0, 100.0));
        // Alt resizes from the centre (50, 25): right edge to 120 → left edge to -20.
        h.undo(&mut doc).unwrap();
        let g = Gesture::begin_resize(&doc, &[a], 3, Point::new(100.0, 25.0), 2);
        g.update(
            &mut doc,
            &mut h,
            Point::new(120.0, 25.0),
            Modifiers {
                shift: false,
                alt: true,
            },
            None,
        )
        .unwrap();
        assert_eq!((pos(&doc, a), size(&doc, a)), ((-20.0, 0.0), (140.0, 50.0)));
        // Left handle dragged past the right edge flips without negative size.
        let g = Gesture::begin_resize(&doc, &[a], 7, Point::new(-20.0, 25.0), 3);
        g.update(
            &mut doc,
            &mut h,
            Point::new(200.0, 25.0),
            Modifiers::default(),
            None,
        )
        .unwrap();
        assert_eq!((pos(&doc, a), size(&doc, a)), ((120.0, 0.0), (80.0, 50.0)));
    }

    #[test]
    fn multi_resize_scales_positions_and_sizes() {
        let (mut doc, _, a, b) = setup();
        let mut h = History::default();
        // Bounds 0..300 × 0..50; drag right edge to 600 → x scale 2.
        let g = Gesture::begin_resize(&doc, &[a, b], 3, Point::new(300.0, 25.0), 1);
        g.update(
            &mut doc,
            &mut h,
            Point::new(600.0, 25.0),
            Modifiers::default(),
            None,
        )
        .unwrap();
        assert_eq!((pos(&doc, a), size(&doc, a)), ((0.0, 0.0), (200.0, 50.0)));
        assert_eq!((pos(&doc, b), size(&doc, b)), ((400.0, 0.0), (200.0, 50.0)));
    }

    #[test]
    fn rotate_with_shift_snaps_to_15_degrees() {
        let (mut doc, _, a, _) = setup();
        let mut h = History::default();
        // Centre (50, 25); start to the right, drag to ~ +40° → snaps to 45°.
        let g = Gesture::begin_rotate(&doc, &[a], Point::new(150.0, 25.0), 1);
        let p = Point::new(
            50.0 + 100.0 * 40f64.to_radians().cos(),
            25.0 + 100.0 * 40f64.to_radians().sin(),
        );
        g.update(
            &mut doc,
            &mut h,
            p,
            Modifiers {
                shift: true,
                alt: false,
            },
            None,
        )
        .unwrap();
        let t = doc.node(a).unwrap().transform.0;
        let angle = f64::from(t[1]).atan2(f64::from(t[0])).to_degrees();
        assert!((angle - 45.0).abs() < 1e-3, "angle {angle}");
        // The centre stays put.
        let c = doc
            .world_transform(a)
            .apply(ude_core::Point::new(50.0, 25.0));
        assert!((c.x - 50.0).abs() < 1e-3 && (c.y - 25.0).abs() < 1e-3);
    }

    #[test]
    fn child_moves_in_parent_space() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        // Parent scaled 2× and offset: a 10-unit document move is 5 local units.
        let frame = doc
            .build_child(
                page,
                Node::new(NodeKind::Frame, Symbol::EMPTY)
                    .with_transform(CoreAffine::translate(100.0, 0.0) * CoreAffine::scale(2.0, 2.0))
                    .with_size(100.0, 100.0),
            )
            .unwrap();
        let child = doc
            .build_child(
                frame,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(10.0, 10.0),
            )
            .unwrap();
        let mut h = History::default();
        let g = Gesture::begin_move(&doc, &[child, frame], Point::ORIGIN, 1);
        g.update(
            &mut doc,
            &mut h,
            Point::new(10.0, 0.0),
            Modifiers::default(),
            None,
        )
        .unwrap();
        assert_eq!(
            pos(&doc, frame),
            (110.0, 0.0),
            "only the top-level selection moves"
        );
        assert_eq!(pos(&doc, child), (0.0, 0.0));

        let g = Gesture::begin_move(&doc, &[child], Point::ORIGIN, 2);
        g.update(
            &mut doc,
            &mut h,
            Point::new(10.0, 0.0),
            Modifiers::default(),
            None,
        )
        .unwrap();
        assert_eq!(pos(&doc, child), (5.0, 0.0));
    }

    #[test]
    fn delete_and_undo() {
        let (mut doc, page, a, b) = setup();
        let mut h = History::default();
        delete(&mut doc, &mut h, &[a]).unwrap();
        assert_eq!(doc.children(page), &[b]);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(page), &[a, b]);
    }
}
