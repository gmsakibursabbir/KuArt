//! Structural editing commands: duplicate, copy/paste, group/ungroup and stacking order. Each
//! command is one undoable transaction.

use ude_core::scene::BoolOpKind;
use ude_core::{
    Affine, DocError, Document, History, Node, NodeId, NodeKind, Op, Prop, Rect, Transaction,
};

/// Copied layers, independent of the document they came from (same-document paste only: names,
/// fonts and assets are referenced by the source document's tables).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    /// Top-level subtrees, each in its parent's coordinates.
    trees: Vec<Tree>,
}

#[derive(Clone, Debug)]
pub(crate) struct Tree {
    pub(crate) node: Node,
    pub(crate) children: Vec<Tree>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }
}

/// Top-level ids in paint order (an id whose ancestor is also selected is skipped).
pub(crate) fn top_level(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|&id| {
            doc.contains(id)
                && !matches!(
                    doc.node(id).map(|n| &n.kind),
                    Some(NodeKind::Page | NodeKind::Document)
                )
                && !doc.ancestors(id).any(|a| ids.contains(&a))
        })
        .collect();
    out.sort_by_key(|&id| index_in_parent(doc, id));
    out.dedup();
    out
}

fn index_in_parent(doc: &Document, id: NodeId) -> (Option<NodeId>, usize) {
    let parent = doc.node(id).and_then(|n| n.parent);
    let index = parent.map_or(0, |p| {
        doc.children(p).iter().position(|&c| c == id).unwrap_or(0)
    });
    (parent, index)
}

pub(crate) fn tree(doc: &Document, id: NodeId) -> Option<Tree> {
    let n = doc.node(id)?;
    let mut node = n.clone();
    node.parent = None;
    node.children = Vec::new();
    Some(Tree {
        node,
        children: n.children.iter().filter_map(|&c| tree(doc, c)).collect(),
    })
}

/// Creates `t` detached, then attaches it at `parent[index]` inside `tx`.
pub(crate) fn insert(
    doc: &mut Document,
    tx: &mut Transaction,
    t: &Tree,
    parent: NodeId,
    index: u32,
) -> Result<NodeId, DocError> {
    let id = doc.create(t.node.clone());
    tx.apply(doc, Op::Attach { id, parent, index })?;
    for (i, c) in t.children.iter().enumerate() {
        insert(doc, tx, c, id, i as u32)?;
    }
    Ok(id)
}

pub fn copy(doc: &Document, ids: &[NodeId]) -> Clipboard {
    Clipboard {
        trees: top_level(doc, ids)
            .into_iter()
            .filter_map(|id| tree(doc, id))
            .collect(),
    }
}

/// Pastes on top of `parent`'s children. Returns the new layers.
pub fn paste(
    doc: &mut Document,
    history: &mut History,
    clip: &Clipboard,
    parent: NodeId,
) -> Result<Vec<NodeId>, DocError> {
    let mut tx = Transaction::new("Paste");
    let mut out = Vec::new();
    for t in &clip.trees {
        let index = doc.children(parent).len() as u32;
        match insert(doc, &mut tx, t, parent, index) {
            Ok(id) => out.push(id),
            Err(e) => {
                tx.rollback(doc)?;
                return Err(e);
            }
        }
    }
    history.commit(doc, tx);
    Ok(out)
}

