//! Structural editing commands: duplicate, copy/paste, group/ungroup and stacking order. Each
//! command is one undoable transaction.

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
struct Tree {
    node: Node,
    children: Vec<Tree>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }
}

/// Top-level ids in paint order (an id whose ancestor is also selected is skipped).
fn top_level(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
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

fn tree(doc: &Document, id: NodeId) -> Option<Tree> {
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
fn insert(
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
    let ids = top_level(doc, ids);
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
    let name = doc.strings.intern("Group");
    let group = doc.create(
        Node::new(NodeKind::Group, name)
            .with_transform(Affine::translate(bounds.x0, bounds.y0))
            .with_size(bounds.width(), bounds.height()),
    );
    let mut tx = Transaction::new("Group");
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
        .filter(|&id| matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::Group)))
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
