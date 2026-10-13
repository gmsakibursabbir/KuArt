//! Components: create a component, place instances, detach them, and keep instances in sync
//! with their main component.
//!
//! Sync updates instance layers in place, matched by position in the tree (it never deletes or
//! rebuilds them, so imported variant instances stay intact). Edits made inside an instance
//! are recorded as overrides — properties that differ from the main — and survive later syncs.
//! Our overrides use paths of child indices written as "#i"; imported overrides (source ids) are
//! kept as they are.

use ude_core::scene::{ComponentInfo, ComponentRef, InstanceData, Override};
use ude_core::{
    Affine, DocError, Document, History, Node, NodeId, NodeKind, Op, Prop, Symbol, Transaction,
};

use crate::commands::{insert, top_level, tree};

/// The properties sync copies from a main component's layers (and overrides can replace).
fn props(n: &Node, root: bool) -> Vec<Prop> {
    let mut v = vec![
        Prop::Name(n.name),
        Prop::Size(n.size),
        Prop::Opacity(n.opacity),
        Prop::Blend(n.blend),
        Prop::Flags(n.flags),
        Prop::Fills(n.fills.clone()),
        Prop::Ext(n.ext.clone()),
        Prop::CornerRadius(n.corner_radius),
    ];
    // The instance itself keeps its own name, position and kind.
    if !root {
        v.push(Prop::Transform(n.transform));
        v.push(Prop::Kind(n.kind.clone()));
    }
    v
}

fn main_of(n: &Node) -> Option<NodeId> {
    match &n.kind {
        NodeKind::Instance(d) => match d.component {
            ComponentRef::Local(m) => Some(m),
            ComponentRef::External(_) => None,
        },
        _ => None,
    }
}

/// The node at `path` (child indices) below `root`.
fn at_path(doc: &Document, root: NodeId, path: &[usize]) -> Option<NodeId> {
    path.iter()
        .try_fold(root, |id, &i| doc.children(id).get(i).copied())
}

fn index_path(doc: &Document, ancestor: NodeId, id: NodeId) -> Option<Vec<usize>> {
    let mut path = Vec::new();
    let mut cur = id;
    while cur != ancestor {
        let parent = doc.node(cur)?.parent?;
        path.push(doc.children(parent).iter().position(|&c| c == cur)?);
        cur = parent;
    }
    path.reverse();
    Some(path)
}

fn path_symbols(doc: &mut Document, path: &[usize]) -> Box<[Symbol]> {
    path.iter()
        .map(|i| doc.strings.intern(&format!("#{i}")))
        .collect()
}

fn parse_path(doc: &Document, path: &[Symbol]) -> Option<Vec<usize>> {
    path.iter()
        .map(|s| doc.strings.resolve(*s).strip_prefix('#')?.parse().ok())
        .collect()
}

/// Nearest instance ancestor of `id` (not `id` itself).
fn instance_above(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.ancestors(id)
        .find(|&a| doc.node(a).is_some_and(|n| main_of(n).is_some()))
}

/// The main component `id` belongs to (itself or an ancestor).
fn component_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    std::iter::once(id).chain(doc.ancestors(id)).find(|&a| {
        doc.node(a)
            .is_some_and(|n| matches!(n.kind, NodeKind::Component(_)))
    })
}

