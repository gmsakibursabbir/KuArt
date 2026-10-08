//! Selection state and the rules that turn a pointer hit into a selected layer.
//!
//! Rules follow mainstream design tools:
//! * a click selects the top-level layer under the pointer: a direct child of an
//!   artboard, or a top-level layer on the page (group-first);
//! * Ctrl/Cmd+click selects the deepest layer under the pointer (deep select);
//! * double-click drills one level into the currently selected group/frame;
//! * locked layers (or layers inside locked ones) are not selectable by pointer;
//! * a marquee selects the layers it touches at the same level, and an artboard
//!   only when fully enclosed.

use ude_core::scene::NodeFlags;
use ude_core::{Document, NodeId, NodeKind, Rect};
use ude_spatial::{SpatialIndex, contains_point, paint_order_cmp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    /// Top-level layer within its artboard (default click).
    GroupFirst,
    /// Deepest layer under the pointer (Ctrl/Cmd+click).
    Deep,
}

/// Ordered set of selected nodes (selection order is kept for alignment tools later).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<NodeId>,
}

impl Selection {
    pub fn ids(&self) -> &[NodeId] {
        &self.ids
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }

    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.ids.clear();
        self.extend(ids);
    }

    pub fn extend(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        for id in ids {
            if !self.ids.contains(&id) {
                self.ids.push(id);
            }
        }
    }

    pub fn toggle(&mut self, id: NodeId) {
        match self.ids.iter().position(|&s| s == id) {
            Some(i) => {
                self.ids.remove(i);
            }
            None => self.ids.push(id),
        }
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Drops nodes that no longer exist or were detached (e.g. after undo).
    pub fn retain_valid(&mut self, doc: &Document) {
        self.ids
            .retain(|&id| doc.contains(id) && doc.is_attached(id));
    }
}

/// Topmost node whose exact outline contains the point (document units).
/// `tolerance` is in document units (typically a few screen pixels / zoom).
pub fn hit_test(
    doc: &Document,
    index: &SpatialIndex,
    x: f64,
    y: f64,
    tolerance: f64,
) -> Option<NodeId> {
    index
        .query_point(x as f32, y as f32, tolerance as f32)
        .filter(|&id| contains_point(doc, id, x, y, tolerance))
        .max_by(|&a, &b| paint_order_cmp(doc, a, b))
}

/// Maps a raw hit to the layer that a click selects. `None` if it is locked.
pub fn resolve_target(
    doc: &Document,
    page: NodeId,
    hit: NodeId,
    mode: SelectionMode,
) -> Option<NodeId> {
    // Path from the page's child down to the hit.
    let mut path: Vec<NodeId> = doc.ancestors(hit).take_while(|&a| a != page).collect();
    path.reverse();
    path.push(hit);
    if path.iter().any(|&id| {
        doc.node(id)
            .is_some_and(|n| n.flags.contains(NodeFlags::LOCKED))
    }) {
        return None;
    }
    let target = match mode {
        SelectionMode::Deep => hit,
        SelectionMode::GroupFirst => {
            let top = path[0];
            // Inside an artboard, select the artboard's child; otherwise the top-level layer.
            if doc.is_artboard(top) && path.len() > 1 {
                path[1]
            } else {
                top
            }
        }
    };
    Some(target)
}

/// One level deeper than `current` along the path to `hit` (double-click to enter a group).
pub fn drill(doc: &Document, current: NodeId, hit: NodeId) -> Option<NodeId> {
    let mut child = hit;
    for a in doc.ancestors(hit) {
        if a == current {
            return Some(child);
        }
        child = a;
    }
    None
}

