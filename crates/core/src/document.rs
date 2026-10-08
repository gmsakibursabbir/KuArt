//! The document: node arena, tree invariants, and the single mutation path ([`Op`]).

use std::fmt;
use std::mem;

use crate::assets::AssetStore;
use crate::geometry::{Affine, Rect, Size};
use crate::id::{Arena, NodeId};
use crate::intern::{Interner, Symbol};
use crate::memory::{HeapSize, MemoryReport};
use crate::scene::{Node, NodeExt, NodeFlags, NodeKind};
use crate::styles::{BlendMode, Paints};

/// One replaceable node field. Applying a `Set` swaps the value in, returning the old one.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Name(Symbol),
    Transform(Affine),
    Size(Size),
    Opacity(f32),
    Blend(BlendMode),
    Flags(NodeFlags),
    Fills(Paints),
    Ext(Option<Box<NodeExt>>),
    Kind(NodeKind),
}

impl Prop {
    /// Identifies the field, for coalescing repeated edits of the same property.
    pub fn key(&self) -> u8 {
        match self {
            Prop::Name(_) => 0,
            Prop::Transform(_) => 1,
            Prop::Size(_) => 2,
            Prop::Opacity(_) => 3,
            Prop::Blend(_) => 4,
            Prop::Flags(_) => 5,
            Prop::Fills(_) => 6,
            Prop::Ext(_) => 7,
            Prop::Kind(_) => 8,
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            Prop::Fills(p) => p.heap_bytes(),
            Prop::Ext(Some(e)) => size_of::<NodeExt>() + e.heap_bytes(),
            Prop::Kind(k) => k.heap_bytes(),
            _ => 0,
        }
    }
}

/// Reversible document mutation. [`Document::apply`] returns the inverse.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Insert a detached node under `parent` at `index`.
    Attach {
        id: NodeId,
        parent: NodeId,
        index: u32,
    },
    /// Remove a node (and its subtree) from the tree; it stays in the arena for undo.
    Detach {
        id: NodeId,
    },
    Set {
        id: NodeId,
        value: Prop,
    },
}

impl Op {
    pub fn id(&self) -> NodeId {
        match self {
            Op::Attach { id, .. } | Op::Detach { id } | Op::Set { id, .. } => *id,
        }
    }

    pub fn heap_size(&self) -> usize {
        size_of::<Op>()
            + if let Op::Set { value, .. } = self {
                value.heap_bytes()
            } else {
                0
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocError {
    NotFound(NodeId),
    AlreadyAttached(NodeId),
    NotAttached(NodeId),
    IndexOutOfRange { parent: NodeId, index: u32 },
    Cycle { id: NodeId, parent: NodeId },
    RootImmutable,
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocError::NotFound(id) => write!(f, "node {id:?} does not exist"),
            DocError::AlreadyAttached(id) => write!(f, "node {id:?} is already attached"),
            DocError::NotAttached(id) => write!(f, "node {id:?} is not attached"),
            DocError::IndexOutOfRange { parent, index } => {
                write!(f, "index {index} out of range for {parent:?}")
            }
            DocError::Cycle { id, parent } => {
                write!(f, "attaching {id:?} under {parent:?} would create a cycle")
            }
            DocError::RootImmutable => write!(f, "the document root cannot be moved or retyped"),
        }
    }
}

impl std::error::Error for DocError {}

/// Nodes touched since the last [`Document::take_changes`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub nodes: Vec<NodeId>,
    /// Tree structure changed (attach/detach); layers panel and spatial index must refresh.
    pub structure: bool,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && !self.structure
    }
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: String,
    pub source: Option<crate::scene::SourceFormat>,
}