/// Records the edited layers inside instances as overrides, then brings the instances of every
/// edited main component up to date. Returns the changes for the caller to commit or merge.
pub fn sync(doc: &mut Document, edited: &[NodeId]) -> Result<Transaction, DocError> {
    let mut tx = Transaction::new("Update instances");
    // 1. Overrides: what an edited layer inside an instance does differently from the main.
    for &id in edited {
        let Some(inst) = instance_above(doc, id) else {
            continue;
        };
        let Some(main) = doc.node(inst).and_then(main_of) else {
            continue;
        };
        let Some(path) = index_path(doc, inst, id) else {
            continue;
        };
        let Some(counterpart) = at_path(doc, main, &path) else {
            continue;
        };
        let (Some(n), Some(m)) = (doc.node(id), doc.node(counterpart)) else {
            continue;
        };
        let changed: Vec<Prop> = props(n, false)
            .into_iter()
            .zip(props(m, false))
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a)
            .collect();
        let keys: Vec<u8> = props(n, false).iter().map(Prop::key).collect();
        let symbols = path_symbols(doc, &path);
        let Some(NodeKind::Instance(data)) = doc.node(inst).map(|n| n.kind.clone()) else {
            continue;
        };
        let mut data = (*data).clone();
        let mut list: Vec<Override> = data
            .overrides
            .iter()
            .filter(|o| !(o.path == symbols && keys.contains(&o.prop.key())))
            .cloned()
            .collect();
        list.extend(changed.into_iter().map(|prop| Override {
            path: symbols.clone(),
            prop,
        }));
        if list.as_slice() != &*data.overrides {
            data.overrides = list.into();
            tx.apply(
                doc,
                Op::Set {
                    id: inst,
                    value: Prop::Kind(NodeKind::Instance(Box::new(data))),
                },
            )?;
        }
    }
    // 2. Instances of edited mains follow them.
    let mut mains: Vec<NodeId> = edited
        .iter()
        .filter_map(|&id| component_of(doc, id))
        .collect();
    mains.sort_unstable();
    mains.dedup();
    if mains.is_empty() {
        return Ok(tx);
    }
    let instances: Vec<NodeId> = doc
        .descendants(doc.root())
        .filter(|&id| {
            doc.node(id)
                .and_then(main_of)
                .is_some_and(|m| mains.contains(&m))
        })
        .collect();
    for inst in instances {
        let Some(main) = doc.node(inst).and_then(main_of) else {
            continue;
        };
        let overrides: Vec<(Vec<usize>, Prop)> = match doc.node(inst).map(|n| &n.kind) {
            Some(NodeKind::Instance(d)) => d
                .overrides
                .iter()
                .filter_map(|o| Some((parse_path(doc, &o.path)?, o.prop.clone())))
                .collect(),
            _ => Vec::new(),
        };
        copy_into(doc, &mut tx, main, inst, &mut Vec::new(), &overrides)?;
    }
    Ok(tx)
}

/// Records how every instance currently differs from its main as overrides. Run when a file
/// is opened (imported instances carry differences, such as their own labels, that were never
/// recorded), so editing the main later cannot wipe them. Applied directly, without history.
pub fn capture_baselines(doc: &mut Document) {
    let instances: Vec<(NodeId, NodeId)> = doc
        .descendants(doc.root())
        .filter_map(|id| Some((id, doc.node(id).and_then(main_of)?)))
        .collect();
    for (inst, main) in instances {
        let mut found = Vec::new();
        diffs(doc, main, inst, &mut Vec::new(), &mut found);
        if found.is_empty() {
            continue;
        }
        let Some(NodeKind::Instance(data)) = doc.node(inst).map(|n| n.kind.clone()) else {
            continue;
        };
        let mut data = (*data).clone();
        let mut list = data.overrides.to_vec();
        for (path, prop) in found {
            let symbols = path_symbols(doc, &path);
            if !list
                .iter()
                .any(|o| o.path == symbols && o.prop.key() == prop.key())
            {
                list.push(Override {
                    path: symbols,
                    prop,
                });
            }
        }
        data.overrides = list.into();
        let _ = doc.apply(Op::Set {
            id: inst,
            value: Prop::Kind(NodeKind::Instance(Box::new(data))),
        });
    }
}

fn diffs(
    doc: &Document,
    from: NodeId,
    to: NodeId,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, Prop)>,
) {
    let (Some(f), Some(t)) = (doc.node(from), doc.node(to)) else {
        return;
    };
    if !path.is_empty() {
        for (w, h) in props(f, false).into_iter().zip(props(t, false)) {
            if w != h {
                out.push((path.clone(), h));
            }
        }
    }
    let (fc, tc) = (doc.children(from), doc.children(to));
    if fc.len() == tc.len() {
        for (i, (&a, &b)) in fc.iter().zip(tc).enumerate() {
            path.push(i);
            diffs(doc, a, b, path, out);
            path.pop();
        }
    }
}

