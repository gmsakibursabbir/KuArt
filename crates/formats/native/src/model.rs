//! Encoding of the Universal Design Model. Encoder and decoder for each type sit
//! side by side so they stay in sync.
//!
//! Determinism: node ids are written as pre-order indices, symbols and assets as
//! first-use indices. Two documents with the same content therefore encode to the
//! same bytes, whatever their edit history.

use std::collections::HashMap;
use std::sync::Arc;

use ude_core::assets::{AssetId, Codec};
use ude_core::geometry::{Affine, FillRule, PathBuilder, PathData, PathVerb, Point, Size};
use ude_core::layout::{
    AutoLayout, ChildSizing, Constraint, Constraints, ContainerSizing, CrossAlign, GridKind,
    LayoutChild, LayoutDirection, LayoutGrid, MainAlign, SizeLimits,
};
use ude_core::scene::{
    ArcData, BoolOpKind, ComponentInfo, ComponentRef, CornerRadii, InstanceData, LayerMask,
    Override, Preserved, VectorData, VectorPath,
};
use ude_core::styles::{
    BlendMode, Color, Effect, Gradient, GradientKind, GradientStop, ImageScaleMode, LineCap,
    LineJoin, Paint, PaintKind, Shadow, StrokeAlign,
};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextAlignV, TextCase, TextData, TextDecoration, TextRun,
    TextSizing, TextStyle,
};
use ude_core::{
    Node, NodeExt, NodeFlags, NodeId, NodeKind, Paints, Prop, SourceFormat, Stroke, Symbol,
};

use crate::wire::{Reader, WireEnum, Writer};
use crate::{NativeError, wire_enum};

wire_enum!(BlendMode {
    PassThrough,
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
});
wire_enum!(GradientKind {
    Linear,
    Radial,
    Angular,
    Diamond
});
wire_enum!(ImageScaleMode {
    Fill,
    Fit,
    Crop,
    Tile
});
wire_enum!(StrokeAlign {
    Inside,
    Center,
    Outside
});
wire_enum!(LineCap {
    Butt,
    Round,
    Square
});
wire_enum!(LineJoin {
    Miter,
    Round,
    Bevel
});
wire_enum!(TextDecoration {
    None,
    Underline,
    Strikethrough
});
wire_enum!(TextCase {
    Original,
    Upper,
    Lower,
    Title,
    SmallCaps
});
wire_enum!(TextSizing {
    AutoWidth,
    AutoHeight,
    Fixed,
    Truncate
});
wire_enum!(TextAlignH {
    Left,
    Center,
    Right,
    Justify
});
wire_enum!(TextAlignV {
    Top,
    Center,
    Bottom
});
wire_enum!(LayoutDirection {
    Horizontal,
    Vertical,
    Wrap
});
wire_enum!(MainAlign {
    Start,
    Center,
    End,
    SpaceBetween
});
wire_enum!(CrossAlign {
    Start,
    Center,
    End,
    Baseline,
    Stretch
});
wire_enum!(ContainerSizing { Fixed, Hug });
wire_enum!(ChildSizing { Fixed, Hug, Fill });
wire_enum!(Constraint {
    Min,
    Max,
    Center,
    Stretch,
    Scale
});
wire_enum!(GridKind {
    Columns,
    Rows,
    Grid
});
wire_enum!(FillRule { NonZero, EvenOdd });
wire_enum!(PathVerb {
    MoveTo,
    LineTo,
    QuadTo,
    CubicTo,
    Close
});
wire_enum!(BoolOpKind {
    Union,
    Subtract,
    Intersect,
    Exclude
});
wire_enum!(SourceFormat {
    Native,
    Figma,
    Xd,
    Psd,
    Svg,
    Pdf,
    Raster,
    Sketch,
    Illustrator
});
wire_enum!(Codec {
    Png,
    Jpeg,
    WebP,
    PsdChannels,
    Raw,
    Zlib,
    Zstd
});