/// Duplicates each layer in place, directly above the original.
pub fn duplicate(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Vec<NodeId>, DocError> {
    let mut tx = Transaction::new("Duplicate");
    let mut out = Vec::new();
    for id in top_level(doc, ids).into_iter().rev() {
        let (Some(parent), index) = index_in_parent(doc, id) else {
            continue;
        };
        let Some(t) = tree(doc, id) else { continue };
        match insert(doc, &mut tx, &t, parent, index as u32 + 1) {
            Ok(new) => out.push(new),
            Err(e) => {
                tx.rollback(doc)?;
                return Err(e);
            }
        }
    }
    history.commit(doc, tx);
    out.reverse();
    Ok(out)
}

/// Groups layers under the parent of the topmost one; the group sits where that layer was.
/// Layers from other parents keep their on-screen position. Returns the group.
pub fn group(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Option<NodeId>, DocError> {
    group_as(doc, history, ids, NodeKind::Group, "Group")
}

/// Combines the layers into a live boolean group (bottom layer first, as in Figma). The group
/// takes the bottom layer's fills and strokes; the layers stay editable inside it.
pub fn boolean(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
    kind: BoolOpKind,
) -> Result<Option<NodeId>, DocError> {
    let label = match kind {
        BoolOpKind::Union => "Union",
        BoolOpKind::Subtract => "Subtract",
        BoolOpKind::Intersect => "Intersect",
        BoolOpKind::Exclude => "Exclude",
    };
    if top_level(doc, ids).len() < 2 {
        return Ok(None);
    }
    group_as(doc, history, ids, NodeKind::BooleanOp(kind), label)
}

/// Groups the layers and makes the bottom one a mask: it clips the others to its outline.
pub fn mask(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Option<NodeId>, DocError> {
    let Some(g) = group_as(doc, history, ids, NodeKind::Group, "Mask group")? else {
        return Ok(None);
    };
    let Some(&bottom) = doc.children(g).first() else {
        return Ok(Some(g));
    };
    let mut flags = doc.node(bottom).map(|n| n.flags).unwrap_or_default();
    flags.set(ude_core::scene::NodeFlags::IS_MASK, true);
    let mut tx = Transaction::new("Use as mask");
    tx.apply(
        doc,
        Op::Set {
            id: bottom,
            value: Prop::Flags(flags),
        },
    )?;
    history.commit(doc, tx);
    Ok(Some(g))
}

/// Replaces each selected boolean group by a single vector layer with its result.
/// Returns the new layers.
pub fn flatten(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Vec<NodeId>, DocError> {
    let mut tx = Transaction::new("Flatten");
    let mut out = Vec::new();
    let result = (|| {
        for id in top_level(doc, ids) {
            let (Some(parent), index) = index_in_parent(doc, id) else {
                continue;
            };
            let Some(v) = ude_boolean::as_vector(doc, id) else {
                continue;
            };
            let new = doc.create(v);
            tx.apply(
                doc,
                Op::Attach {
                    id: new,
                    parent,
                    index: index as u32 + 1,
                },
            )?;
            tx.apply(doc, Op::Detach { id })?;
            out.push(new);
        }
        Ok(())
    })();
    if let Err(e) = result {
        tx.rollback(doc)?;
        return Err(e);
    }
    if !out.is_empty() {
        history.commit(doc, tx);
    }
    Ok(out)
}

pub(crate) fn group_as(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
    kind: NodeKind,
    label: &'static str,
) -> Result<Option<NodeId>, DocError> {
    let mut ids = top_level(doc, ids);
    // Keep the stacking order (bottom first), whatever order they were selected in.
    ids.sort_by_key(|&id| index_in_parent(doc, id).1);
    let Some(&last) = ids.last() else {
        return Ok(None);
    };
    let (Some(parent), index) = index_in_parent(doc, last) else {
        return Ok(None);
    };
    let parent_world = doc.world_transform(parent);
    let to_parent = parent_world.inverse().unwrap_or(Affine::IDENTITY);
    // Each layer's transform and box in the new parent's space.
    let placed: Vec<(NodeId, Affine)> = ids
        .iter()
        .map(|&id| (id, to_parent * doc.world_transform(id)))
        .collect();
    let bounds = placed
        .iter()
        .filter_map(|&(id, t)| {
            let n = doc.node(id)?;
            Some(t.transform_rect_bbox(Rect::new(0.0, 0.0, n.size.w, n.size.h)))
        })
        .reduce(|a, b| a.union(&b))
        .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
    let boolean = matches!(kind, NodeKind::BooleanOp(_));
    let style = ids
        .first()
        .and_then(|&id| doc.node(id))
        .filter(|_| boolean)
        .map(|n| {
            (
                n.fills.clone(),
                n.ext.as_ref().and_then(|e| e.strokes.clone()),
            )
        });
    let name = doc.strings.intern(label);
    let mut node = Node::new(kind, name)
        .with_transform(Affine::translate(bounds.x0, bounds.y0))
        .with_size(bounds.width(), bounds.height());
    if let Some((fills, strokes)) = style {
        node.fills = fills;
        if strokes.is_some() {
            node.ext.get_or_insert_with(Default::default).strokes = strokes;
        }
    }
    let group = doc.create(node);
    let mut tx = Transaction::new(label);
    let result = (|| {
        // Insert above the topmost layer, then move the layers in, bottom first.
        tx.apply(
            doc,
            Op::Attach {
                id: group,
                parent,
                index: index as u32 + 1,
            },
        )?;
        for (i, &(id, t)) in placed.iter().enumerate() {
            tx.apply(doc, Op::Detach { id })?;
            tx.apply(
                doc,
                Op::Set {
                    id,
                    value: Prop::Transform(Affine::translate(-bounds.x0, -bounds.y0) * t),
                },
            )?;
            tx.apply(
                doc,
                Op::Attach {
                    id,
                    parent: group,
                    index: i as u32,
                },
            )?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        tx.rollback(doc)?;
        return Err(e);
    }
    history.commit(doc, tx);
    Ok(Some(group))
}

/// Replaces each selected group by its children (same place in the stack, same position).
/// Returns the released layers.
pub fn ungroup(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Vec<NodeId>, DocError> {
    let groups: Vec<NodeId> = top_level(doc, ids)
        .into_iter()
        .filter(|&id| {
            matches!(
                doc.node(id).map(|n| &n.kind),
                Some(NodeKind::Group | NodeKind::BooleanOp(_))
            )
        })
        .collect();
    let mut tx = Transaction::new("Ungroup");
    let mut out = Vec::new();
    let result = (|| {
        for g in groups {
            let (Some(parent), index) = index_in_parent(doc, g) else {
                continue;
            };
            let gt = doc.node(g).map_or(Affine::IDENTITY, |n| n.transform);
            let kids: Vec<NodeId> = doc.children(g).to_vec();
            for (i, &c) in kids.iter().enumerate() {
                let ct = doc.node(c).map_or(Affine::IDENTITY, |n| n.transform);
                tx.apply(doc, Op::Detach { id: c })?;
                tx.apply(
                    doc,
                    Op::Set {
                        id: c,
                        value: Prop::Transform(gt * ct),
                    },
                )?;
                tx.apply(
                    doc,
                    Op::Attach {
                        id: c,
                        parent,
                        index: (index + 1 + i) as u32,
                    },
                )?;
                out.push(c);
            }
            tx.apply(doc, Op::Detach { id: g })?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        tx.rollback(doc)?;
        return Err(e);
    }
    history.commit(doc, tx);
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Forward,
    Backward,
    ToFront,
    ToBack,
}

/// Moves layers within their parent's stack.
pub fn arrange(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
    how: Arrange,
) -> Result<(), DocError> {
    let mut ids = top_level(doc, ids);
    // Move the layer nearest the destination first so the others keep their relative order.
    if matches!(how, Arrange::Forward | Arrange::ToFront) {
        ids.reverse();
    }
    let mut tx = Transaction::new("Arrange");
    let result = (|| {
        for (k, &id) in ids.iter().enumerate() {
            let (Some(parent), index) = index_in_parent(doc, id) else {
                continue;
            };
            let count = doc.children(parent).len();
            let target = match how {
                Arrange::Forward => (index + 1).min(count - 1),
                Arrange::Backward => index.saturating_sub(1),
                Arrange::ToFront => count - 1 - k.min(count - 1),
                Arrange::ToBack => k.min(count - 1),
            };
            if target == index {
                continue;
            }
            tx.apply(doc, Op::Detach { id })?;
            tx.apply(
                doc,
                Op::Attach {
                    id,
                    parent,
                    index: target as u32,
                },
            )?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        tx.rollback(doc)?;
        return Err(e);
    }
    if !tx.is_empty() {
        history.commit(doc, tx);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
    /// Equal gaps between layers, left to right.
    DistributeH,
    /// Equal gaps between layers, top to bottom.
    DistributeV,
}

/// Aligns layers to their combined bounds (a single layer aligns to its parent frame), or
/// distributes three or more with equal gaps.
pub fn align(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
    how: Align,
) -> Result<(), DocError> {
    let ids = top_level(doc, ids);
    let boxes: Vec<(NodeId, Rect)> = ids
        .iter()
        .filter_map(|&id| Some((id, doc.world_bounds(id)?)))
        .collect();
    if boxes.is_empty() {
        return Ok(());
    }
    let target = if let [(id, _)] = boxes[..] {
        // One layer: align inside its parent frame (not a page).
        match doc.node(id).and_then(|n| n.parent) {
            Some(p) if !matches!(doc.node(p).map(|n| &n.kind), Some(NodeKind::Page)) => {
                match doc.world_bounds(p) {
                    Some(b) => b,
                    None => return Ok(()),
                }
            }
            _ => return Ok(()),
        }
    } else {
        boxes
            .iter()
            .map(|(_, b)| *b)
            .reduce(|a, b| a.union(&b))
            .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
    };
    let mut moves: Vec<(NodeId, f32, f32)> = Vec::new();
    match how {
        Align::DistributeH | Align::DistributeV if boxes.len() < 3 => return Ok(()),
        Align::DistributeH | Align::DistributeV => {
            let horizontal = how == Align::DistributeH;
            let mut sorted = boxes.clone();
            sorted.sort_by(|a, b| {
                let (ka, kb) = if horizontal {
                    (a.1.x0, b.1.x0)
                } else {
                    (a.1.y0, b.1.y0)
                };
                ka.total_cmp(&kb)
            });
            let total: f32 = sorted
                .iter()
                .map(|(_, b)| if horizontal { b.width() } else { b.height() })
                .sum();
            let span = if horizontal {
                target.width()
            } else {
                target.height()
            };
            let gap = (span - total) / (sorted.len() - 1) as f32;
            let mut at = if horizontal { target.x0 } else { target.y0 };
            for (id, b) in sorted {
                let delta = at - if horizontal { b.x0 } else { b.y0 };
                moves.push(if horizontal {
                    (id, delta, 0.0)
                } else {
                    (id, 0.0, delta)
                });
                at += (if horizontal { b.width() } else { b.height() }) + gap;
            }
        }
        _ => {
            for (id, b) in &boxes {
                let (dx, dy) = match how {
                    Align::Left => (target.x0 - b.x0, 0.0),
                    Align::HCenter => ((target.x0 + target.x1 - b.x0 - b.x1) / 2.0, 0.0),
                    Align::Right => (target.x1 - b.x1, 0.0),
                    Align::Top => (0.0, target.y0 - b.y0),
                    Align::VCenter => (0.0, (target.y0 + target.y1 - b.y0 - b.y1) / 2.0),
                    Align::Bottom => (0.0, target.y1 - b.y1),
                    _ => (0.0, 0.0),
                };
                moves.push((*id, dx, dy));
            }
        }
    }
    let mut tx = Transaction::new("Align");
    for (id, dx, dy) in moves {
        if dx.abs() < 1e-4 && dy.abs() < 1e-4 {
            continue;
        }
        let Some(n) = doc.node(id) else { continue };
        let to_parent = n.parent.map_or(Affine::IDENTITY, |p| {
            doc.world_transform(p).inverse().unwrap_or(Affine::IDENTITY)
        });
        let [a, b, c, d, _, _] = to_parent.0;
        let (lx, ly) = (a * dx + c * dy, b * dx + d * dy);
        let value = Prop::Transform(Affine::translate(lx, ly) * n.transform);
        if let Err(e) = tx.apply(doc, Op::Set { id, value }) {
            tx.rollback(doc)?;
            return Err(e);
        }
    }
    if !tx.is_empty() {
        history.commit(doc, tx);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Point, Symbol};

    fn setup() -> (Document, NodeId, [NodeId; 3]) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let mk = |doc: &mut Document, x: f32| {
            doc.build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::translate(x, 10.0))
                    .with_size(20.0, 20.0),
            )
            .unwrap()
        };
        let a = mk(&mut doc, 0.0);
        let b = mk(&mut doc, 100.0);
        let c = mk(&mut doc, 200.0);
        (doc, page, [a, b, c])
    }

    #[test]
    fn duplicate_copy_paste_undo() {
        let (mut doc, page, [a, b, _]) = setup();
        let mut h = History::default();
        let d = duplicate(&mut doc, &mut h, &[a, b]).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(
            doc.children(page)[1],
            d[0],
            "copy sits right above its original"
        );
        assert_eq!(doc.children(page).len(), 5);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(page).len(), 3);

        let clip = copy(&doc, &[a]);
        let p = paste(&mut doc, &mut h, &clip, page).unwrap();
        assert_eq!(*doc.children(page).last().unwrap(), p[0]);
        assert_eq!(
            doc.node(p[0]).unwrap().transform.translation(),
            Point::new(0.0, 10.0)
        );
    }

    #[test]
    fn boolean_flatten_and_undo() {
        let (mut doc, page, [a, b, c]) = setup();
        let mut h = History::default();
        // Selected top-first: the group still keeps stacking order (bottom first).
        let g = boolean(&mut doc, &mut h, &[b, a], BoolOpKind::Union)
            .unwrap()
            .unwrap();
        assert_eq!(doc.children(page), &[g, c]);
        assert_eq!(doc.children(g), &[a, b]);
        assert!(matches!(
            doc.node(g).unwrap().kind,
            NodeKind::BooleanOp(BoolOpKind::Union)
        ));
        assert!(
            boolean(&mut doc, &mut h, &[c], BoolOpKind::Union)
                .unwrap()
                .is_none()
        );

        let flat = flatten(&mut doc, &mut h, &[g]).unwrap();
        assert_eq!(doc.children(page), &[flat[0], c]);
        assert!(matches!(
            doc.node(flat[0]).unwrap().kind,
            NodeKind::Vector(_)
        ));
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(page), &[g, c]);
        assert_eq!(ungroup(&mut doc, &mut h, &[g]).unwrap(), vec![a, b]);
    }

    #[test]
    fn group_and_ungroup_keep_positions() {
        let (mut doc, page, [a, b, c]) = setup();
        let mut h = History::default();
        let g = group(&mut doc, &mut h, &[a, b]).unwrap().unwrap();
        assert_eq!(
            doc.children(page),
            &[g, c],
            "group takes the topmost member's place"
        );
        assert_eq!(doc.children(g), &[a, b]);
        let gn = doc.node(g).unwrap();
        assert_eq!(gn.transform.translation(), Point::new(0.0, 10.0));
        assert_eq!((gn.size.w, gn.size.h), (120.0, 20.0));
        assert_eq!(
            doc.world_transform(b).translation(),
            Point::new(100.0, 10.0)
        );

        let released = ungroup(&mut doc, &mut h, &[g]).unwrap();
        assert_eq!(released, vec![a, b]);
        assert_eq!(doc.children(page), &[a, b, c]);
        assert_eq!(
            doc.world_transform(b).translation(),
            Point::new(100.0, 10.0)
        );
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(page), &[g, c]);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(page), &[a, b, c]);
    }

    #[test]
    fn align_and_distribute() {
        let (mut doc, page, [a, b, c]) = setup();
        let mut h = History::default();
        // Offset b vertically, then align tops.
        doc.apply(Op::Set {
            id: b,
            value: Prop::Transform(Affine::translate(100.0, 40.0)),
        })
        .unwrap();
        align(&mut doc, &mut h, &[a, b, c], Align::Top).unwrap();
        for id in [a, b, c] {
            assert_eq!(doc.world_bounds(id).unwrap().y0, 10.0);
        }
        // Squeeze c left, then distribute: gaps become equal (80 each).
        doc.apply(Op::Set {
            id: c,
            value: Prop::Transform(Affine::translate(150.0, 10.0)),
        })
        .unwrap();
        doc.apply(Op::Set {
            id: c,
            value: Prop::Transform(Affine::translate(300.0, 10.0)),
        })
        .unwrap();
        align(&mut doc, &mut h, &[a, b, c], Align::DistributeH).unwrap();
        let xs: Vec<f32> = [a, b, c]
            .iter()
            .map(|&id| doc.world_bounds(id).unwrap().x0)
            .collect();
        assert_eq!(xs, [0.0, 150.0, 300.0]);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.world_bounds(b).unwrap().x0, 100.0);
        let _ = page;
    }

    #[test]
    fn arrange_layers() {
        let (mut doc, page, [a, b, c]) = setup();
        let mut h = History::default();
        arrange(&mut doc, &mut h, &[a], Arrange::Forward).unwrap();
        assert_eq!(doc.children(page), &[b, a, c]);
        arrange(&mut doc, &mut h, &[a, b], Arrange::ToFront).unwrap();
        assert_eq!(doc.children(page), &[c, b, a]);
        arrange(&mut doc, &mut h, &[a], Arrange::ToBack).unwrap();
        assert_eq!(doc.children(page), &[a, c, b]);
        arrange(&mut doc, &mut h, &[a], Arrange::Backward).unwrap();
        assert_eq!(doc.children(page), &[a, c, b], "already at the back");
    }
}
