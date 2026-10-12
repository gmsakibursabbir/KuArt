//! Auto layout solver (Figma semantics): measure hug sizes bottom-up, then arrange top-down
//! (fill sizes, gaps, padding, alignment, wrapping, min/max limits). Pure: it returns the
//! property changes as a transaction for the caller to commit or merge.

use std::collections::HashMap;

use ude_core::layout::{
    AutoLayout, ChildSizing, ContainerSizing, CrossAlign, LayoutDirection, MainAlign,
};
use ude_core::{Affine, DocError, Document, Node, NodeId, Op, Prop, Size, Transaction};

const EPS: f32 = 0.01;

fn auto_layout(n: &Node) -> Option<&AutoLayout> {
    n.ext.as_ref()?.auto_layout.as_deref()
}

fn sizing(n: &Node) -> (ChildSizing, ChildSizing) {
    n.ext
        .as_ref()
        .map_or((ChildSizing::Fixed, ChildSizing::Fixed), |e| {
            (e.layout_child.h_sizing, e.layout_child.v_sizing)
        })
}

fn clamp(n: &Node, w: f32, h: f32) -> (f32, f32) {
    let Some(l) = n
        .ext
        .as_ref()
        .and_then(|e| e.layout_child.limits.as_deref())
    else {
        return (w.max(0.0), h.max(0.0));
    };
    let c = |v: f32, i: usize| {
        let v = l.min[i].map_or(v, |m| v.max(m));
        l.max[i].map_or(v, |m| v.min(m)).max(0.0)
    };
    (c(w, 0), c(h, 1))
}

/// Children that take part in the layout.
fn flow(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.children(id)
        .iter()
        .copied()
        .filter(|&c| {
            doc.node(c).is_some_and(|n| {
                n.is_visible() && !n.ext.as_ref().is_some_and(|e| e.layout_child.absolute)
            })
        })
        .collect()
}

struct Solver<'a> {
    doc: &'a Document,
    measured: HashMap<NodeId, (f32, f32)>,
    out: Vec<(NodeId, Prop)>,
}