// NodeKind tags (on-disk contract; append only).
const K_DOCUMENT: u8 = 0;
const K_PAGE: u8 = 1;
const K_FRAME: u8 = 2;
const K_GROUP: u8 = 3;
const K_RECTANGLE: u8 = 4;
const K_ELLIPSE: u8 = 5;
const K_POLYGON: u8 = 6;
const K_STAR: u8 = 7;
const K_LINE: u8 = 8;
const K_VECTOR: u8 = 9;
const K_TEXT: u8 = 10;
const K_RASTER: u8 = 11;
const K_COMPONENT: u8 = 12;
const K_COMPONENT_SET: u8 = 13;
const K_INSTANCE: u8 = 14;
const K_BOOLEAN: u8 = 15;
const K_SLICE: u8 = 16;
const K_PRESERVED: u8 = 17;

fn w_color(w: &mut Writer, c: Color) {
    w.f32s(&[c.r, c.g, c.b, c.a]);
}

fn r_color(r: &mut Reader) -> Result<Color, NativeError> {
    let [r_, g, b, a] = r.f32s::<4>()?;
    Ok(Color { r: r_, g, b, a })
}

fn w_opt_f32(w: &mut Writer, v: Option<f32>) {
    w.bool(v.is_some());
    if let Some(v) = v {
        w.f32(v);
    }
}

fn r_opt_f32(r: &mut Reader) -> Result<Option<f32>, NativeError> {
    Ok(if r.bool()? { Some(r.f32()?) } else { None })
}

fn w_enum<E: WireEnum>(w: &mut Writer, e: E) {
    w.u8(e.code());
}

fn r_enum<E: WireEnum>(r: &mut Reader) -> Result<E, NativeError> {
    E::from_code(r.u8()?)
}

pub(crate) struct Enc {
    nodes: HashMap<NodeId, u32>,
    syms: HashMap<Symbol, u32>,
    pub sym_order: Vec<Symbol>,
    assets: HashMap<AssetId, u32>,
    pub asset_order: Vec<AssetId>,
}

impl Enc {
    /// `order` is the pre-order list of all saved nodes; its positions become their ids.
    pub fn new(order: &[NodeId]) -> Self {
        Self {
            nodes: order
                .iter()
                .enumerate()
                .map(|(i, &id)| (id, i as u32))
                .collect(),
            syms: HashMap::new(),
            sym_order: Vec::new(),
            assets: HashMap::new(),
            asset_order: Vec::new(),
        }
    }

    fn sym(&mut self, w: &mut Writer, s: Symbol) {
        let i = match self.syms.get(&s) {
            Some(&i) => i,
            None => {
                let i = self.sym_order.len() as u32;
                self.syms.insert(s, i);
                self.sym_order.push(s);
                i
            }
        };
        w.var(u64::from(i));
    }

    fn asset(&mut self, w: &mut Writer, a: AssetId) {
        let i = match self.assets.get(&a) {
            Some(&i) => i,
            None => {
                let i = self.asset_order.len() as u32;
                self.assets.insert(a, i);
                self.asset_order.push(a);
                i
            }
        };
        w.var(u64::from(i));
    }

    pub fn node(&mut self, w: &mut Writer, n: &Node) {
        w.record(|w| {
            self.kind(w, &n.kind);
            self.sym(w, n.name);
            w.len(n.children.len());
            w.f32s(&n.transform.0);
            w.f32s(&[n.size.w, n.size.h]);
            w.f32(n.opacity);
            w.f32(n.corner_radius);
            w_enum(w, n.blend);
            w.var(u64::from(n.flags.bits()));
            self.paints(w, &n.fills);
            w.bool(n.ext.is_some());
            if let Some(ext) = &n.ext {
                self.ext(w, ext);
            }
        });
    }

