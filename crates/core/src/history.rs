//! Undo/redo built from inverse ops, with gesture coalescing and a byte budget.
//! Never snapshots the document.

use std::collections::{HashSet, VecDeque};

use crate::document::{DocError, Document, Op};

/// Ops applied as one user action. Collects inverses as it goes.
pub struct Transaction {
    label: Box<str>,
    inverse: Vec<Op>,
    merge_key: Option<u64>,
}

impl Transaction {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            inverse: Vec::new(),
            merge_key: None,
        }
    }

    /// Consecutive transactions with the same key (e.g. one drag gesture) merge
    /// into a single undo step, provided they only set properties.
    pub fn with_merge_key(mut self, key: u64) -> Self {
        self.merge_key = Some(key);
        self
    }

    pub fn apply(&mut self, doc: &mut Document, op: Op) -> Result<(), DocError> {
        let inv = doc.apply(op)?;
        self.inverse.push(inv);
        Ok(())
    }

    /// Reverts everything applied so far (e.g. after a failed op).
    pub fn rollback(self, doc: &mut Document) -> Result<(), DocError> {
        for op in self.inverse.into_iter().rev() {
            doc.apply(op)?;
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.inverse.is_empty()
    }
}

struct Entry {
    label: Box<str>,
    /// Ops to apply, in order, to perform this step.
    ops: Vec<Op>,
    merge_key: Option<u64>,
    bytes: usize,
}

impl Entry {
    fn new(label: Box<str>, ops: Vec<Op>, merge_key: Option<u64>) -> Self {
        let bytes = size_of::<Entry>() + label.len() + ops.iter().map(Op::heap_size).sum::<usize>();
        Self {
            label,
            ops,
            merge_key,
            bytes,
        }
    }

    fn only_sets(&self) -> bool {
        self.ops.iter().all(|op| matches!(op, Op::Set { .. }))
    }
}

pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    budget_bytes: usize,
    used_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::with_budget(32 * 1024 * 1024)
    }
}

impl History {
    pub fn with_budget(budget_bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            budget_bytes,
            used_bytes: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| &*e.label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| &*e.label)
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn commit(&mut self, doc: &mut Document, tx: Transaction) {
        if tx.is_empty() {
            return;
        }
        for entry in std::mem::take(&mut self.redo) {
            self.used_bytes -= entry.bytes;
            purge_unreachable(doc, &entry);
        }

        let mut ops = tx.inverse;
        ops.reverse();
        let incoming = Entry::new(tx.label, ops, tx.merge_key);

        // Incoming property sets may join an entry that also attached nodes (create + drag):
        // undo runs the newer sets first, then the older structural ops, which is the right order.
        let mergeable = incoming.merge_key.is_some()
            && incoming.only_sets()
            && self
                .undo
                .back()
                .is_some_and(|top| top.merge_key == incoming.merge_key);
        if mergeable && let Some(top) = self.undo.back_mut() {
            // The older entry already restores the earliest value of each (node, prop).
            let known: HashSet<_> = top.ops.iter().filter_map(set_key).collect();
            let mut merged: Vec<Op> = incoming
                .ops
                .into_iter()
                .filter(|op| set_key(op).is_none_or(|k| !known.contains(&k)))
                .collect();
            merged.append(&mut top.ops);
            self.used_bytes -= top.bytes;
            *top = Entry::new(std::mem::take(&mut top.label), merged, top.merge_key);
            self.used_bytes += top.bytes;
        } else {
            self.used_bytes += incoming.bytes;
            self.undo.push_back(incoming);
        }
        self.enforce_budget(doc);
    }