impl Solver<'_> {
    /// Natural size of a node: hug sizes of auto layout frames, the box of anything else.
    fn measure(&mut self, id: NodeId) -> (f32, f32) {
        if let Some(&m) = self.measured.get(&id) {
            return m;
        }
        let Some(n) = self.doc.node(id) else {
            return (0.0, 0.0);
        };
        let own = (n.size.w, n.size.h);
        let m = match auto_layout(n) {
            None => own,
            Some(al) => {
                let al = *al;
                let kids = flow(self.doc, id);
                let sizes: Vec<(f32, f32)> = kids.iter().map(|&c| self.measure(c)).collect();
                let horizontal = al.direction != LayoutDirection::Vertical;
                let (main_pad, cross_pad) = pads(&al, horizontal);
                let main: f32 = sizes
                    .iter()
                    .map(|s| if horizontal { s.0 } else { s.1 })
                    .sum::<f32>()
                    + al.gap * kids.len().saturating_sub(1) as f32
                    + main_pad;
                let cross = sizes
                    .iter()
                    .map(|s| if horizontal { s.1 } else { s.0 })
                    .fold(0.0, f32::max)
                    + cross_pad;
                let hug = |s: ContainerSizing, natural: f32, fixed: f32| {
                    if s == ContainerSizing::Hug {
                        natural
                    } else {
                        fixed
                    }
                };
                // Wrapping only hugs the height; its width stays as set.
                let (main_size, cross_size) = if horizontal {
                    (own.0, own.1)
                } else {
                    (own.1, own.0)
                };
                let main = if al.direction == LayoutDirection::Wrap {
                    main_size
                } else {
                    hug(al.primary_sizing, main, main_size)
                };
                let cross = hug(al.counter_sizing, cross, cross_size);
                if horizontal {
                    (main, cross)
                } else {
                    (cross, main)
                }
            }
        };
        let m = clamp(n, m.0, m.1);
        self.measured.insert(id, m);
        m
    }

    fn set(&mut self, id: NodeId, n: &Node, size: (f32, f32), pos: Option<(f32, f32)>) {
        if (n.size.w - size.0).abs() > EPS || (n.size.h - size.1).abs() > EPS {
            self.out.push((id, Prop::Size(Size::new(size.0, size.1))));
        }
        if let Some((x, y)) = pos {
            let t = n.transform.translation();
            if (t.x - x).abs() > EPS || (t.y - y).abs() > EPS {
                let mut a = n.transform;
                a.0[4] = x;
                a.0[5] = y;
                self.out.push((id, Prop::Transform(a)));
            }
        }
    }

    /// Lays out `id` at `size` and everything below it.
    fn arrange(&mut self, id: NodeId, size: (f32, f32)) {
        let doc = self.doc;
        let Some(n) = doc.node(id) else { return };
        let Some(al) = auto_layout(n).copied() else {
            return;
        };
        let horizontal = al.direction != LayoutDirection::Vertical;
        let [pt, pr, pb, pl] = al.padding;
        let (inner_w, inner_h) = ((size.0 - pl - pr).max(0.0), (size.1 - pt - pb).max(0.0));
        let (inner_main, inner_cross) = if horizontal {
            (inner_w, inner_h)
        } else {
            (inner_h, inner_w)
        };
        let kids = flow(doc, id);

        // Child sizes along (main, cross); fill shares what fixed and hug children leave.
        let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(kids.len());
        let mut fills = 0usize;
        for &c in &kids {
            let Some(cn) = doc.node(c) else { continue };
            let (hs, vs) = sizing(cn);
            let (ms, cs) = if horizontal { (hs, vs) } else { (vs, hs) };
            let natural = self.measure(c);
            let (nm, nc) = if horizontal {
                natural
            } else {
                (natural.1, natural.0)
            };
            let main = if ms == ChildSizing::Fill { 0.0 } else { nm };
            fills += usize::from(ms == ChildSizing::Fill);
            let cross = if cs == ChildSizing::Fill || al.cross_align == CrossAlign::Stretch {
                inner_cross
            } else {
                nc
            };
            sizes.push((main, cross));
        }
        let wrap = al.direction == LayoutDirection::Wrap;
        if fills > 0 && !wrap {
            let used: f32 = sizes.iter().map(|s| s.0).sum::<f32>()
                + al.gap * kids.len().saturating_sub(1) as f32;
            let share = ((inner_main - used) / fills as f32).max(0.0);
            for (s, &c) in sizes.iter_mut().zip(&kids) {
                let Some(cn) = doc.node(c) else { continue };
                let (hs, vs) = sizing(cn);
                if (if horizontal { hs } else { vs }) == ChildSizing::Fill {
                    s.0 = share;
                }
            }
        }
        for (s, &c) in sizes.iter_mut().zip(&kids) {
            if let Some(cn) = doc.node(c) {
                let (w, h) = if horizontal { (s.0, s.1) } else { (s.1, s.0) };
                let (w, h) = clamp(cn, w, h);
                *s = if horizontal { (w, h) } else { (h, w) };
            }
        }

        // Lines: one, or several when wrapping.
        let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
        let mut start = 0;
        let mut run = 0.0;
        for (i, s) in sizes.iter().enumerate() {
            let next = if i == start { s.0 } else { run + al.gap + s.0 };
            if wrap && i > start && next > inner_main + EPS {
                lines.push(start..i);
                start = i;
                run = s.0;
            } else {
                run = next;
            }
        }
        lines.push(start..sizes.len());

        let mut cross_at = 0.0;
        let mut final_size = size;
        for line in lines {
            let items = &sizes[line.clone()];
            let total: f32 = items.iter().map(|s| s.0).sum();
            let count = items.len();
            let gaps = al.gap * count.saturating_sub(1) as f32;
            let free = inner_main - total - gaps;
            let (mut at, gap) = match al.main_align {
                MainAlign::Start => (0.0, al.gap),
                MainAlign::Center => (free / 2.0, al.gap),
                MainAlign::End => (free, al.gap),
                MainAlign::SpaceBetween if count > 1 => {
                    (0.0, (inner_main - total) / (count - 1) as f32)
                }
                MainAlign::SpaceBetween => (0.0, al.gap),
            };
            let line_cross = if wrap {
                items.iter().map(|s| s.1).fold(0.0, f32::max)
            } else {
                inner_cross
            };
            for (k, &(m, c)) in items.iter().enumerate() {
                let i = line.start + k;
                let cross = cross_at
                    + match al.cross_align {
                        CrossAlign::Center => (line_cross - c) / 2.0,
                        CrossAlign::End => line_cross - c,
                        _ => 0.0,
                    };
                let (x, y) = if horizontal {
                    (pl + at, pt + cross)
                } else {
                    (pl + cross, pt + at)
                };
                let (w, h) = if horizontal { (m, c) } else { (c, m) };
                let id = kids[i];
                if let Some(cn) = doc.node(id) {
                    self.set(id, cn, (w, h), Some((x, y)));
                }
                self.arrange(id, (w, h));
                at += m + gap;
            }
            cross_at += line_cross + al.cross_gap;
        }
        if wrap && al.counter_sizing == ContainerSizing::Hug {
            let h = (cross_at - al.cross_gap).max(0.0) + pt + pb;
            final_size.1 = h;
        }
        self.set(id, n, final_size, None);
    }
}