pub struct Document {
    arena: Arena,
    root: NodeId,
    pub strings: Interner,
    pub assets: AssetStore,
    pub meta: Metadata,
    changes: ChangeSet,
    revision: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        let mut strings = Interner::default();
        let name = strings.intern("Document");
        let mut arena = Arena::default();
        let root = arena.alloc(Node::new(NodeKind::Document, name));
        Self {
            arena,
            root,
            strings,
            assets: AssetStore::default(),
            meta: Metadata::default(),
            changes: ChangeSet::default(),
            revision: 0,
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.arena.get(id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.arena.get(id).is_some()
    }

    /// Live nodes, including detached ones kept alive for undo.
    pub fn node_count(&self) -> usize {
        self.arena.len()
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.arena.get(id).map_or(&[], |n| &n.children)
    }

    /// Allocates a detached node. Tree links in `node` are reset; use `Op::Attach` to insert it.
    pub fn create(&mut self, mut node: Node) -> NodeId {
        node.parent = None;
        node.children = Vec::new();
        self.arena.alloc(node)
    }

    /// Non-undoable construction for importers: create and append under `parent`.
    pub fn build_child(&mut self, parent: NodeId, node: Node) -> Result<NodeId, DocError> {
        let index = u32::try_from(self.children(parent).len()).expect("too many children");
        if !self.contains(parent) {
            return Err(DocError::NotFound(parent));
        }
        let id = self.create(node);
        self.apply(Op::Attach { id, parent, index })?;
        Ok(id)
    }

    /// The only mutation path. Returns the inverse operation.
    pub fn apply(&mut self, op: Op) -> Result<Op, DocError> {
        let inverse = match op {
            Op::Attach { id, parent, index } => {
                if id == self.root {
                    return Err(DocError::RootImmutable);
                }
                let node = self.arena.get(id).ok_or(DocError::NotFound(id))?;
                if node.parent.is_some() {
                    return Err(DocError::AlreadyAttached(id));
                }
                let p = self.arena.get(parent).ok_or(DocError::NotFound(parent))?;
                if index as usize > p.children.len() {
                    return Err(DocError::IndexOutOfRange { parent, index });
                }
                if self.is_ancestor_or_self(id, parent) {
                    return Err(DocError::Cycle { id, parent });
                }
                self.arena
                    .get_mut(parent)
                    .ok_or(DocError::NotFound(parent))?
                    .children
                    .insert(index as usize, id);
                self.arena.get_mut(id).ok_or(DocError::NotFound(id))?.parent = Some(parent);
                self.changes.structure = true;
                self.changes.nodes.push(parent);
                Op::Detach { id }
            }
            Op::Detach { id } => {
                if id == self.root {
                    return Err(DocError::RootImmutable);
                }
                let parent = self
                    .arena
                    .get(id)
                    .ok_or(DocError::NotFound(id))?
                    .parent
                    .ok_or(DocError::NotAttached(id))?;
                let siblings = &mut self
                    .arena
                    .get_mut(parent)
                    .ok_or(DocError::NotFound(parent))?
                    .children;
                let index = siblings
                    .iter()
                    .position(|&c| c == id)
                    .ok_or(DocError::NotAttached(id))?;
                siblings.remove(index);
                self.arena.get_mut(id).ok_or(DocError::NotFound(id))?.parent = None;
                self.changes.structure = true;
                self.changes.nodes.push(parent);
                Op::Attach {
                    id,
                    parent,
                    index: index as u32,
                }
            }
            Op::Set { id, value } => {
                if id == self.root && matches!(value, Prop::Kind(_)) {
                    return Err(DocError::RootImmutable);
                }
                let node = self.arena.get_mut(id).ok_or(DocError::NotFound(id))?;
                let old = swap_prop(node, value);
                Op::Set { id, value: old }
            }
        };
        self.changes.nodes.push(inverse.id());
        self.revision += 1;
        Ok(inverse)
    }

    /// Frees a detached subtree. Returns `false` (and does nothing) if `id` is attached or is the root.
    pub fn purge(&mut self, id: NodeId) -> bool {
        match self.arena.get(id) {
            Some(n) if n.parent.is_none() && id != self.root => {}
            _ => return false,
        }
        let doomed: Vec<NodeId> = self.descendants(id).collect();
        for d in doomed {
            self.arena.free(d);
        }
        true
    }

    /// True if `id` is reachable from the root.
    pub fn is_attached(&self, id: NodeId) -> bool {
        self.ancestors(id).last() == Some(self.root) || (id == self.root)
    }

    fn is_ancestor_or_self(&self, ancestor: NodeId, id: NodeId) -> bool {
        id == ancestor || self.ancestors(id).any(|a| a == ancestor)
    }

    /// Parents of `id`, nearest first (excluding `id`).
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.arena.get(id).and_then(|n| n.parent), move |&p| {
            self.arena.get(p).and_then(|n| n.parent)
        })
    }

    /// Pre-order traversal of `id` and its subtree, in paint order.
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            doc: self,
            stack: if self.contains(id) {
                vec![id]
            } else {
                Vec::new()
            },
        }
    }

    pub fn world_transform(&self, id: NodeId) -> Affine {
        let local = self.arena.get(id).map_or(Affine::IDENTITY, |n| n.transform);
        self.ancestors(id).fold(local, |t, p| {
            self.arena.get(p).map_or(t, |n| n.transform * t)
        })
    }

    /// World-space bounding box of the node's own box (no strokes/effects).
    pub fn world_bounds(&self, id: NodeId) -> Option<Rect> {
        let node = self.arena.get(id)?;
        let local = Rect::new(0.0, 0.0, node.size.w, node.size.h);
        Some(self.world_transform(id).transform_rect_bbox(local))
    }

    /// Frames directly under a page.
    pub fn is_artboard(&self, id: NodeId) -> bool {
        let Some(node) = self.arena.get(id) else {
            return false;
        };
        matches!(node.kind, NodeKind::Frame)
            && node
                .parent
                .and_then(|p| self.arena.get(p))
                .is_some_and(|p| matches!(p.kind, NodeKind::Page))
    }

    /// Drains accumulated changes, deduplicated.
    pub fn take_changes(&mut self) -> ChangeSet {
        let mut c = mem::take(&mut self.changes);
        c.nodes.sort_unstable();
        c.nodes.dedup();
        c
    }

    pub fn memory_report(&self) -> MemoryReport {
        let mut r = MemoryReport::default();
        r.push("Nodes", self.heap_size());
        r.push("Strings", self.strings.heap_bytes());
        r.push("Assets (meta)", self.assets.heap_bytes());
        r
    }
}

