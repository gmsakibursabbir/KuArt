//! Moving layers between documents: names, fonts and images are interned in each document's
//! own tables, so a copied subtree has its symbols and asset ids remapped into the target.
//! Used by copy/paste across tabs, pasting SVG from other apps, and importing into a document.

use std::collections::HashMap;

use ude_core::scene::{ComponentRef, InstanceData, LayerMask};
use ude_core::styles::PaintKind;
use ude_core::{AssetId, Document, Node, NodeKind, Paints, Symbol};

use crate::commands::Tree;

pub(crate) struct Remap<'a> {
    from: &'a Document,
    assets: HashMap<AssetId, AssetId>,
    /// Same document: links to main components stay; otherwise instances become detached
    /// copies of how they look.
    keep_links: bool,
}

impl<'a> Remap<'a> {
    pub(crate) fn new(from: &'a Document, keep_links: bool) -> Self {
        Self {
            from,
            assets: HashMap::new(),
            keep_links,
        }
    }

    fn sym(&self, to: &mut Document, s: Symbol) -> Symbol {
        to.strings.intern(self.from.strings.resolve(s))
    }

    fn asset(&mut self, to: &mut Document, a: AssetId) -> AssetId {
        if let Some(&b) = self.assets.get(&a) {
            return b;
        }
        let b = match self.from.assets.get(a) {
            Some(e) => to.assets.insert(e.clone()),
            None => a,
        };
        self.assets.insert(a, b);
        b
    }

    fn paints(&mut self, to: &mut Document, p: &Paints) -> Paints {
        if p.is_empty() {
            return p.clone();
        }
        let list = p
            .as_slice()
            .iter()
            .cloned()
            .map(|mut q| {
                if let PaintKind::Image { asset, .. } = &mut q.kind {
                    *asset = self.asset(to, *asset);
                }
                q
            })
            .collect();
        Paints::new(list)
    }

    fn node(&mut self, to: &mut Document, n: &mut Node) {
        n.name = self.sym(to, n.name);
        n.fills = self.paints(to, &n.fills);
        match &mut n.kind {
            NodeKind::Text(t) => {
                let mut styles = t.styles.to_vec();
                for s in &mut styles {
                    s.family = self.sym(to, s.family);
                    s.fills = self.paints(to, &s.fills);
                }
                t.styles = styles.into();
            }
            NodeKind::Raster { asset } => *asset = self.asset(to, *asset),
            NodeKind::Component(info) => {
                info.key = self.sym(to, info.key);
                info.description = self.sym(to, info.description);
                let props: Vec<(Symbol, Symbol)> = info
                    .variant_props
                    .iter()
                    .map(|&(k, v)| (self.sym(to, k), self.sym(to, v)))
                    .collect();
                info.variant_props = props.into();
            }
            NodeKind::Instance(d) if !self.keep_links => {
                // Its main is in the other document: keep the look, drop the link.
                let key = match d.component {
                    ComponentRef::External(k) => self.sym(to, k),
                    ComponentRef::Local(_) => Symbol::EMPTY,
                };
                **d = InstanceData {
                    component: ComponentRef::External(key),
                    overrides: Box::new([]),
                };
            }
            _ => {}
        }
        if let Some(ext) = n.ext.as_mut() {
            ext.source_id = self.sym(to, ext.source_id);
            if let Some(s) = ext.strokes.as_mut() {
                s.paints = self.paints(to, &s.paints);
            }
            if let Some(m) = ext.layer_mask.as_mut()
                && let LayerMask::Raster { asset, .. } = m.as_mut()
            {
                *asset = self.asset(to, *asset);
            }
            for p in &mut ext.preserved {
                p.key = self.sym(to, p.key);
                p.data = self.asset(to, p.data);
            }
        }
    }

    /// Rewrites a subtree taken from `from` so it is valid in `to`.
    pub(crate) fn tree(&mut self, to: &mut Document, t: &mut Tree) {
        self.node(to, &mut t.node);
        for c in &mut t.children {
            self.tree(to, c);
        }
    }
}