    pub fn undo(&mut self, doc: &mut Document) -> Result<bool, DocError> {
        let Some(entry) = self.undo.pop_back() else {
            return Ok(false);
        };
        self.used_bytes -= entry.bytes;
        let redo = run(doc, entry)?;
        self.used_bytes += redo.bytes;
        self.redo.push(redo);
        Ok(true)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Result<bool, DocError> {
        let Some(entry) = self.redo.pop() else {
            return Ok(false);
        };
        self.used_bytes -= entry.bytes;
        let undo = run(doc, entry)?;
        self.used_bytes += undo.bytes;
        self.undo.push_back(undo);
        Ok(true)
    }

    /// Drops all history and frees nodes that only history kept alive.
    pub fn clear(&mut self, doc: &mut Document) {
        for e in self.undo.drain(..).chain(self.redo.drain(..)) {
            purge_unreachable(doc, &e);
        }
        self.used_bytes = 0;
    }

    fn enforce_budget(&mut self, doc: &mut Document) {
        while self.used_bytes > self.budget_bytes && self.undo.len() > 1 {
            if let Some(old) = self.undo.pop_front() {
                self.used_bytes -= old.bytes;
                purge_unreachable(doc, &old);
            }
        }
    }
}

/// Applies an entry's ops; returns the entry that reverses it.
fn run(doc: &mut Document, entry: Entry) -> Result<Entry, DocError> {
    let mut inverse = Vec::with_capacity(entry.ops.len());
    for op in entry.ops {
        inverse.push(doc.apply(op)?);
    }
    inverse.reverse();
    Ok(Entry::new(entry.label, inverse, entry.merge_key))
}

fn set_key(op: &Op) -> Option<(crate::NodeId, u8)> {
    match op {
        Op::Set { id, value } => Some((*id, value.key())),
        _ => None,
    }
}

/// A dropped entry was the last way to re-attach the nodes it moves; free those still detached.
fn purge_unreachable(doc: &mut Document, entry: &Entry) {
    for op in &entry.ops {
        if let Op::Attach { id, .. } | Op::Detach { id } = op
            && doc.node(*id).is_some_and(|n| n.parent.is_none())
        {
            doc.purge(*id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Prop;
    use crate::geometry::Affine;
    use crate::scene::{Node, NodeKind};
    use crate::{NodeId, Symbol};

    fn doc_with_rect() -> (Document, NodeId) {
        let mut doc = Document::new();
        let r = doc
            .build_child(doc.root(), Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        (doc, r)
    }

    fn move_to(doc: &mut Document, h: &mut History, id: NodeId, x: f32, key: Option<u64>) {
        let mut tx = Transaction::new("Move");
        if let Some(k) = key {
            tx = tx.with_merge_key(k);
        }
        tx.apply(
            doc,
            Op::Set {
                id,
                value: Prop::Transform(Affine::translate(x, 0.0)),
            },
        )
        .unwrap();
        h.commit(doc, tx);
    }

    #[test]
    fn undo_redo_roundtrip() {
        let (mut doc, r) = doc_with_rect();
        let mut h = History::default();
        move_to(&mut doc, &mut h, r, 10.0, None);
        move_to(&mut doc, &mut h, r, 20.0, None);
        assert_eq!(h.len(), 2);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.node(r).unwrap().transform, Affine::translate(10.0, 0.0));
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.node(r).unwrap().transform, Affine::IDENTITY);
        assert!(!h.undo(&mut doc).unwrap());
        h.redo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.node(r).unwrap().transform, Affine::translate(20.0, 0.0));
    }

    #[test]
    fn drag_coalesces_into_one_step() {
        let (mut doc, r) = doc_with_rect();
        let mut h = History::default();
        for x in 1..=100 {
            move_to(&mut doc, &mut h, r, x as f32, Some(42));
        }
        assert_eq!(h.len(), 1);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.node(r).unwrap().transform, Affine::IDENTITY);
        h.redo(&mut doc).unwrap();
        assert_eq!(
            doc.node(r).unwrap().transform,
            Affine::translate(100.0, 0.0)
        );
    }

    #[test]
    fn delete_undo_and_purge_after_new_edit() {
        let (mut doc, r) = doc_with_rect();
        let mut h = History::default();

        // Create a node through history, undo it, then make a new edit: the node is purged.
        let created = doc.create(Node::new(NodeKind::Ellipse(None), Symbol::EMPTY));
        let mut tx = Transaction::new("Add ellipse");
        let root = doc.root();
        tx.apply(
            &mut doc,
            Op::Attach {
                id: created,
                parent: root,
                index: 1,
            },
        )
        .unwrap();
        h.commit(&mut doc, tx);
        h.undo(&mut doc).unwrap();
        assert!(doc.contains(created));
        move_to(&mut doc, &mut h, r, 5.0, None);
        assert!(!doc.contains(created), "redo branch dropped, orphan freed");

        // Delete then undo restores the same id at the same index.
        let mut tx = Transaction::new("Delete");
        tx.apply(&mut doc, Op::Detach { id: r }).unwrap();
        h.commit(&mut doc, tx);
        assert!(!doc.is_attached(r));
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.children(doc.root()), &[r]);
    }

    #[test]
    fn budget_trims_oldest_and_purges() {
        let mut doc = Document::new();
        let mut h = History::with_budget(1);
        let a = doc
            .build_child(doc.root(), Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        let mut tx = Transaction::new("Delete");
        tx.apply(&mut doc, Op::Detach { id: a }).unwrap();
        h.commit(&mut doc, tx);
        assert!(doc.contains(a), "newest entry is always kept");
        let b = doc
            .build_child(doc.root(), Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        move_to(&mut doc, &mut h, b, 1.0, None);
        assert_eq!(h.len(), 1);
        assert!(
            !doc.contains(a),
            "trimmed delete can no longer be undone, node freed"
        );
    }

    #[test]
    fn rollback_restores_state() {
        let (mut doc, r) = doc_with_rect();
        let mut tx = Transaction::new("Batch");
        tx.apply(
            &mut doc,
            Op::Set {
                id: r,
                value: Prop::Opacity(0.3),
            },
        )
        .unwrap();
        let root = doc.root();
        assert!(tx.apply(&mut doc, Op::Detach { id: root }).is_err());
        tx.rollback(&mut doc).unwrap();
        assert_eq!(doc.node(r).unwrap().opacity, 1.0);
    }
}
