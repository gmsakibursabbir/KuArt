//! Spatial queries over one page: an R-tree of world-space node bounds, kept in sync
//! incrementally from [`ChangeSet`]s, plus exact hit testing against node outlines.
//!
//! Used by the editor (hit testing, marquee, snapping candidates) and, later, by the
//! renderer for culling. See `docs/rendering-strategy.md` §8–9.

mod hit;

/// kurbo (f64) is used for editor math; the model stores compact f32 values.
pub use kurbo;

/// Core (f32) → kurbo (f64) transform.
pub fn to_kurbo(a: &Affine) -> kurbo::Affine {
    kurbo::Affine::new(a.0.map(f64::from))
}

/// kurbo (f64) → core (f32) transform.
pub fn from_kurbo(a: kurbo::Affine) -> Affine {
    Affine(a.as_coeffs().map(|v| v as f32))
}

pub use hit::{contains_point, paint_order_cmp};

use std::collections::HashMap;

use rstar::{AABB, RTree, RTreeObject};
use ude_core::{Affine, ChangeSet, Document, NodeId, NodeKind, Rect};

/// One indexed node: its world-space bounds including stroke/effect outsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub id: NodeId,
    pub bounds: Rect,
}

impl RTreeObject for Entry {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [self.bounds.x0, self.bounds.y0],
            [self.bounds.x1, self.bounds.y1],
        )
    }
}

/// Above this fraction of changed nodes, rebuilding (bulk load) beats incremental updates.
const REBUILD_FRACTION: f32 = 0.25;

pub struct SpatialIndex {
    page: NodeId,
    tree: RTree<Entry>,
    entries: HashMap<NodeId, Entry>,
}

/// Nodes with their own geometry. Containers without a box (groups, boolean ops) are
/// found through their children; the editor maps hits to the right selection level.
fn has_own_bounds(kind: &NodeKind) -> bool {
    !matches!(
        kind,
        NodeKind::Document
            | NodeKind::Page
            | NodeKind::Group
            | NodeKind::BooleanOp(_)
            | NodeKind::Slice
    )
}

impl SpatialIndex {
    pub fn build(doc: &Document, page: NodeId) -> Self {
        let mut entries = Vec::new();
        collect(doc, page, Affine::IDENTITY, &mut entries);
        Self {
            page,
            entries: entries.iter().map(|e| (e.id, *e)).collect(),
            tree: RTree::bulk_load(entries),
        }
    }

    pub fn page(&self) -> NodeId {
        self.page
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn bounds(&self, id: NodeId) -> Option<Rect> {
        self.entries.get(&id).map(|e| e.bounds)
    }

    /// Applies document changes. Subtrees of changed nodes are re-indexed (a moved
    /// frame moves its children); detached or hidden subtrees are removed.
    pub fn update(&mut self, doc: &Document, changes: &ChangeSet) {
        if changes.nodes.is_empty() {
            return;
        }
        if changes.nodes.len() as f32 > self.entries.len().max(64) as f32 * REBUILD_FRACTION {
            *self = Self::build(doc, self.page);
            return;
        }
        for &id in &changes.nodes {
            self.remove_subtree(doc, id);
            if self.on_page(doc, id) {
                let parent = doc
                    .node(id)
                    .and_then(|n| n.parent)
                    .map_or(Affine::IDENTITY, |p| doc.world_transform(p));
                let mut fresh = Vec::new();
                collect_node(doc, id, parent, &mut fresh);
                for e in fresh {
                    self.tree.insert(e);
                    self.entries.insert(e.id, e);
                }
            }
        }
    }

    /// Candidates whose bounds contain `(x, y)` (document units), unordered.
    pub fn query_point(&self, x: f32, y: f32, tolerance: f32) -> impl Iterator<Item = NodeId> + '_ {
        self.query_rect(Rect::new(
            x - tolerance,
            y - tolerance,
            x + tolerance,
            y + tolerance,
        ))
    }