/// Layers selected by a marquee (document-space rectangle), in paint order.
pub fn marquee_select(
    doc: &Document,
    index: &SpatialIndex,
    rect: Rect,
    mode: SelectionMode,
) -> Vec<NodeId> {
    let page = index.page();
    let mut out: Vec<NodeId> = Vec::new();
    for id in index.query_rect(rect) {
        let Some(target) = resolve_target(doc, page, id, mode) else {
            continue;
        };
        if out.contains(&target) {
            continue;
        }
        // Artboards are selected only when fully enclosed; otherwise their children are.
        if doc.is_artboard(target)
            && let Some(b) = index.bounds(target)
            && !(rect.x0 <= b.x0 && rect.y0 <= b.y0 && rect.x1 >= b.x1 && rect.y1 >= b.y1)
        {
            continue;
        }
        if matches!(
            doc.node(target).map(|n| &n.kind),
            Some(NodeKind::Page | NodeKind::Document)
        ) {
            continue;
        }
        out.push(target);
    }
    // A layer inside another selected layer is already covered by it.
    let selected = out.clone();
    out.retain(|&t| !doc.ancestors(t).any(|a| selected.contains(&a)));
    out.sort_by(|&a, &b| paint_order_cmp(doc, a, b));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Affine, Node, Symbol};

    struct Fixture {
        doc: Document,
        page: NodeId,
        artboard: NodeId,
        group: NodeId,
        inner: NodeId,
        loose: NodeId,
    }

    /// page ─ artboard(0,0 400×400) ─ group ─ inner rect (50,50 50×50)
    ///      └ loose rect (600,0 50×50)
    fn fixture() -> Fixture {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let artboard = doc
            .build_child(
                page,
                Node::new(NodeKind::Frame, Symbol::EMPTY).with_size(400.0, 400.0),
            )
            .unwrap();
        let group = doc
            .build_child(artboard, Node::new(NodeKind::Group, Symbol::EMPTY))
            .unwrap();
        let inner = doc
            .build_child(
                group,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::translate(50.0, 50.0))
                    .with_size(50.0, 50.0),
            )
            .unwrap();
        let loose = doc
            .build_child(
                page,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY)
                    .with_transform(Affine::translate(600.0, 0.0))
                    .with_size(50.0, 50.0),
            )
            .unwrap();
        Fixture {
            doc,
            page,
            artboard,
            group,
            inner,
            loose,
        }
    }

    #[test]
    fn click_rules() {
        let f = fixture();
        let idx = SpatialIndex::build(&f.doc, f.page);
        let hit = hit_test(&f.doc, &idx, 75.0, 75.0, 0.0).unwrap();
        assert_eq!(hit, f.inner, "topmost exact hit");
        assert_eq!(
            resolve_target(&f.doc, f.page, hit, SelectionMode::GroupFirst),
            Some(f.group)
        );
        assert_eq!(
            resolve_target(&f.doc, f.page, hit, SelectionMode::Deep),
            Some(f.inner)
        );
        assert_eq!(drill(&f.doc, f.group, hit), Some(f.inner));

        // Empty artboard area hits the artboard itself.
        let bg = hit_test(&f.doc, &idx, 300.0, 300.0, 0.0).unwrap();
        assert_eq!(
            resolve_target(&f.doc, f.page, bg, SelectionMode::GroupFirst),
            Some(f.artboard)
        );
        assert_eq!(hit_test(&f.doc, &idx, 500.0, 500.0, 0.0), None);
    }

    #[test]
    fn locked_layers_are_not_selectable() {
        let mut f = fixture();
        let mut flags = f.doc.node(f.group).unwrap().flags;
        flags.set(NodeFlags::LOCKED, true);
        f.doc
            .apply(ude_core::Op::Set {
                id: f.group,
                value: ude_core::Prop::Flags(flags),
            })
            .unwrap();
        let idx = SpatialIndex::build(&f.doc, f.page);
        let hit = hit_test(&f.doc, &idx, 75.0, 75.0, 0.0).unwrap();
        assert_eq!(
            resolve_target(&f.doc, f.page, hit, SelectionMode::GroupFirst),
            None
        );
    }

    #[test]
    fn marquee_rules() {
        let f = fixture();
        let idx = SpatialIndex::build(&f.doc, f.page);
        // Inside the artboard: selects the group, not the partially covered artboard.
        let inside = marquee_select(
            &f.doc,
            &idx,
            Rect::new(40.0, 40.0, 60.0, 60.0),
            SelectionMode::GroupFirst,
        );
        assert_eq!(inside, vec![f.group]);
        // Enclosing everything: the artboard (which covers its group) and the loose rect.
        let all = marquee_select(
            &f.doc,
            &idx,
            Rect::new(-10.0, -10.0, 700.0, 500.0),
            SelectionMode::GroupFirst,
        );
        assert_eq!(all, vec![f.artboard, f.loose]);
    }

    #[test]
    fn selection_set_semantics() {
        let f = fixture();
        let mut s = Selection::default();
        s.set([f.inner, f.inner, f.loose]);
        assert_eq!(s.ids(), &[f.inner, f.loose]);
        s.toggle(f.inner);
        assert_eq!(s.ids(), &[f.loose]);
        s.toggle(f.group);
        assert_eq!(s.ids(), &[f.loose, f.group]);
    }
}