fn pads(al: &AutoLayout, horizontal: bool) -> (f32, f32) {
    let [t, r, b, l] = al.padding;
    if horizontal {
        (l + r, t + b)
    } else {
        (t + b, l + r)
    }
}

/// The auto layout frames to re-solve after editing `ids`: the outermost auto layout frame
/// around each (or the layer itself), so the whole affected tree is laid out once.
pub fn roots_for(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut roots: Vec<NodeId> = Vec::new();
    for &id in ids {
        let mut top = None;
        let mut cur = Some(id);
        while let Some(c) = cur {
            if doc.node(c).is_some_and(|n| auto_layout(n).is_some()) {
                top = Some(c);
            }
            cur = doc.node(c).and_then(|n| n.parent);
        }
        if let Some(t) = top
            && !roots.contains(&t)
        {
            roots.push(t);
        }
    }
    roots
}

/// Solves the auto layout under `roots`; returns the changes (empty when nothing moves).
pub fn reflow(doc: &mut Document, roots: &[NodeId]) -> Result<Transaction, DocError> {
    let mut s = Solver {
        doc,
        measured: HashMap::new(),
        out: Vec::new(),
    };
    for &r in roots {
        let size = s.measure(r);
        if let Some(n) = s.doc.node(r) {
            s.set(r, n, size, None);
        }
        s.arrange(r, size);
    }
    let out = s.out;
    let mut tx = Transaction::new("Auto layout");
    for (id, value) in out {
        tx.apply(doc, Op::Set { id, value })?;
    }
    Ok(tx)
}