    /// Nodes whose bounds intersect `rect`, unordered.
    pub fn query_rect(&self, rect: Rect) -> impl Iterator<Item = NodeId> + '_ {
        let env = AABB::from_corners([rect.x0, rect.y0], [rect.x1, rect.y1]);
        self.tree.locate_in_envelope_intersecting(env).map(|e| e.id)
    }

    /// Approximate heap use (tree nodes + id map), for the memory panel.
    pub fn heap_bytes(&self) -> usize {
        // rstar stores leaves plus ~1/6 internal nodes; each holds an envelope and children.
        self.entries.len() * (size_of::<Entry>() * 7 / 6 + 16)
            + self.entries.capacity() * (size_of::<NodeId>() + size_of::<Entry>() + 8)
    }

    fn on_page(&self, doc: &Document, id: NodeId) -> bool {
        doc.contains(id) && id != self.page && doc.ancestors(id).any(|a| a == self.page)
    }

    fn remove_subtree(&mut self, doc: &Document, id: NodeId) {
        // Detached nodes still exist (kept for undo), so their subtree can be walked.
        // Purged nodes were detached first, so they are already gone from the index.
        let ids: Vec<NodeId> = if doc.contains(id) {
            doc.descendants(id).collect()
        } else {
            vec![id]
        };
        for d in ids {
            if let Some(e) = self.entries.remove(&d) {
                self.tree.remove(&e);
            }
        }
    }
}

/// Indexes the children of `parent` (whose world transform is `world`), recursively.
fn collect(doc: &Document, parent: NodeId, world: Affine, out: &mut Vec<Entry>) {
    for &child in doc.children(parent) {
        collect_node(doc, child, world, out);
    }
}

fn collect_node(doc: &Document, id: NodeId, parent_world: Affine, out: &mut Vec<Entry>) {
    let Some(n) = doc.node(id) else { return };
    if !n.is_visible() {
        return;
    }
    let world = parent_world * n.transform;
    if has_own_bounds(&n.kind) {
        let o = n.paint_outset();
        let local = Rect::new(-o, -o, n.size.w + o, n.size.h + o);
        out.push(Entry {
            id,
            bounds: world.transform_rect_bbox(local),
        });
    }
    collect(doc, id, world, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Node, Op, Prop, Symbol};

    fn doc_with_frame() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let frame = doc
            .build_child(
                page,
                Node::new(NodeKind::Frame, Symbol::EMPTY)
                    .with_transform(Affine::translate(100.0, 100.0))
                    .with_size(200.0, 200.0),
            )
            .unwrap();
        let rect = doc
            .build_child(
                frame,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::translate(10.0, 10.0))
                    .with_size(20.0, 20.0),
            )
            .unwrap();
        (doc, page, frame, rect)
    }

    #[test]
    fn indexes_world_bounds() {
        let (doc, page, frame, rect) = doc_with_frame();
        let idx = SpatialIndex::build(&doc, page);
        assert_eq!(idx.len(), 2);
        assert_eq!(
            idx.bounds(rect),
            Some(Rect::new(110.0, 110.0, 130.0, 130.0))
        );
        let mut hits: Vec<_> = idx.query_point(120.0, 120.0, 0.0).collect();
        hits.sort();
        let mut expected = vec![frame, rect];
        expected.sort();
        assert_eq!(hits, expected);
        assert_eq!(idx.query_point(50.0, 50.0, 0.0).count(), 0);
    }

    #[test]
    fn moving_a_parent_moves_indexed_children() {
        let (mut doc, page, frame, rect) = doc_with_frame();
        let mut idx = SpatialIndex::build(&doc, page);
        let _ = doc.take_changes();
        doc.apply(Op::Set {
            id: frame,
            value: Prop::Transform(Affine::translate(500.0, 0.0)),
        })
        .unwrap();
        let changes = doc.take_changes();
        idx.update(&doc, &changes);
        assert_eq!(idx.bounds(rect), Some(Rect::new(510.0, 10.0, 530.0, 30.0)));
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn detach_hide_and_reattach() {
        let (mut doc, page, frame, rect) = doc_with_frame();
        let mut idx = SpatialIndex::build(&doc, page);
        let _ = doc.take_changes();

        let undo = doc.apply(Op::Detach { id: frame }).unwrap();
        let changes = doc.take_changes();
        idx.update(&doc, &changes);
        assert!(idx.is_empty(), "detached subtree leaves the index");

        doc.apply(undo).unwrap();
        let changes = doc.take_changes();
        idx.update(&doc, &changes);
        assert_eq!(idx.len(), 2);

        let mut flags = doc.node(rect).unwrap().flags;
        flags.set(ude_core::NodeFlags::VISIBLE, false);
        doc.apply(Op::Set {
            id: rect,
            value: Prop::Flags(flags),
        })
        .unwrap();
        let changes = doc.take_changes();
        idx.update(&doc, &changes);
        assert_eq!(idx.bounds(rect), None, "hidden nodes are not hit-testable");
    }
}
