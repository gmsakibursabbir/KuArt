//! Scene graph nodes: compact hot part ([`Node`]) and sparse cold part ([`NodeExt`]).

use std::sync::Arc;

use crate::assets::AssetId;
use crate::document::Prop;
use crate::geometry::{Affine, FillRule, PathData, Size};
use crate::id::NodeId;
use crate::intern::Symbol;
use crate::layout::{AutoLayout, Constraints, LayoutChild, LayoutGrid, SizeLimits};
use crate::styles::{BlendMode, Effect, Paints, Stroke};
use crate::typography::TextData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NodeFlags(u16);

impl NodeFlags {
    pub const VISIBLE: NodeFlags = NodeFlags(1 << 0);
    pub const LOCKED: NodeFlags = NodeFlags(1 << 1);
    pub const CLIPS_CONTENT: NodeFlags = NodeFlags(1 << 2);
    /// Masks the following siblings up to the next mask.
    pub const IS_MASK: NodeFlags = NodeFlags(1 << 3);
    /// Rendered from a stored fallback raster because the source feature is not fully supported.
    pub const USES_FALLBACK: NodeFlags = NodeFlags(1 << 4);

    pub const fn empty() -> Self {
        NodeFlags(0)
    }

    pub const fn contains(self, other: NodeFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn set(&mut self, other: NodeFlags, on: bool) {
        if on {
            self.0 |= other.0
        } else {
            self.0 &= !other.0
        }
    }

    pub const fn union(self, other: NodeFlags) -> Self {
        NodeFlags(self.0 | other.0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Self {
        NodeFlags(bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFormat {
    Native,
    Figma,
    Xd,
    Psd,
    Svg,
    Pdf,
    Raster,
}

/// Per-feature import fidelity, surfaced in the UI and the compatibility matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SupportLevel {
    Full,
    Partial,
    /// Displayed (possibly via fallback) and written back on export, but not editable.
    Preserved,
    Unsupported,
}

/// Raw source data we cannot model; kept so export does not destroy it.
#[derive(Clone, Debug, PartialEq)]
pub struct Preserved {
    pub source: SourceFormat,
    /// Format-specific key, e.g. a PSD tagged-block key like `lfx2`.
    pub key: Symbol,
    pub data: AssetId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcData {
    pub start: f32,
    pub sweep: f32,
    pub inner_ratio: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    pub path: PathData,
    pub fill_rule: FillRule,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct VectorData {
    pub paths: Box<[VectorPath]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOpKind {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentInfo {
    pub key: Symbol,
    pub description: Symbol,
    /// Variant properties, e.g. `(State, Hover)`.
    pub variant_props: Box<[(Symbol, Symbol)]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComponentRef {
    Local(NodeId),
    /// Component from an external library, by key.
    External(Symbol),
}

/// Override of one property on a node inside the instance's component,
/// addressed by stable source ids so it survives component edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub path: Box<[Symbol]>,
    pub prop: Prop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceData {
    pub component: ComponentRef,
    pub overrides: Box<[Override]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Document,
    Page,
    /// Frames that are direct children of a page are artboards.
    Frame,
    Group,
    Rectangle,
    Ellipse(Option<Box<ArcData>>),
    Polygon {
        sides: u16,
    },
    Star {
        points: u16,
        inner_ratio: f32,
    },
    Line,
    Vector(Arc<VectorData>),
    Text(Box<TextData>),
    Raster {
        asset: AssetId,
    },
    Component(Box<ComponentInfo>),
    ComponentSet,
    Instance(Box<InstanceData>),
    BooleanOp(BoolOpKind),
    Slice,
    /// Unknown source node, drawn from its fallback raster.
    Preserved {
        source: SourceFormat,
    },
}

impl NodeKind {
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            NodeKind::Document
                | NodeKind::Page
                | NodeKind::Frame
                | NodeKind::Group
                | NodeKind::Component(_)
                | NodeKind::ComponentSet
                | NodeKind::Instance(_)
                | NodeKind::BooleanOp(_)
        )
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            NodeKind::Ellipse(Some(_)) => size_of::<ArcData>(),
            NodeKind::Vector(v) => {
                let own = size_of::<VectorData>()
                    + v.paths
                        .iter()
                        .map(|p| size_of::<VectorPath>() + p.path.heap_bytes())
                        .sum::<usize>();
                own / Arc::strong_count(v)
            }
            NodeKind::Text(t) => size_of::<TextData>() + t.heap_bytes(),
            NodeKind::Component(c) => size_of::<ComponentInfo>() + c.variant_props.len() * 8,
            NodeKind::Instance(i) => {
                size_of::<InstanceData>() + i.overrides.len() * size_of::<Override>()
            }
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CornerRadii {
    /// top-left, top-right, bottom-right, bottom-left
    pub radii: [f32; 4],
    /// iOS-style corner smoothing, 0..=1.
    pub smoothing: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerMask {
    Raster {
        asset: AssetId,
        default_color: u8,
        disabled: bool,
    },
    Vector {
        path: Arc<VectorData>,
        disabled: bool,
    },
}

/// Rare node data; absent for most nodes (8 bytes in [`Node`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NodeExt {
    // Large, less common parts are boxed so a NodeExt holding only, say, an effect stays small.
    pub strokes: Option<Box<Stroke>>,
    pub effects: Box<[Effect]>,
    /// Per-corner radii and smoothing; overrides `Node::corner_radius` when set.
    pub corners: Option<CornerRadii>,
    pub auto_layout: Option<Box<AutoLayout>>,
    pub layout_child: LayoutChild,
    pub constraints: Constraints,
    pub grids: Box<[LayoutGrid]>,
    pub layer_mask: Option<Box<LayerMask>>,
    /// Original id in the source file (round-trip, diagnostics, override paths).
    pub source_id: Symbol,
    pub preserved: Vec<Preserved>,
}

impl NodeExt {
    pub fn heap_bytes(&self) -> usize {
        self.strokes.as_ref().map_or(0, |s| {
            size_of::<Stroke>() + s.paints.heap_bytes() + s.dashes.len() * 4
        }) + self
            .auto_layout
            .as_ref()
            .map_or(0, |_| size_of::<AutoLayout>())
            + self
                .layer_mask
                .as_ref()
                .map_or(0, |_| size_of::<LayerMask>())
            + self
                .layout_child
                .limits
                .as_ref()
                .map_or(0, |_| size_of::<SizeLimits>())
            + self.effects.len() * size_of::<Effect>()
            + self.grids.len() * size_of::<LayoutGrid>()
            + self.preserved.capacity() * size_of::<Preserved>()
    }
}

/// Hot node data. Kept at or below 128 bytes (see `tests::node_is_compact`).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub name: Symbol,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Local → parent space.
    pub transform: Affine,
    /// Local box `(0,0)-(w,h)`.
    pub size: Size,
    pub opacity: f32,
    /// Uniform corner radius (the common case). Per-corner radii live in `NodeExt::corners`.
    /// Fits in padding: `Node` stays 120 bytes.
    pub corner_radius: f32,
    pub blend: BlendMode,
    pub flags: NodeFlags,
    pub fills: Paints,
    pub ext: Option<Box<NodeExt>>,
}

impl Node {
    pub fn new(kind: NodeKind, name: Symbol) -> Self {
        let blend = if matches!(kind, NodeKind::Group) {
            BlendMode::PassThrough
        } else {
            BlendMode::Normal
        };
        Self {
            kind,
            name,
            parent: None,
            children: Vec::new(),
            transform: Affine::IDENTITY,
            size: Size::default(),
            opacity: 1.0,
            corner_radius: 0.0,
            blend,
            flags: NodeFlags::VISIBLE,
            fills: Paints::default(),
            ext: None,
        }
    }

    pub fn with_transform(mut self, t: Affine) -> Self {
        self.transform = t;
        self
    }

    pub fn with_size(mut self, w: f32, h: f32) -> Self {
        self.size = Size::new(w, h);
        self
    }

    pub fn with_fills(mut self, fills: Paints) -> Self {
        self.fills = fills;
        self
    }

    /// Effective radii: top-left, top-right, bottom-right, bottom-left.
    pub fn corner_radii(&self) -> [f32; 4] {
        self.ext
            .as_ref()
            .and_then(|e| e.corners)
            .map_or([self.corner_radius; 4], |c| c.radii)
    }

    pub fn is_visible(&self) -> bool {
        self.flags.contains(NodeFlags::VISIBLE)
    }

    /// Ext data, created on first write.
    pub fn ext_mut(&mut self) -> &mut NodeExt {
        self.ext.get_or_insert_with(Default::default)
    }

    pub fn heap_bytes(&self) -> usize {
        self.children.capacity() * size_of::<NodeId>()
            + self.fills.heap_bytes()
            + self.kind.heap_bytes()
            + self
                .ext
                .as_ref()
                .map_or(0, |e| size_of::<NodeExt>() + e.heap_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_is_compact() {
        assert!(
            size_of::<NodeExt>() <= 128,
            "NodeExt is {} bytes",
            size_of::<NodeExt>()
        );
        assert!(
            size_of::<Node>() <= 128,
            "Node is {} bytes",
            size_of::<Node>()
        );
        assert!(
            size_of::<NodeKind>() <= 16,
            "NodeKind is {} bytes",
            size_of::<NodeKind>()
        );
    }

    #[test]
    fn ids_are_compact() {
        assert_eq!(size_of::<NodeId>(), 8);
        assert_eq!(size_of::<Option<NodeId>>(), 8);
        assert_eq!(size_of::<Symbol>(), 4);
    }

    #[test]
    fn flags() {
        let mut f = NodeFlags::VISIBLE;
        f.set(NodeFlags::LOCKED, true);
        assert!(f.contains(NodeFlags::VISIBLE.union(NodeFlags::LOCKED)));
        f.set(NodeFlags::VISIBLE, false);
        assert!(!f.contains(NodeFlags::VISIBLE));
    }
}