    fn kind(&mut self, w: &mut Writer, k: &NodeKind) {
        match k {
            NodeKind::Document => w.u8(K_DOCUMENT),
            NodeKind::Page => w.u8(K_PAGE),
            NodeKind::Frame => w.u8(K_FRAME),
            NodeKind::Group => w.u8(K_GROUP),
            NodeKind::Rectangle => w.u8(K_RECTANGLE),
            NodeKind::Ellipse(arc) => {
                w.u8(K_ELLIPSE);
                w.bool(arc.is_some());
                if let Some(a) = arc {
                    w.f32s(&[a.start, a.sweep, a.inner_ratio]);
                }
            }
            NodeKind::Polygon { sides } => {
                w.u8(K_POLYGON);
                w.var(u64::from(*sides));
            }
            NodeKind::Star {
                points,
                inner_ratio,
            } => {
                w.u8(K_STAR);
                w.var(u64::from(*points));
                w.f32(*inner_ratio);
            }
            NodeKind::Line => w.u8(K_LINE),
            NodeKind::Vector(v) => {
                w.u8(K_VECTOR);
                vector(w, v);
            }
            NodeKind::Text(t) => {
                w.u8(K_TEXT);
                self.text(w, t);
            }
            NodeKind::Raster { asset } => {
                w.u8(K_RASTER);
                self.asset(w, *asset);
            }
            NodeKind::Component(c) => {
                w.u8(K_COMPONENT);
                self.sym(w, c.key);
                self.sym(w, c.description);
                w.len(c.variant_props.len());
                for &(k, v) in c.variant_props.iter() {
                    self.sym(w, k);
                    self.sym(w, v);
                }
            }
            NodeKind::ComponentSet => w.u8(K_COMPONENT_SET),
            NodeKind::Instance(i) => {
                w.u8(K_INSTANCE);
                match &i.component {
                    ComponentRef::Local(id) => match self.nodes.get(id) {
                        Some(&idx) => {
                            w.u8(0);
                            w.var(u64::from(idx));
                        }
                        // Dangling reference (component deleted): keep the instance, drop the link.
                        None => {
                            w.u8(1);
                            self.sym(w, Symbol::EMPTY);
                        }
                    },
                    ComponentRef::External(key) => {
                        w.u8(1);
                        self.sym(w, *key);
                    }
                }
                w.len(i.overrides.len());
                for o in i.overrides.iter() {
                    w.len(o.path.len());
                    for &s in o.path.iter() {
                        self.sym(w, s);
                    }
                    self.prop(w, &o.prop);
                }
            }
            NodeKind::BooleanOp(op) => {
                w.u8(K_BOOLEAN);
                w_enum(w, *op);
            }
            NodeKind::Slice => w.u8(K_SLICE),
            NodeKind::Preserved { source } => {
                w.u8(K_PRESERVED);
                w_enum(w, *source);
            }
        }
    }

    fn prop(&mut self, w: &mut Writer, p: &Prop) {
        w.u8(p.key());
        match p {
            Prop::Name(s) => self.sym(w, *s),
            Prop::Transform(t) => w.f32s(&t.0),
            Prop::Size(s) => w.f32s(&[s.w, s.h]),
            Prop::Opacity(o) => w.f32(*o),
            Prop::Blend(b) => w_enum(w, *b),
            Prop::Flags(f) => w.var(u64::from(f.bits())),
            Prop::Fills(p) => self.paints(w, p),
            Prop::Ext(e) => {
                w.bool(e.is_some());
                if let Some(e) = e {
                    self.ext(w, e);
                }
            }
            Prop::Kind(k) => self.kind(w, k),
            Prop::CornerRadius(v) => w.f32(*v),
        }
    }

    /// Length-prefixed so the decoder can share identical paint lists.
    fn paints(&mut self, w: &mut Writer, p: &Paints) {
        w.record(|w| {
            w.len(p.as_slice().len());
            for paint in p.as_slice() {
                match &paint.kind {
                    PaintKind::Solid(c) => {
                        w.u8(0);
                        w_color(w, *c);
                    }
                    PaintKind::Gradient(g) => {
                        w.u8(1);
                        w_enum(w, g.kind);
                        w.len(g.stops.len());
                        for s in g.stops.iter() {
                            w.f32(s.offset);
                            w_color(w, s.color);
                        }
                        w.f32s(&g.transform.0);
                    }
                    PaintKind::Image {
                        asset,
                        mode,
                        transform,
                    } => {
                        w.u8(2);
                        self.asset(w, *asset);
                        w_enum(w, *mode);
                        w.bool(transform.is_some());
                        if let Some(t) = transform {
                            w.f32s(&t.0);
                        }
                    }
                }
                w.f32(paint.opacity);
                w_enum(w, paint.blend);
                w.bool(paint.visible);
            }
        });
    }