impl HeapSize for Document {
    /// Arena slots plus per-node heap data.
    fn heap_size(&self) -> usize {
        self.arena.heap_bytes()
            + self
                .arena
                .iter()
                .map(|(_, n)| n.heap_bytes())
                .sum::<usize>()
    }
}

fn swap_prop(node: &mut Node, value: Prop) -> Prop {
    match value {
        Prop::Name(v) => Prop::Name(mem::replace(&mut node.name, v)),
        Prop::Transform(v) => Prop::Transform(mem::replace(&mut node.transform, v)),
        Prop::Size(v) => Prop::Size(mem::replace(&mut node.size, v)),
        Prop::Opacity(v) => Prop::Opacity(mem::replace(&mut node.opacity, v.clamp(0.0, 1.0))),
        Prop::Blend(v) => Prop::Blend(mem::replace(&mut node.blend, v)),
        Prop::Flags(v) => Prop::Flags(mem::replace(&mut node.flags, v)),
        Prop::Fills(v) => Prop::Fills(mem::replace(&mut node.fills, v)),
        Prop::Ext(v) => Prop::Ext(mem::replace(&mut node.ext, v)),
        Prop::Kind(v) => Prop::Kind(mem::replace(&mut node.kind, v)),
    }
}

pub struct Descendants<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.stack.pop()?;
        if let Some(n) = self.doc.node(id) {
            self.stack.extend(n.children.iter().rev());
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::styles::{Color, Paint};

    fn named(doc: &mut Document, kind: NodeKind, name: &str) -> Node {
        Node::new(kind, doc.strings.intern(name))
    }

    #[test]
    fn build_and_traverse() {
        let mut doc = Document::new();
        let page = named(&mut doc, NodeKind::Page, "Page 1");
        let page = doc.build_child(doc.root(), page).unwrap();
        let frame = named(&mut doc, NodeKind::Frame, "Hero");
        let frame = doc
            .build_child(
                page,
                frame
                    .with_transform(Affine::translate(100.0, 50.0))
                    .with_size(400.0, 300.0),
            )
            .unwrap();
        let rect = named(&mut doc, NodeKind::Rectangle, "Bg");
        let rect = doc
            .build_child(
                frame,
                rect.with_transform(Affine::translate(10.0, 10.0))
                    .with_size(20.0, 20.0),
            )
            .unwrap();

        let order: Vec<_> = doc.descendants(doc.root()).collect();
        assert_eq!(order, vec![doc.root(), page, frame, rect]);
        assert!(doc.is_artboard(frame));
        assert!(!doc.is_artboard(rect));
        assert_eq!(
            doc.world_bounds(rect),
            Some(Rect::new(110.0, 60.0, 130.0, 80.0))
        );
        assert!(doc.is_attached(rect));
    }

    #[test]
    fn ops_invert() {
        let mut doc = Document::new();
        let page = named(&mut doc, NodeKind::Page, "P");
        let page = doc.build_child(doc.root(), page).unwrap();
        let a = named(&mut doc, NodeKind::Rectangle, "A");
        let a = doc.build_child(page, a).unwrap();

        let fills = Paints::new(vec![Paint::solid(Color::WHITE)]);
        let inv = doc
            .apply(Op::Set {
                id: a,
                value: Prop::Fills(fills.clone()),
            })
            .unwrap();
        assert_eq!(doc.node(a).unwrap().fills, fills);
        doc.apply(inv).unwrap();
        assert!(doc.node(a).unwrap().fills.is_empty());

        let inv = doc.apply(Op::Detach { id: a }).unwrap();
        assert!(!doc.is_attached(a));
        assert!(doc.contains(a), "detached nodes stay alive for undo");
        doc.apply(inv).unwrap();
        assert_eq!(doc.children(page), &[a]);
    }

    #[test]
    fn rejects_invalid_ops() {
        let mut doc = Document::new();
        let g = named(&mut doc, NodeKind::Group, "G");
        let g = doc.build_child(doc.root(), g).unwrap();
        let child = named(&mut doc, NodeKind::Group, "C");
        let child = doc.build_child(g, child).unwrap();

        doc.apply(Op::Detach { id: g }).unwrap();
        assert_eq!(
            doc.apply(Op::Attach {
                id: g,
                parent: child,
                index: 0
            }),
            Err(DocError::Cycle {
                id: g,
                parent: child
            })
        );
        assert_eq!(
            doc.apply(Op::Attach {
                id: g,
                parent: doc.root(),
                index: 5
            }),
            Err(DocError::IndexOutOfRange {
                parent: doc.root(),
                index: 5
            })
        );
        assert_eq!(
            doc.apply(Op::Detach { id: doc.root() }),
            Err(DocError::RootImmutable)
        );
        assert_eq!(
            doc.apply(Op::Attach {
                id: child,
                parent: doc.root(),
                index: 0
            }),
            Err(DocError::AlreadyAttached(child))
        );
    }

    #[test]
    fn purge_frees_detached_subtree_and_invalidates_ids() {
        let mut doc = Document::new();
        let g = named(&mut doc, NodeKind::Group, "G");
        let g = doc.build_child(doc.root(), g).unwrap();
        let c = named(&mut doc, NodeKind::Rectangle, "C");
        let c = doc.build_child(g, c).unwrap();
        assert!(!doc.purge(g), "attached nodes are never purged");
        doc.apply(Op::Detach { id: g }).unwrap();
        assert!(doc.purge(g));
        assert!(!doc.contains(g) && !doc.contains(c));

        // Slot reuse must not resurrect the stale id.
        let n = named(&mut doc, NodeKind::Rectangle, "N");
        let n = doc.create(n);
        assert!(doc.contains(n));
        assert!(!doc.contains(g) && !doc.contains(c));
    }

    #[test]
    fn change_tracking() {
        let mut doc = Document::new();
        let a = named(&mut doc, NodeKind::Rectangle, "A");
        let a = doc.build_child(doc.root(), a).unwrap();
        let _ = doc.take_changes();
        doc.apply(Op::Set {
            id: a,
            value: Prop::Opacity(0.5),
        })
        .unwrap();
        doc.apply(Op::Set {
            id: a,
            value: Prop::Opacity(0.25),
        })
        .unwrap();
        let c = doc.take_changes();
        assert_eq!(c.nodes, vec![a]);
        assert!(!c.structure);
        assert!(doc.take_changes().is_empty());
    }
}