/// Sets `to` (and below) to `from`'s properties, except overridden ones; stops where the
/// structures differ.
fn copy_into(
    doc: &mut Document,
    tx: &mut Transaction,
    from: NodeId,
    to: NodeId,
    path: &mut Vec<usize>,
    overrides: &[(Vec<usize>, Prop)],
) -> Result<(), DocError> {
    let root = path.is_empty();
    let (Some(f), Some(t)) = (doc.node(from), doc.node(to)) else {
        return Ok(());
    };
    let wanted = props(f, root);
    let have = props(t, root);
    for (w, h) in wanted.into_iter().zip(have) {
        let value = overrides
            .iter()
            .find(|(p, o)| p == path && o.key() == w.key())
            .map_or(w, |(_, o)| o.clone());
        if value != h {
            tx.apply(doc, Op::Set { id: to, value })?;
        }
    }
    let (fc, tc) = (doc.children(from).to_vec(), doc.children(to).to_vec());
    if fc.len() == tc.len() {
        for (i, (a, b)) in fc.into_iter().zip(tc).enumerate() {
            path.push(i);
            copy_into(doc, tx, a, b, path, overrides)?;
            path.pop();
        }
    }
    Ok(())
}

/// Makes a component: a frame becomes one; other layers are wrapped in a new component.
pub fn create_component(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Option<NodeId>, DocError> {
    let ids = top_level(doc, ids);
    if let [id] = ids.as_slice()
        && matches!(doc.node(*id).map(|n| &n.kind), Some(NodeKind::Frame))
    {
        let mut tx = Transaction::new("Create component");
        let kind = component_kind();
        tx.apply(
            doc,
            Op::Set {
                id: *id,
                value: Prop::Kind(kind),
            },
        )?;
        history.commit(doc, tx);
        return Ok(Some(*id));
    }
    let kind = component_kind();
    crate::commands::group_as(doc, history, &ids, kind, "Create component")
}

fn component_kind() -> NodeKind {
    NodeKind::Component(Box::new(ComponentInfo {
        key: Symbol::EMPTY,
        description: Symbol::EMPTY,
        variant_props: Box::new([]),
    }))
}

/// Places an instance of the main component `main` to its right. Returns the instance.
pub fn create_instance(
    doc: &mut Document,
    history: &mut History,
    main: NodeId,
) -> Result<Option<NodeId>, DocError> {
    let Some(m) = doc.node(main) else {
        return Ok(None);
    };
    if !matches!(m.kind, NodeKind::Component(_)) {
        return Ok(None);
    }
    let Some(parent) = m.parent else {
        return Ok(None);
    };
    let Some(mut t) = tree(doc, main) else {
        return Ok(None);
    };
    t.node.kind = NodeKind::Instance(Box::new(InstanceData {
        component: ComponentRef::Local(main),
        overrides: Box::new([]),
    }));
    t.node.transform = Affine::translate(m.size.w + 40.0, 0.0) * m.transform;
    let index = doc.children(parent).len() as u32;
    let mut tx = Transaction::new("Create instance");
    let id = match insert(doc, &mut tx, &t, parent, index) {
        Ok(id) => id,
        Err(e) => {
            tx.rollback(doc)?;
            return Err(e);
        }
    };
    history.commit(doc, tx);
    Ok(Some(id))
}

/// Turns instances into plain frames (they stop following their component).
pub fn detach(
    doc: &mut Document,
    history: &mut History,
    ids: &[NodeId],
) -> Result<Vec<NodeId>, DocError> {
    let mut tx = Transaction::new("Detach instance");
    let mut out = Vec::new();
    for &id in ids {
        if doc
            .node(id)
            .is_some_and(|n| matches!(n.kind, NodeKind::Instance(_)))
        {
            tx.apply(
                doc,
                Op::Set {
                    id,
                    value: Prop::Kind(NodeKind::Frame),
                },
            )?;
            out.push(id);
        }
    }
    history.commit(doc, tx);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::styles::{Color, Paint};
    use ude_core::{Paints, Symbol};

    fn red() -> Paints {
        Paints::new(vec![Paint::solid(Color::from_rgba8(255, 0, 0, 255))])
    }

    #[test]
    fn instances_follow_the_main_and_keep_their_overrides() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let frame = doc
            .build_child(
                page,
                Node::new(NodeKind::Frame, Symbol::EMPTY).with_size(100.0, 50.0),
            )
            .unwrap();
        let label = doc
            .build_child(
                frame,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(10.0, 10.0),
            )
            .unwrap();
        let dot = doc
            .build_child(
                frame,
                Node::new(NodeKind::Ellipse(None), Symbol::EMPTY).with_size(5.0, 5.0),
            )
            .unwrap();
        let mut h = History::default();
        let main = create_component(&mut doc, &mut h, &[frame])
            .unwrap()
            .unwrap();
        assert_eq!(main, frame);
        let inst = create_instance(&mut doc, &mut h, main).unwrap().unwrap();
        assert_eq!(doc.node(inst).unwrap().transform.translation().x, 140.0);
        let (i_label, i_dot) = (doc.children(inst)[0], doc.children(inst)[1]);

        // Override inside the instance: the dot turns red there only.
        let mut tx = Transaction::new("edit");
        tx.apply(
            &mut doc,
            Op::Set {
                id: i_dot,
                value: Prop::Fills(red()),
            },
        )
        .unwrap();
        h.commit(&mut doc, tx);
        let tx = sync(&mut doc, &[i_dot]).unwrap();
        h.amend(&mut doc, tx);

        // Edit the main: both layers change size; the instance follows, keeping the red dot.
        let mut tx = Transaction::new("edit main");
        tx.apply(
            &mut doc,
            Op::Set {
                id: label,
                value: Prop::Size(ude_core::Size::new(30.0, 30.0)),
            },
        )
        .unwrap();
        tx.apply(
            &mut doc,
            Op::Set {
                id: dot,
                value: Prop::Size(ude_core::Size::new(8.0, 8.0)),
            },
        )
        .unwrap();
        h.commit(&mut doc, tx);
        let tx = sync(&mut doc, &[label, dot]).unwrap();
        h.amend(&mut doc, tx);
        assert_eq!(doc.node(i_label).unwrap().size.w, 30.0);
        assert_eq!(doc.node(i_dot).unwrap().size.w, 8.0);
        assert_eq!(doc.node(i_dot).unwrap().fills, red(), "override kept");
        assert!(doc.node(dot).unwrap().fills.is_empty(), "main unchanged");

        // One undo reverts the main edit and the instance update together.
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.node(i_label).unwrap().size.w, 10.0);

        let d = detach(&mut doc, &mut h, &[inst]).unwrap();
        assert!(matches!(doc.node(d[0]).unwrap().kind, NodeKind::Frame));
    }

    #[test]
    fn imported_instance_differences_survive_main_edits() {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let mut h = History::default();
        let frame = doc
            .build_child(page, Node::new(NodeKind::Frame, Symbol::EMPTY))
            .unwrap();
        let shape = doc
            .build_child(
                frame,
                Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(10.0, 10.0),
            )
            .unwrap();
        let main = create_component(&mut doc, &mut h, &[frame])
            .unwrap()
            .unwrap();
        let inst = create_instance(&mut doc, &mut h, main).unwrap().unwrap();
        // As imported: the instance's shape is red, with no override recorded.
        let i_shape = doc.children(inst)[0];
        doc.apply(Op::Set {
            id: i_shape,
            value: Prop::Fills(red()),
        })
        .unwrap();
        capture_baselines(&mut doc);
        let mut tx = Transaction::new("edit main");
        tx.apply(
            &mut doc,
            Op::Set {
                id: shape,
                value: Prop::Size(ude_core::Size::new(20.0, 20.0)),
            },
        )
        .unwrap();
        h.commit(&mut doc, tx);
        let tx = sync(&mut doc, &[shape]).unwrap();
        h.amend(&mut doc, tx);
        let n = doc.node(i_shape).unwrap();
        assert_eq!((n.size.w, n.fills.clone()), (20.0, red()));
    }
}