    fn ext(&mut self, w: &mut Writer, e: &NodeExt) {
        w.record(|w| {
            w.bool(e.strokes.is_some());
            if let Some(s) = &e.strokes {
                self.paints(w, &s.paints);
                w.f32(s.width);
                w_enum(w, s.align);
                w_enum(w, s.cap);
                w_enum(w, s.join);
                w.f32(s.miter_limit);
                w.len(s.dashes.len());
                w.f32s(&s.dashes);
            }
            w.len(e.effects.len());
            for fx in e.effects.iter() {
                match fx {
                    Effect::DropShadow(s) | Effect::InnerShadow(s) => {
                        w.u8(if matches!(fx, Effect::DropShadow(_)) {
                            0
                        } else {
                            1
                        });
                        w_color(w, s.color);
                        w.f32s(&[s.offset.x, s.offset.y, s.blur, s.spread]);
                        w_enum(w, s.blend);
                        w.bool(s.visible);
                    }
                    Effect::LayerBlur { radius, visible }
                    | Effect::BackgroundBlur { radius, visible } => {
                        w.u8(if matches!(fx, Effect::LayerBlur { .. }) {
                            2
                        } else {
                            3
                        });
                        w.f32(*radius);
                        w.bool(*visible);
                    }
                }
            }
            w.bool(e.corners.is_some());
            if let Some(c) = &e.corners {
                w.f32s(&c.radii);
                w.f32(c.smoothing);
            }
            w.bool(e.auto_layout.is_some());
            if let Some(a) = &e.auto_layout {
                w_enum(w, a.direction);
                w.f32s(&[a.gap, a.cross_gap]);
                w.f32s(&a.padding);
                w_enum(w, a.main_align);
                w_enum(w, a.cross_align);
                w_enum(w, a.primary_sizing);
                w_enum(w, a.counter_sizing);
            }
            let lc = &e.layout_child;
            w_enum(w, lc.h_sizing);
            w_enum(w, lc.v_sizing);
            w.bool(lc.absolute);
            w.bool(lc.limits.is_some());
            if let Some(l) = &lc.limits {
                for v in l.min.iter().chain(&l.max) {
                    w_opt_f32(w, *v);
                }
            }
            w_enum(w, e.constraints.horizontal);
            w_enum(w, e.constraints.vertical);
            w.len(e.grids.len());
            for g in e.grids.iter() {
                w_enum(w, g.kind);
                w.var(u64::from(g.count));
                w.f32s(&[g.gutter, g.margin]);
                w_opt_f32(w, g.size);
            }
            match e.layer_mask.as_deref() {
                None => w.u8(0),
                Some(LayerMask::Raster {
                    asset,
                    default_color,
                    disabled,
                }) => {
                    w.u8(1);
                    self.asset(w, *asset);
                    w.u8(*default_color);
                    w.bool(*disabled);
                }
                Some(LayerMask::Vector { path, disabled }) => {
                    w.u8(2);
                    vector(w, path);
                    w.bool(*disabled);
                }
            }
            self.sym(w, e.source_id);
            w.len(e.preserved.len());
            for p in &e.preserved {
                w_enum(w, p.source);
                self.sym(w, p.key);
                self.asset(w, p.data);
            }
        });
    }

    fn text(&mut self, w: &mut Writer, t: &TextData) {
        w.str(&t.content);
        w.len(t.runs.len());
        for run in t.runs.iter() {
            w.var(u64::from(run.range.start));
            w.var(u64::from(run.range.end));
            w.var(u64::from(run.style));
        }
        w.len(t.styles.len());
        for s in t.styles.iter() {
            self.sym(w, s.family);
            w.var(u64::from(s.weight));
            w.bool(s.italic);
            w.f32(s.size);
            let (tag, v) = match s.line_height {
                LineHeight::Auto => (0, 0.0),
                LineHeight::Px(v) => (1, v),
                LineHeight::Percent(v) => (2, v),
            };
            w.u8(tag);
            w.f32(v);
            let (tag, v) = match s.letter_spacing {
                LetterSpacing::Normal => (0, 0.0),
                LetterSpacing::Px(v) => (1, v),
                LetterSpacing::Percent(v) => (2, v),
            };
            w.u8(tag);
            w.f32(v);
            w_enum(w, s.decoration);
            w_enum(w, s.case);
            self.paints(w, &s.fills);
        }
        w_enum(w, t.sizing);
        w_enum(w, t.align_h);
        w_enum(w, t.align_v);
        w.f32(t.paragraph_spacing);
    }
}