/// Turns auto layout on for a frame with sensible defaults, keeping the children's order by
/// position (left to right, or top to bottom).
pub fn default_auto_layout(doc: &Document, id: NodeId) -> AutoLayout {
    let kids = flow(doc, id);
    let pos = |c: NodeId| {
        doc.node(c)
            .map_or(Affine::IDENTITY, |n| n.transform)
            .translation()
    };
    let (xs, ys): (Vec<f32>, Vec<f32>) = kids.iter().map(|&c| (pos(c).x, pos(c).y)).unzip();
    let spread = |v: &[f32]| {
        v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
    };
    let direction = if kids.len() > 1 && spread(&ys) > spread(&xs) {
        LayoutDirection::Vertical
    } else {
        LayoutDirection::Horizontal
    };
    AutoLayout {
        direction,
        gap: 10.0,
        cross_gap: 10.0,
        padding: [10.0; 4],
        main_align: MainAlign::Start,
        cross_align: CrossAlign::Start,
        primary_sizing: ContainerSizing::Hug,
        counter_sizing: ContainerSizing::Hug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{NodeKind, Symbol};

    fn frame(doc: &mut Document, parent: NodeId, al: Option<AutoLayout>, w: f32, h: f32) -> NodeId {
        let mut n = Node::new(NodeKind::Frame, Symbol::EMPTY).with_size(w, h);
        if let Some(al) = al {
            n.ext_mut().auto_layout = Some(Box::new(al));
        }
        doc.build_child(parent, n).unwrap()
    }

    fn rect(doc: &mut Document, parent: NodeId, w: f32, h: f32) -> NodeId {
        doc.build_child(
            parent,
            Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(w, h),
        )
        .unwrap()
    }

    fn pos(doc: &Document, id: NodeId) -> (f32, f32) {
        let t = doc.node(id).unwrap().transform.translation();
        (t.x, t.y)
    }

    fn size(doc: &Document, id: NodeId) -> (f32, f32) {
        let n = doc.node(id).unwrap();
        (n.size.w, n.size.h)
    }

    fn setup() -> (Document, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        (doc, page)
    }

    #[test]
    fn hug_row_with_gap_padding_and_centre() {
        let (mut doc, page) = setup();
        let al = AutoLayout {
            gap: 10.0,
            padding: [5.0, 5.0, 5.0, 5.0],
            cross_align: CrossAlign::Center,
            primary_sizing: ContainerSizing::Hug,
            counter_sizing: ContainerSizing::Hug,
            ..Default::default()
        };
        let f = frame(&mut doc, page, Some(al), 1.0, 1.0);
        let a = rect(&mut doc, f, 20.0, 20.0);
        let b = rect(&mut doc, f, 30.0, 40.0);
        let tx = reflow(&mut doc, &[f]).unwrap();
        assert!(!tx.is_empty());
        assert_eq!(size(&doc, f), (70.0, 50.0));
        assert_eq!(pos(&doc, a), (5.0, 15.0));
        assert_eq!(pos(&doc, b), (35.0, 5.0));
        // Solving again changes nothing.
        assert!(reflow(&mut doc, &[f]).unwrap().is_empty());
    }

    #[test]
    fn fill_children_share_the_space_and_nested_frames_hug() {
        let (mut doc, page) = setup();
        let row = AutoLayout {
            gap: 0.0,
            primary_sizing: ContainerSizing::Fixed,
            counter_sizing: ContainerSizing::Fixed,
            cross_align: CrossAlign::Stretch,
            ..Default::default()
        };
        let f = frame(&mut doc, page, Some(row), 300.0, 50.0);
        let fixed = rect(&mut doc, f, 100.0, 10.0);
        let mut n = Node::new(NodeKind::Rectangle, Symbol::EMPTY).with_size(10.0, 10.0);
        n.ext_mut().layout_child.h_sizing = ChildSizing::Fill;
        let fill = doc.build_child(f, n).unwrap();
        let col = AutoLayout {
            direction: LayoutDirection::Vertical,
            gap: 4.0,
            primary_sizing: ContainerSizing::Hug,
            counter_sizing: ContainerSizing::Hug,
            ..Default::default()
        };
        let inner = frame(&mut doc, f, Some(col), 1.0, 1.0);
        rect(&mut doc, inner, 40.0, 10.0);
        rect(&mut doc, inner, 20.0, 10.0);
        reflow(&mut doc, &[f]).unwrap();
        assert_eq!(
            size(&doc, inner).0,
            40.0,
            "nested column hugs its widest child"
        );
        assert_eq!(
            size(&doc, fill),
            (160.0, 50.0),
            "fill takes the rest; stretch fills the height"
        );
        assert_eq!(pos(&doc, fixed), (0.0, 0.0));
        assert_eq!(pos(&doc, fill), (100.0, 0.0));
        assert_eq!(pos(&doc, inner), (260.0, 0.0));
    }

    #[test]
    fn wrap_breaks_lines_and_hugs_height() {
        let (mut doc, page) = setup();
        let al = AutoLayout {
            direction: LayoutDirection::Wrap,
            gap: 10.0,
            cross_gap: 5.0,
            primary_sizing: ContainerSizing::Fixed,
            counter_sizing: ContainerSizing::Hug,
            ..Default::default()
        };
        let f = frame(&mut doc, page, Some(al), 100.0, 1.0);
        let ids: Vec<NodeId> = (0..3).map(|_| rect(&mut doc, f, 40.0, 20.0)).collect();
        reflow(&mut doc, &[f]).unwrap();
        assert_eq!(pos(&doc, ids[1]), (50.0, 0.0));
        assert_eq!(pos(&doc, ids[2]), (0.0, 25.0), "third item wraps");
        assert_eq!(size(&doc, f), (100.0, 45.0));
    }

    #[test]
    fn roots_are_the_outermost_auto_layout_frames() {
        let (mut doc, page) = setup();
        let outer = frame(&mut doc, page, Some(AutoLayout::default()), 10.0, 10.0);
        let inner = frame(&mut doc, outer, Some(AutoLayout::default()), 10.0, 10.0);
        let leaf = rect(&mut doc, inner, 5.0, 5.0);
        let loose = rect(&mut doc, page, 5.0, 5.0);
        assert_eq!(roots_for(&doc, &[leaf, loose]), vec![outer]);
    }
}