fn vector(w: &mut Writer, v: &VectorData) {
    w.record(|w| {
        w.len(v.paths.len());
        for p in v.paths.iter() {
            w_enum(w, p.fill_rule);
            w.len(p.path.verbs().len());
            for &verb in p.path.verbs() {
                w_enum(w, verb);
            }
            w.len(p.path.points().len());
            for pt in p.path.points() {
                w.f32s(&[pt.x, pt.y]);
            }
        }
    });
}

pub(crate) struct Dec {
    pub syms: Vec<Symbol>,
    pub assets: Vec<AssetId>,
    pub ids: Vec<NodeId>,
    paints: HashMap<Vec<u8>, Paints>,
    vectors: HashMap<Vec<u8>, Arc<VectorData>>,
    depth: u32,
    max_depth: u32,
}

impl Dec {
    pub fn new(syms: Vec<Symbol>, assets: Vec<AssetId>, ids: Vec<NodeId>, max_depth: u32) -> Self {
        Self {
            syms,
            assets,
            ids,
            paints: HashMap::new(),
            vectors: HashMap::new(),
            depth: 0,
            max_depth,
        }
    }

    fn sym(&self, r: &mut Reader) -> Result<Symbol, NativeError> {
        let i = r.var()?;
        usize::try_from(i)
            .ok()
            .and_then(|i| self.syms.get(i))
            .copied()
            .ok_or(NativeError::Corrupt("symbol index"))
    }

    fn asset(&self, r: &mut Reader) -> Result<AssetId, NativeError> {
        let i = r.var()?;
        usize::try_from(i)
            .ok()
            .and_then(|i| self.assets.get(i))
            .copied()
            .ok_or(NativeError::Corrupt("asset index"))
    }

    /// Returns the node and its child count. Trailing unknown fields are ignored.
    pub fn node(&mut self, r: &mut Reader) -> Result<(Node, u32), NativeError> {
        let mut r = r.record()?;
        let kind = self.kind(&mut r)?;
        let name = self.sym(&mut r)?;
        let children = r.u32()?;
        let mut node = Node::new(kind, name);
        node.transform = Affine(r.f32s()?);
        let [w, h] = r.f32s()?;
        node.size = Size::new(w, h);
        node.opacity = r.f32()?;
        node.corner_radius = r.f32()?;
        node.blend = r_enum(&mut r)?;
        node.flags = NodeFlags::from_bits(r.u16()?);
        node.fills = self.paints(&mut r)?;
        if r.bool()? {
            node.ext = Some(Box::new(self.ext(&mut r)?));
        }
        Ok((node, children))
    }

    fn kind(&mut self, r: &mut Reader) -> Result<NodeKind, NativeError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(NativeError::LimitExceeded("nesting depth"));
        }
        let k = self.kind_inner(r);
        self.depth -= 1;
        k
    }

    fn kind_inner(&mut self, r: &mut Reader) -> Result<NodeKind, NativeError> {
        Ok(match r.u8()? {
            K_DOCUMENT => NodeKind::Document,
            K_PAGE => NodeKind::Page,
            K_FRAME => NodeKind::Frame,
            K_GROUP => NodeKind::Group,
            K_RECTANGLE => NodeKind::Rectangle,
            K_ELLIPSE => NodeKind::Ellipse(if r.bool()? {
                let [start, sweep, inner_ratio] = r.f32s()?;
                Some(Box::new(ArcData {
                    start,
                    sweep,
                    inner_ratio,
                }))
            } else {
                None
            }),
            K_POLYGON => NodeKind::Polygon { sides: r.u16()? },
            K_STAR => NodeKind::Star {
                points: r.u16()?,
                inner_ratio: r.f32()?,
            },
            K_LINE => NodeKind::Line,
            K_VECTOR => NodeKind::Vector(self.vector(r)?),
            K_TEXT => NodeKind::Text(Box::new(self.text(r)?)),
            K_RASTER => NodeKind::Raster {
                asset: self.asset(r)?,
            },
            K_COMPONENT => {
                let key = self.sym(r)?;
                let description = self.sym(r)?;
                let n = r.count(2)?;
                let mut props = Vec::with_capacity(n);
                for _ in 0..n {
                    props.push((self.sym(r)?, self.sym(r)?));
                }
                NodeKind::Component(Box::new(ComponentInfo {
                    key,
                    description,
                    variant_props: props.into(),
                }))
            }
            K_COMPONENT_SET => NodeKind::ComponentSet,
            K_INSTANCE => {
                let component = match r.u8()? {
                    0 => {
                        let idx = r.var()?;
                        let id = usize::try_from(idx)
                            .ok()
                            .and_then(|i| self.ids.get(i))
                            .copied();
                        ComponentRef::Local(id.ok_or(NativeError::Corrupt("component reference"))?)
                    }
                    1 => ComponentRef::External(self.sym(r)?),
                    _ => return Err(NativeError::Corrupt("component reference kind")),
                };
                let n = r.count(2)?;
                let mut overrides = Vec::with_capacity(n);
                for _ in 0..n {
                    let len = r.count(1)?;
                    let mut path = Vec::with_capacity(len);
                    for _ in 0..len {
                        path.push(self.sym(r)?);
                    }
                    overrides.push(Override {
                        path: path.into(),
                        prop: self.prop(r)?,
                    });
                }
                NodeKind::Instance(Box::new(InstanceData {
                    component,
                    overrides: overrides.into(),
                }))
            }
            K_BOOLEAN => NodeKind::BooleanOp(r_enum(r)?),
            K_SLICE => NodeKind::Slice,
            K_PRESERVED => NodeKind::Preserved { source: r_enum(r)? },
            _ => return Err(NativeError::Corrupt("node kind")),
        })
    }

    fn prop(&mut self, r: &mut Reader) -> Result<Prop, NativeError> {
        Ok(match r.u8()? {
            0 => Prop::Name(self.sym(r)?),
            1 => Prop::Transform(Affine(r.f32s()?)),
            2 => {
                let [w, h] = r.f32s()?;
                Prop::Size(Size::new(w, h))
            }
            3 => Prop::Opacity(r.f32()?),
            4 => Prop::Blend(r_enum(r)?),
            5 => Prop::Flags(NodeFlags::from_bits(r.u16()?)),
            6 => Prop::Fills(self.paints(r)?),
            7 => Prop::Ext(if r.bool()? {
                Some(Box::new(self.ext(r)?))
            } else {
                None
            }),
            8 => Prop::Kind(self.kind(r)?),
            9 => Prop::CornerRadius(r.f32()?),
            _ => return Err(NativeError::Corrupt("override property")),
        })
    }

    fn paints(&mut self, r: &mut Reader) -> Result<Paints, NativeError> {
        let raw = r.bytes()?;
        if let Some(p) = self.paints.get(raw) {
            return Ok(p.clone());
        }
        let mut pr = Reader::new(raw);
        let n = pr.count(6)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = match pr.u8()? {
                0 => PaintKind::Solid(r_color(&mut pr)?),
                1 => {
                    let kind = r_enum(&mut pr)?;
                    let n = pr.count(20)?;
                    let mut stops = Vec::with_capacity(n);
                    for _ in 0..n {
                        stops.push(GradientStop {
                            offset: pr.f32()?,
                            color: r_color(&mut pr)?,
                        });
                    }
                    let transform = Affine(pr.f32s()?);
                    PaintKind::Gradient(Arc::new(Gradient {
                        kind,
                        stops: stops.into(),
                        transform,
                    }))
                }
                2 => PaintKind::Image {
                    asset: self.asset(&mut pr)?,
                    mode: r_enum(&mut pr)?,
                    transform: if pr.bool()? {
                        Some(Box::new(Affine(pr.f32s()?)))
                    } else {
                        None
                    },
                },
                _ => return Err(NativeError::Corrupt("paint kind")),
            };
            out.push(Paint {
                kind,
                opacity: pr.f32()?,
                blend: r_enum(&mut pr)?,
                visible: pr.bool()?,
            });
        }
        let p = Paints::new(out);
        self.paints.insert(raw.to_vec(), p.clone());
        Ok(p)
    }

    fn vector(&mut self, r: &mut Reader) -> Result<Arc<VectorData>, NativeError> {
        let raw = r.bytes()?;
        if let Some(v) = self.vectors.get(raw) {
            return Ok(v.clone());
        }
        let mut vr = Reader::new(raw);
        let n = vr.count(3)?;
        let mut paths = Vec::with_capacity(n);
        for _ in 0..n {
            let fill_rule = r_enum(&mut vr)?;
            let nv = vr.count(1)?;
            let mut verbs = Vec::with_capacity(nv);
            for _ in 0..nv {
                verbs.push(r_enum::<PathVerb>(&mut vr)?);
            }
            let np = vr.count(8)?;
            let mut pts = Vec::with_capacity(np);
            for _ in 0..np {
                let [x, y] = vr.f32s()?;
                pts.push(Point::new(x, y));
            }
            paths.push(VectorPath {
                path: build_path(&verbs, &pts)?,
                fill_rule,
            });
        }
        let v = Arc::new(VectorData {
            paths: paths.into(),
        });
        self.vectors.insert(raw.to_vec(), v.clone());
        Ok(v)
    }

    fn ext(&mut self, r: &mut Reader) -> Result<NodeExt, NativeError> {
        let mut r = r.record()?;
        let r = &mut r;
        let mut e = NodeExt::default();
        if r.bool()? {
            let paints = self.paints(r)?;
            let width = r.f32()?;
            let (align, cap, join) = (r_enum(r)?, r_enum(r)?, r_enum(r)?);
            let miter_limit = r.f32()?;
            let n = r.count(4)?;
            let mut dashes = Vec::with_capacity(n);
            for _ in 0..n {
                dashes.push(r.f32()?);
            }
            e.strokes = Some(Box::new(Stroke {
                paints,
                width,
                align,
                cap,
                join,
                miter_limit,
                dashes: dashes.into(),
            }));
        }
        let n = r.count(6)?;
        let mut effects = Vec::with_capacity(n);
        for _ in 0..n {
            effects.push(match r.u8()? {
                tag @ (0 | 1) => {
                    let color = r_color(r)?;
                    let [x, y, blur, spread] = r.f32s()?;
                    let s = Shadow {
                        color,
                        offset: Point::new(x, y),
                        blur,
                        spread,
                        blend: r_enum(r)?,
                        visible: r.bool()?,
                    };
                    if tag == 0 {
                        Effect::DropShadow(s)
                    } else {
                        Effect::InnerShadow(s)
                    }
                }
                2 => Effect::LayerBlur {
                    radius: r.f32()?,
                    visible: r.bool()?,
                },
                3 => Effect::BackgroundBlur {
                    radius: r.f32()?,
                    visible: r.bool()?,
                },
                _ => return Err(NativeError::Corrupt("effect kind")),
            });
        }
        e.effects = effects.into();
        if r.bool()? {
            e.corners = Some(CornerRadii {
                radii: r.f32s()?,
                smoothing: r.f32()?,
            });
        }
        if r.bool()? {
            let direction = r_enum(r)?;
            let [gap, cross_gap] = r.f32s()?;
            e.auto_layout = Some(Box::new(AutoLayout {
                direction,
                gap,
                cross_gap,
                padding: r.f32s()?,
                main_align: r_enum(r)?,
                cross_align: r_enum(r)?,
                primary_sizing: r_enum(r)?,
                counter_sizing: r_enum(r)?,
            }));
        }
        e.layout_child = LayoutChild {
            h_sizing: r_enum(r)?,
            v_sizing: r_enum(r)?,
            absolute: r.bool()?,
            limits: if r.bool()? {
                Some(Box::new(SizeLimits {
                    min: [r_opt_f32(r)?, r_opt_f32(r)?],
                    max: [r_opt_f32(r)?, r_opt_f32(r)?],
                }))
            } else {
                None
            },
        };
        e.constraints = Constraints {
            horizontal: r_enum(r)?,
            vertical: r_enum(r)?,
        };
        let n = r.count(4)?;
        let mut grids = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = r_enum(r)?;
            let count = r.u16()?;
            let [gutter, margin] = r.f32s()?;
            grids.push(LayoutGrid {
                kind,
                count,
                gutter,
                margin,
                size: r_opt_f32(r)?,
            });
        }
        e.grids = grids.into();
        e.layer_mask = match r.u8()? {
            0 => None,
            1 => Some(Box::new(LayerMask::Raster {
                asset: self.asset(r)?,
                default_color: r.u8()?,
                disabled: r.bool()?,
            })),
            2 => Some(Box::new(LayerMask::Vector {
                path: self.vector(r)?,
                disabled: r.bool()?,
            })),
            _ => return Err(NativeError::Corrupt("layer mask kind")),
        };
        e.source_id = self.sym(r)?;
        let n = r.count(3)?;
        for _ in 0..n {
            e.preserved.push(Preserved {
                source: r_enum(r)?,
                key: self.sym(r)?,
                data: self.asset(r)?,
            });
        }
        Ok(e)
    }

    fn text(&mut self, r: &mut Reader) -> Result<TextData, NativeError> {
        let content: Box<str> = r.str()?.into();
        let n = r.count(3)?;
        let mut runs = Vec::with_capacity(n);
        for _ in 0..n {
            let (start, end) = (r.u32()?, r.u32()?);
            runs.push(TextRun {
                range: start..end,
                style: r.u16()?,
            });
        }
        let n = r.count(16)?;
        let mut styles = Vec::with_capacity(n);
        for _ in 0..n {
            let family = self.sym(r)?;
            let weight = r.u16()?;
            let italic = r.bool()?;
            let size = r.f32()?;
            let line_height = match (r.u8()?, r.f32()?) {
                (0, _) => LineHeight::Auto,
                (1, v) => LineHeight::Px(v),
                (2, v) => LineHeight::Percent(v),
                _ => return Err(NativeError::Corrupt("line height")),
            };
            let letter_spacing = match (r.u8()?, r.f32()?) {
                (0, _) => LetterSpacing::Normal,
                (1, v) => LetterSpacing::Px(v),
                (2, v) => LetterSpacing::Percent(v),
                _ => return Err(NativeError::Corrupt("letter spacing")),
            };
            styles.push(TextStyle {
                family,
                weight,
                italic,
                size,
                line_height,
                letter_spacing,
                decoration: r_enum(r)?,
                case: r_enum(r)?,
                fills: self.paints(r)?,
            });
        }
        let t = TextData {
            content,
            runs: runs.into(),
            styles: styles.into(),
            sizing: r_enum(r)?,
            align_h: r_enum(r)?,
            align_v: r_enum(r)?,
            paragraph_spacing: r.f32()?,
        };
        if !t.validate() {
            return Err(NativeError::Corrupt("text runs"));
        }
        Ok(t)
    }
}

fn build_path(verbs: &[PathVerb], pts: &[Point]) -> Result<PathData, NativeError> {
    let needed: usize = verbs.iter().map(|v| v.point_count()).sum();
    if needed != pts.len() {
        return Err(NativeError::Corrupt("path point count"));
    }
    let mut b = PathBuilder::default();
    let mut i = 0;
    for &v in verbs {
        let p = &pts[i..i + v.point_count()];
        match v {
            PathVerb::MoveTo => b.move_to(p[0].x, p[0].y),
            PathVerb::LineTo => b.line_to(p[0].x, p[0].y),
            PathVerb::QuadTo => b.quad_to(p[0].x, p[0].y, p[1].x, p[1].y),
            PathVerb::CubicTo => b.cubic_to(p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y),
            PathVerb::Close => b.close(),
        };
        i += v.point_count();
    }
    Ok(b.build())
}
