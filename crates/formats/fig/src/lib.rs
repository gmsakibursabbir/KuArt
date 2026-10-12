//! Figma import (`docs/format-strategy.md` §2). A `.fig` file ("Save local copy") is a ZIP
//! holding `canvas.fig` and `images/<hash>`; older files are a bare `canvas.fig`. That file is
//! `fig-kiwi` + version + length-prefixed chunks: a Kiwi schema and a "Message" whose
//! `nodeChanges` list every node with its parent and position, and whose `blobs` hold vector
//! geometry. Chunks are raw-deflate or zstd compressed.
//!
//! Mapped: pages, frames, groups, sections, components and instances (expanded from their
//! component), rectangles, ellipses, lines, stars, polygons, vectors and boolean results
//! (as their computed geometry), text with styled runs, solid/gradient/image paints, strokes,
//! shadows and blurs, corner radii, masks, clipping and auto layout.

mod kiwi;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use kiwi::{Schema, Value};
use ude_core::assets::{AssetEntry, AssetId, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::layout::{
    AutoLayout, ChildSizing, ContainerSizing, CrossAlign, LayoutDirection, MainAlign,
};
use ude_core::scene::{
    ComponentInfo, ComponentRef, CornerRadii, InstanceData, NodeFlags, VectorData, VectorPath,
};
use ude_core::styles::{
    BlendMode, Color, Effect, Gradient, GradientKind, GradientStop, ImageScaleMode, Paint,
    PaintKind, Paints, Shadow, Stroke, StrokeAlign,
};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextAlignV, TextCase, TextData, TextDecoration, TextRun,
    TextSizing, TextStyle,
};
use ude_core::{
    Affine, Document, FillRule, Node, NodeId, NodeKind, Op, PathData, Point, Prop, SourceFormat,
    Symbol,
};

/// Decompressed chunk and ZIP entry limit (untrusted-input guard).
const MAX_CHUNK: usize = 1 << 30;
/// Instances nested deeper than this are not expanded (guards against cycles).
const MAX_INSTANCE_DEPTH: usize = 24;

pub struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
}

pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    let mut images: HashMap<String, Vec<u8>> = HashMap::new();
    let canvas: Vec<u8>;
    let canvas_bytes: &[u8] = if ude_format_zip::Zip::is_zip(bytes) {
        let z = ude_format_zip::Zip::open(bytes)?;
        canvas = z
            .read("canvas.fig", MAX_CHUNK)?
            .ok_or("the ZIP has no canvas.fig: not a Figma file")?;
        let names: Vec<String> = z
            .names()
            .filter(|n| n.starts_with("images/"))
            .map(str::to_owned)
            .collect();
        for n in names {
            if let Some(data) = z.read(&n, MAX_CHUNK)? {
                images.insert(n["images/".len()..].to_ascii_lowercase(), data);
            }
        }
        &canvas
    } else {
        bytes
    };
    let mut warnings = Warnings::default();
    let header = canvas_bytes.get(..8).ok_or("not a Figma file")?;
    match header {
        b"fig-kiwi" => {}
        b"fig-jam." => warnings.figjam = true,
        _ => return Err("not a Figma file (expected a fig-kiwi header)".into()),
    }
    let mut p = 12; // header + version
    let mut chunks = Vec::new();
    while p + 4 <= canvas_bytes.len() && chunks.len() < 2 {
        let len = u32::from_le_bytes(canvas_bytes[p..p + 4].try_into().map_err(|_| "bad chunk")?)
            as usize;
        let data = canvas_bytes
            .get(p + 4..p + 4 + len)
            .ok_or("truncated Figma file")?;
        chunks.push(decompress(data)?);
        p += 4 + len;
    }
    let [schema, message] = <[Vec<u8>; 2]>::try_from(chunks).map_err(|_| "truncated Figma file")?;
    let schema = Schema::decode(&schema)?;
    let msg = schema.decode_root("Message", &message)?;

    let blobs: Vec<&[u8]> = msg
        .array("blobs")
        .iter()
        .map(|b| b.bytes("bytes").unwrap_or(&[]))
        .collect();
    let nodes = msg.array("nodeChanges");
    let mut b = Builder {
        doc: Document::new(),
        nodes: HashMap::new(),
        children: HashMap::new(),
        blobs,
        images,
        image_assets: HashMap::new(),
        warnings,
        symbols: HashMap::new(),
        instances: Vec::new(),
    };
    b.doc.meta.title = name.to_owned();
    b.doc.meta.source = Some(SourceFormat::Figma);
    let mut document = None;
    for nc in nodes {
        let Some(id) = guid(nc.get("guid")) else {
            continue;
        };
        if nc.enum_("type") == Some("DOCUMENT") {
            document = Some(id);
        }
        if let Some(pi) = nc.get("parentIndex")
            && let Some(parent) = guid(pi.get("guid"))
        {
            let pos = pi.str("position").unwrap_or("").to_owned();
            b.children.entry(parent).or_default().push((pos, id));
        }
        b.nodes.insert(id, nc);
    }
    for list in b.children.values_mut() {
        // Fractional-index strings sort in paint order (bottom first).
        list.sort();
    }
    let document = document.ok_or("the file has no document node")?;
    let root = b.doc.root();
    let pages: Vec<Guid> = b.kids(document);
    for page in pages {
        let nc = b.nodes[&page];
        if nc.enum_("type") != Some("CANVAS") || nc.bool("internalOnly") == Some(true) {
            continue;
        }
        let name = b.doc.strings.intern(nc.str("name").unwrap_or("Page"));
        let id = b
            .doc
            .build_child(root, Node::new(NodeKind::Page, name))
            .map_err(|e| e.to_string())?;
        for child in b.kids(page) {
            b.node(child, id, 0, None)?;
        }
    }
    if b.doc.children(root).is_empty() {
        return Err("the file has no pages".into());
    }
    b.link_instances();
    let warnings = b.warnings.finish();
    let mut doc = b.doc;
    let _ = doc.take_changes();
    Ok(Imported { doc, warnings })
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        zstd::stream::read::Decoder::new(data)
            .map_err(|e| e.to_string())?
            .take(MAX_CHUNK as u64)
            .read_to_end(&mut out)
            .map_err(|e| format!("zstd chunk: {e}"))?;
    } else {
        flate2::read::DeflateDecoder::new(data)
            .take(MAX_CHUNK as u64)
            .read_to_end(&mut out)
            .map_err(|e| format!("deflate chunk: {e}"))?;
    }
    Ok(out)
}

type Guid = (u64, u64);

fn guid(v: Option<&Value>) -> Option<Guid> {
    let v = v?;
    Some((v.u64("sessionID")?, v.u64("localID")?))
}

#[derive(Default)]
struct Warnings {
    figjam: bool,
    overrides: usize,
    missing_images: usize,
    unknown: HashMap<String, usize>,
    deep_instances: usize,
}

impl Warnings {
    fn finish(self) -> Vec<String> {
        let mut w = Vec::new();
        if self.figjam {
            w.push("FigJam board: only shapes, text and images are imported".into());
        }
        if self.overrides > 0 {
            w.push(format!(
                "{} instance override(s) could not be applied (nested override paths)",
                self.overrides
            ));
        }
        if self.missing_images > 0 {
            w.push(format!(
                "{} image fill(s) reference images missing from the file",
                self.missing_images
            ));
        }
        if self.deep_instances > 0 {
            w.push(format!(
                "{} instance(s) nested too deeply were not expanded",
                self.deep_instances
            ));
        }
        let mut unknown: Vec<_> = self.unknown.into_iter().collect();
        unknown.sort();
        for (kind, n) in unknown {
            w.push(format!("{n} {kind} node(s) imported as frames"));
        }
        w
    }
}

struct Builder<'a, 's> {
    doc: Document,
    nodes: HashMap<Guid, &'a Value<'s>>,
    children: HashMap<Guid, Vec<(String, Guid)>>,
    blobs: Vec<&'a [u8]>,
    images: HashMap<String, Vec<u8>>,
    image_assets: HashMap<String, AssetId>,
    warnings: Warnings,
    /// Component guid → built node.
    symbols: HashMap<Guid, NodeId>,
    /// Instances to link to their component once everything is built.
    instances: Vec<(NodeId, Guid)>,
}

/// Override values for nodes inside an instance, keyed by the overridden node's guid.
type Overrides<'v, 's> = HashMap<Guid, Vec<&'v Value<'s>>>;

impl<'a, 's> Builder<'a, 's> {
    fn kids(&self, id: Guid) -> Vec<Guid> {
        self.children
            .get(&id)
            .map(|v| v.iter().map(|(_, g)| *g).collect())
            .unwrap_or_default()
    }

    /// Builds node `id` (and its subtree) under `parent`. `overrides` apply inside instances.
    fn node(
        &mut self,
        id: Guid,
        parent: NodeId,
        depth: usize,
        overrides: Option<&Overrides<'a, 's>>,
    ) -> Result<(), String> {
        let Some(&original) = self.nodes.get(&id) else {
            return Ok(());
        };
        // Instance overrides replace fields of the component's node.
        let patched;
        let nc: &Value = match overrides.and_then(|o| o.get(&id)) {
            Some(list) => {
                patched = patch(original, list);
                &patched
            }
            None => original,
        };
        let ty = nc.enum_("type").unwrap_or("");
        let name = self.doc.strings.intern(nc.str("name").unwrap_or(ty));
        let mut node = match ty {
            "FRAME" | "SECTION" | "SYMBOL" | "INSTANCE" | "STICKY" | "SHAPE_WITH_TEXT"
            | "TABLE" => {
                let kind = match ty {
                    "SYMBOL" => NodeKind::Component(Box::new(ComponentInfo {
                        key: self
                            .doc
                            .strings
                            .intern(nc.str("componentKey").unwrap_or("")),
                        description: Symbol::EMPTY,
                        variant_props: Box::new([]),
                    })),
                    "INSTANCE" => NodeKind::Instance(Box::new(InstanceData {
                        component: ComponentRef::External(Symbol::EMPTY),
                        overrides: Box::new([]),
                    })),
                    _ => NodeKind::Frame,
                };
                let mut n = Node::new(kind, name);
                // Frames clip unless "clip content" is off; sections never clip.
                let clips = ty != "SECTION" && nc.bool("frameMaskDisabled") != Some(true);
                n.flags.set(NodeFlags::CLIPS_CONTENT, clips);
                n
            }
            "GROUP" => Node::new(NodeKind::Group, name),
            "RECTANGLE" | "ROUNDED_RECTANGLE" => Node::new(NodeKind::Rectangle, name),
            "ELLIPSE" => Node::new(NodeKind::Ellipse(None), name),
            "LINE" => Node::new(NodeKind::Line, name),
            "TEXT" => match self.text(nc) {
                Some(t) => Node::new(NodeKind::Text(Box::new(t)), name),
                None => return Ok(()),
            },
            "VECTOR" | "STAR" | "REGULAR_POLYGON" | "BOOLEAN_OPERATION" => {
                Node::new(NodeKind::Vector(Arc::new(VectorData::default())), name)
            }
            "CANVAS" | "DOCUMENT" => return Ok(()),
            other => {
                *self.warnings.unknown.entry(other.to_owned()).or_default() += 1;
                Node::new(NodeKind::Frame, name)
            }
        };

        self.common(&mut node, nc);
        if self
            .doc
            .node(parent)
            .and_then(|p| p.ext.as_ref())
            .and_then(|e| e.auto_layout.as_ref())
            .is_some_and(|al| al.direction == LayoutDirection::Vertical)
            && let Some(e) = node.ext.as_mut()
        {
            let lc = &mut e.layout_child;
            std::mem::swap(&mut lc.h_sizing, &mut lc.v_sizing);
        }
        if let NodeKind::Vector(_) = node.kind {
            self.geometry(&mut node, nc);
        }
        let built = self
            .doc
            .build_child(parent, node)
            .map_err(|e| e.to_string())?;
        if ty == "SYMBOL" {
            self.symbols.insert(id, built);
        }

        // Boolean operations are drawn from their computed geometry; operands are not needed.
        if ty == "BOOLEAN_OPERATION" {
            return Ok(());
        }
        let own_kids = self.kids(id);
        if ty == "INSTANCE" && own_kids.is_empty() {
            let Some(symbol) = nc.get("symbolData").and_then(|s| guid(s.get("symbolID"))) else {
                return Ok(());
            };
            self.instances.push((built, symbol));
            if depth >= MAX_INSTANCE_DEPTH {
                self.warnings.deep_instances += 1;
                return Ok(());
            }
            let mut inner = overrides.cloned().unwrap_or_default();
            self.collect_overrides(original, &mut inner);
            for child in self.kids(symbol) {
                self.node(child, built, depth + 1, Some(&inner))?;
            }
        } else {
            for child in own_kids {
                self.node(child, built, depth, overrides)?;
            }
        }
        Ok(())
    }

    /// Single-level overrides (`guidPath` of one guid) are applied; deeper paths are counted.
    fn collect_overrides(&mut self, nc: &'a Value<'s>, out: &mut Overrides<'a, 's>) {
        let Some(data) = nc.get("symbolData") else {
            return;
        };
        for o in data.array("symbolOverrides") {
            let path = o.get("guidPath").map(|p| p.array("guids")).unwrap_or(&[]);
            match path {
                [one] => {
                    if let Some(g) = guid(Some(one)) {
                        out.entry(g).or_default().push(o);
                    }
                }
                _ => self.warnings.overrides += 1,
            }
        }
    }

    fn link_instances(&mut self) {
        for (instance, symbol) in std::mem::take(&mut self.instances) {
            let Some(&component) = self.symbols.get(&symbol) else {
                continue;
            };
            let _ = self.doc.apply(Op::Set {
                id: instance,
                value: Prop::Kind(NodeKind::Instance(Box::new(InstanceData {
                    component: ComponentRef::Local(component),
                    overrides: Box::new([]),
                }))),
            });
        }
    }

    fn common(&mut self, n: &mut Node, nc: &Value) {
        if let Some(t) = nc.get("transform") {
            let m = |k| t.f32(k).unwrap_or(0.0);
            n.transform = Affine([
                t.f32("m00").unwrap_or(1.0),
                m("m10"),
                m("m01"),
                t.f32("m11").unwrap_or(1.0),
                m("m02"),
                m("m12"),
            ]);
        }
        if let Some(s) = nc.get("size") {
            n.size = ude_core::Size::new(
                s.f32("x").unwrap_or(0.0).max(0.0),
                s.f32("y").unwrap_or(0.0).max(0.0),
            );
        }
        n.opacity = nc.f32("opacity").unwrap_or(1.0).clamp(0.0, 1.0);
        n.flags
            .set(NodeFlags::VISIBLE, nc.bool("visible").unwrap_or(true));
        if nc.bool("mask") == Some(true) {
            n.flags.set(NodeFlags::IS_MASK, true);
        }
        if let Some(b) = nc.enum_("blendMode") {
            n.blend = blend(b);
        }
        if !matches!(n.kind, NodeKind::Text(_)) {
            n.fills = self.paints(nc.array("fillPaints"));
        }
        let radius = nc.f32("cornerRadius").unwrap_or(0.0);
        n.corner_radius = radius.max(0.0);
        if nc.bool("rectangleCornerRadiiIndependent") == Some(true) {
            let r = |k| nc.f32(k).unwrap_or(radius).max(0.0);
            n.ext_mut().corners = Some(CornerRadii {
                radii: [
                    r("rectangleTopLeftCornerRadius"),
                    r("rectangleTopRightCornerRadius"),
                    r("rectangleBottomRightCornerRadius"),
                    r("rectangleBottomLeftCornerRadius"),
                ],
                smoothing: nc.f32("cornerSmoothing").unwrap_or(0.0),
            });
        }
        let strokes = self.paints(nc.array("strokePaints"));
        let weight = nc.f32("strokeWeight").unwrap_or(0.0);
        if !strokes.is_empty() && weight > 0.0 {
            let dashes: Vec<f32> = nc
                .array("dashPattern")
                .iter()
                .filter_map(|v| match v {
                    Value::Float(f) => Some(*f),
                    _ => None,
                })
                .collect();
            n.ext_mut().strokes = Some(Box::new(Stroke {
                paints: strokes,
                width: weight,
                align: match nc.enum_("strokeAlign") {
                    Some("INSIDE") => StrokeAlign::Inside,
                    Some("OUTSIDE") => StrokeAlign::Outside,
                    _ => StrokeAlign::Center,
                },
                miter_limit: nc.f32("miterLimit").unwrap_or(4.0),
                dashes: dashes.into(),
                ..Default::default()
            }));
        }
        let effects: Vec<Effect> = nc.array("effects").iter().filter_map(effect).collect();
        if !effects.is_empty() {
            n.ext_mut().effects = effects.into();
        }
        self.auto_layout(n, nc);
    }

    fn auto_layout(&mut self, n: &mut Node, nc: &Value) {
        let child_fill = nc.f32("stackChildPrimaryGrow").unwrap_or(0.0) > 0.0;
        let stretch = nc.enum_("stackChildAlignSelf") == Some("STRETCH");
        let absolute = nc.enum_("stackPositioning") == Some("ABSOLUTE");
        if child_fill || stretch || absolute {
            // Stored as primary → h, counter → v; `node` swaps them under a vertical parent.
            let lc = &mut n.ext_mut().layout_child;
            lc.absolute = absolute;
            if child_fill {
                lc.h_sizing = ChildSizing::Fill;
            }
            if stretch {
                lc.v_sizing = ChildSizing::Fill;
            }
        }
        let direction = match nc.enum_("stackMode") {
            Some("HORIZONTAL") => LayoutDirection::Horizontal,
            Some("VERTICAL") => LayoutDirection::Vertical,
            _ => return,
        };
        let pad = |k: &str, fallback: &str| nc.f32(k).or_else(|| nc.f32(fallback)).unwrap_or(0.0);
        let left = pad("stackHorizontalPadding", "stackPaddingLeft");
        let top = pad("stackVerticalPadding", "stackPaddingTop");
        let sizing = |k| match nc.enum_(k) {
            Some("RESIZE_TO_FIT" | "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE") => ContainerSizing::Hug,
            _ => ContainerSizing::Fixed,
        };
        let wrap = nc.enum_("stackWrap") == Some("WRAP");
        n.ext_mut().auto_layout = Some(Box::new(AutoLayout {
            direction: if wrap {
                LayoutDirection::Wrap
            } else {
                direction
            },
            gap: nc.f32("stackSpacing").unwrap_or(0.0),
            cross_gap: nc
                .f32("stackCounterSpacing")
                .unwrap_or(nc.f32("stackSpacing").unwrap_or(0.0)),
            padding: [
                top,
                nc.f32("stackPaddingRight").unwrap_or(left),
                nc.f32("stackPaddingBottom").unwrap_or(top),
                left,
            ],
            main_align: match nc.enum_("stackPrimaryAlignItems") {
                Some("CENTER") => MainAlign::Center,
                Some("MAX") => MainAlign::End,
                Some("SPACE_BETWEEN" | "SPACE_EVENLY") => MainAlign::SpaceBetween,
                _ => MainAlign::Start,
            },
            cross_align: match nc.enum_("stackCounterAlignItems") {
                Some("CENTER") => CrossAlign::Center,
                Some("MAX") => CrossAlign::End,
                Some("BASELINE") => CrossAlign::Baseline,
                Some("STRETCH") => CrossAlign::Stretch,
                _ => CrossAlign::Start,
            },
            primary_sizing: sizing("stackPrimarySizing"),
            counter_sizing: sizing("stackCounterSizing"),
        }));
    }

    /// Vector-like nodes use Figma's computed geometry (already in node space).
    fn geometry(&mut self, n: &mut Node, nc: &Value) {
        let mut paths = self.paths(nc.array("fillGeometry"));
        if paths.is_empty() {
            // Stroke-only vectors: draw the outlined stroke with the stroke paint.
            paths = self.paths(nc.array("strokeGeometry"));
            if let Some(s) = n.ext.as_mut().and_then(|e| e.strokes.take()) {
                n.fills = s.paints;
            }
        }
        n.kind = NodeKind::Vector(Arc::new(VectorData {
            paths: paths.into(),
        }));
    }

    fn paths(&self, geometry: &[Value]) -> Vec<VectorPath> {
        geometry
            .iter()
            .filter_map(|g| {
                let blob = self.blobs.get(g.u64("commandsBlob")? as usize)?;
                let path = commands(blob)?;
                Some(VectorPath {
                    path,
                    fill_rule: match g.enum_("windingRule") {
                        Some("ODD" | "EVENODD") => FillRule::EvenOdd,
                        _ => FillRule::NonZero,
                    },
                })
            })
            .collect()
    }

    fn paints(&mut self, list: &[Value]) -> Paints {
        let v: Vec<Paint> = list.iter().filter_map(|p| self.paint(p)).collect();
        if v.is_empty() {
            Paints::default()
        } else {
            Paints::new(v)
        }
    }

    fn paint(&mut self, p: &Value) -> Option<Paint> {
        let kind = match p.enum_("type")? {
            "SOLID" => PaintKind::Solid(color(p.get("color"))?),
            t @ ("GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR"
            | "GRADIENT_DIAMOND") => {
                let stops: Vec<GradientStop> = p
                    .array("stops")
                    .iter()
                    .filter_map(|s| {
                        Some(GradientStop {
                            offset: s.f32("position").unwrap_or(0.0),
                            color: color(s.get("color"))?,
                        })
                    })
                    .collect();
                // Figma's transform maps the node's unit box into gradient space; ours is the
                // inverse (gradient space → unit box).
                let transform = matrix(p.get("transform"))
                    .and_then(|m| m.inverse())
                    .unwrap_or(Affine::IDENTITY);
                PaintKind::Gradient(Arc::new(Gradient {
                    kind: match t {
                        "GRADIENT_RADIAL" => GradientKind::Radial,
                        "GRADIENT_ANGULAR" => GradientKind::Angular,
                        "GRADIENT_DIAMOND" => GradientKind::Diamond,
                        _ => GradientKind::Linear,
                    },
                    stops: stops.into(),
                    transform,
                }))
            }
            "IMAGE" => {
                let hash = p.get("image").and_then(|i| i.bytes("hash"))?;
                let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
                let asset = self.image(&hex)?;
                PaintKind::Image {
                    asset,
                    mode: match p.enum_("imageScaleMode") {
                        Some("FIT") => ImageScaleMode::Fit,
                        Some("TILE") => ImageScaleMode::Tile,
                        Some("STRETCH") => ImageScaleMode::Crop,
                        _ => ImageScaleMode::Fill,
                    },
                    transform: None,
                }
            }
            _ => return None,
        };
        Some(Paint {
            kind,
            opacity: p.f32("opacity").unwrap_or(1.0),
            blend: p.enum_("blendMode").map_or(BlendMode::Normal, blend),
            visible: p.bool("visible").unwrap_or(true),
        })
    }

    fn image(&mut self, hex: &str) -> Option<AssetId> {
        if let Some(&a) = self.image_assets.get(hex) {
            return Some(a);
        }
        let Some(bytes) = self.images.remove(hex) else {
            self.warnings.missing_images += 1;
            return None;
        };
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?;
        let codec = match reader.format()? {
            image::ImageFormat::Png => Codec::Png,
            image::ImageFormat::Jpeg => Codec::Jpeg,
            image::ImageFormat::WebP => Codec::WebP,
            // GIFs are stored as PNG (first frame) so every image codec stays supported.
            image::ImageFormat::Gif => {
                let img = image::load_from_memory(&bytes).ok()?;
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .ok()?;
                let id = self.insert_image(png, Codec::Png, img.width(), img.height());
                self.image_assets.insert(hex.to_owned(), id);
                return Some(id);
            }
            _ => return None,
        };
        let (w, h) = reader.into_dimensions().ok()?;
        let id = self.insert_image(bytes, codec, w, h);
        self.image_assets.insert(hex.to_owned(), id);
        Some(id)
    }

    fn insert_image(&mut self, bytes: Vec<u8>, codec: Codec, w: u32, h: u32) -> AssetId {
        self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(&bytes).as_bytes()),
            kind: AssetKind::Image {
                width: w,
                height: h,
            },
            source: AssetSource::Memory {
                bytes: bytes.into(),
                codec,
            },
        })
    }

    fn text(&mut self, nc: &Value) -> Option<TextData> {
        let td = nc.get("textData");
        let content = td
            .and_then(|t| t.str("characters"))
            .or_else(|| nc.str("characters"))
            .unwrap_or("")
            .to_owned();
        let base = self.text_style(nc, None)?;
        let mut styles = vec![base];
        let mut runs = Vec::new();
        let ids: Vec<u64> = td
            .map(|t| t.array("characterStyleIDs"))
            .unwrap_or(&[])
            .iter()
            .map(|v| match v {
                Value::Uint(u) => *u,
                _ => 0,
            })
            .collect();
        let table = td.map(|t| t.array("styleOverrideTable")).unwrap_or(&[]);
        if ids.iter().any(|&i| i != 0) {
            // Style ids are per UTF-16 unit; map them to byte ranges of runs.
            let mut index_of: HashMap<u64, u16> = HashMap::from([(0, 0)]);
            let mut start = 0usize;
            let mut current = None;
            let mut unit = 0usize;
            for (byte, ch) in content.char_indices() {
                let id = ids.get(unit).copied().unwrap_or(0);
                let style = match index_of.get(&id) {
                    Some(&s) => s,
                    None => {
                        let ov = table.iter().find(|t| t.u64("styleID") == Some(id));
                        let s = ov
                            .and_then(|o| self.text_style(o, Some(&styles[0])))
                            .unwrap_or_else(|| styles[0].clone());
                        styles.push(s);
                        let i = (styles.len() - 1) as u16;
                        index_of.insert(id, i);
                        i
                    }
                };
                if current != Some(style) {
                    if let Some(s) = current {
                        runs.push(TextRun {
                            range: start as u32..byte as u32,
                            style: s,
                        });
                    }
                    start = byte;
                    current = Some(style);
                }
                unit += ch.len_utf16();
            }
            if let Some(s) = current {
                runs.push(TextRun {
                    range: start as u32..content.len() as u32,
                    style: s,
                });
            }
        } else {
            runs.push(TextRun {
                range: 0..content.len() as u32,
                style: 0,
            });
        }
        Some(TextData {
            content: content.into(),
            runs: runs.into(),
            styles: styles.into(),
            sizing: match nc.enum_("textAutoResize") {
                Some("WIDTH_AND_HEIGHT") => TextSizing::AutoWidth,
                Some("HEIGHT") => TextSizing::AutoHeight,
                Some("TRUNCATE") => TextSizing::Truncate,
                _ => TextSizing::Fixed,
            },
            align_h: match nc.enum_("textAlignHorizontal") {
                Some("CENTER") => TextAlignH::Center,
                Some("RIGHT") => TextAlignH::Right,
                Some("JUSTIFIED") => TextAlignH::Justify,
                _ => TextAlignH::Left,
            },
            align_v: match nc.enum_("textAlignVertical") {
                Some("CENTER") => TextAlignV::Center,
                Some("BOTTOM") => TextAlignV::Bottom,
                _ => TextAlignV::Top,
            },
            paragraph_spacing: nc.f32("paragraphSpacing").unwrap_or(0.0),
        })
    }

    /// Text style from a node or an override entry; unset fields come from `base`.
    fn text_style(&mut self, v: &Value, base: Option<&TextStyle>) -> Option<TextStyle> {
        let font = v.get("fontName");
        let family = font
            .and_then(|f| f.str("family"))
            .map(|f| self.doc.strings.intern(f));
        let face = font.and_then(|f| f.str("style"));
        let (weight, italic) = face.map_or((400, false), ude_core::typography::parse_font_style);
        let fills = self.paints(v.array("fillPaints"));
        let defaults = TextStyle {
            family: Symbol::EMPTY,
            weight: 400,
            italic: false,
            size: 12.0,
            line_height: LineHeight::Auto,
            letter_spacing: LetterSpacing::Normal,
            decoration: TextDecoration::None,
            case: TextCase::Original,
            fills: Paints::new(vec![Paint::solid(Color::BLACK)]),
        };
        let b = base.unwrap_or(&defaults);
        Some(TextStyle {
            family: family.unwrap_or(b.family),
            weight: if face.is_some() { weight } else { b.weight },
            italic: if face.is_some() { italic } else { b.italic },
            size: v.f32("fontSize").unwrap_or(b.size),
            line_height: match v.get("lineHeight") {
                Some(lh) => match (lh.enum_("units"), lh.f32("value")) {
                    (Some("PIXELS"), Some(px)) => LineHeight::Px(px),
                    (Some("PERCENT"), Some(p)) if (p - 100.0).abs() > 0.01 => {
                        LineHeight::Percent(p)
                    }
                    (Some("RAW"), Some(r)) => LineHeight::Percent(r * 100.0),
                    _ => LineHeight::Auto,
                },
                None => b.line_height,
            },
            letter_spacing: match v.get("letterSpacing") {
                Some(ls) => match (ls.enum_("units"), ls.f32("value")) {
                    (Some("PIXELS"), Some(px)) if px != 0.0 => LetterSpacing::Px(px),
                    (Some("PERCENT"), Some(p)) if p != 0.0 => LetterSpacing::Percent(p),
                    _ => LetterSpacing::Normal,
                },
                None => b.letter_spacing,
            },
            decoration: match v.enum_("textDecoration") {
                Some("UNDERLINE") => TextDecoration::Underline,
                Some("STRIKETHROUGH") => TextDecoration::Strikethrough,
                Some(_) => TextDecoration::None,
                None => b.decoration,
            },
            case: match v.enum_("textCase") {
                Some("UPPER") => TextCase::Upper,
                Some("LOWER") => TextCase::Lower,
                Some("TITLE") => TextCase::Title,
                Some("SMALL_CAPS" | "SMALL_CAPS_FORCED") => TextCase::SmallCaps,
                Some(_) => TextCase::Original,
                None => b.case,
            },
            fills: if fills.is_empty() {
                b.fills.clone()
            } else {
                fills
            },
        })
    }
}

/// Copy of `nc` with the fields of each override replacing its own.
fn patch<'s>(nc: &Value<'s>, overrides: &[&Value<'s>]) -> Value<'s> {
    let Value::Object(fields) = nc else {
        return nc.clone();
    };
    let mut fields = fields.clone();
    for o in overrides {
        let Value::Object(ov) = o else { continue };
        for (k, v) in ov {
            if matches!(*k, "guidPath" | "guid") {
                continue;
            }
            match fields.iter_mut().find(|(fk, _)| fk == k) {
                Some(slot) => slot.1 = v.clone(),
                None => fields.push((k, v.clone())),
            }
        }
    }
    Value::Object(fields)
}

/// Figma vector commands: 0 close, 1 move, 2 line, 3 quad, 4 cubic; f32 LE coordinates.
fn commands(blob: &[u8]) -> Option<PathData> {
    let mut b = ude_core::geometry::PathBuilder::default();
    let mut i = 0;
    let f = |i: usize| -> Option<f32> {
        Some(f32::from_le_bytes(blob.get(i..i + 4)?.try_into().ok()?))
    };
    let mut any = false;
    while i < blob.len() {
        let cmd = blob[i];
        i += 1;
        match cmd {
            0 => {
                b.close();
            }
            1 => {
                b.move_to(f(i)?, f(i + 4)?);
                i += 8;
                any = true;
            }
            2 => {
                b.line_to(f(i)?, f(i + 4)?);
                i += 8;
            }
            3 => {
                b.quad_to(f(i)?, f(i + 4)?, f(i + 8)?, f(i + 12)?);
                i += 16;
            }
            4 => {
                b.cubic_to(
                    f(i)?,
                    f(i + 4)?,
                    f(i + 8)?,
                    f(i + 12)?,
                    f(i + 16)?,
                    f(i + 20)?,
                );
                i += 24;
            }
            _ => return None,
        }
    }
    any.then(|| b.build())
}

fn color(v: Option<&Value>) -> Option<Color> {
    let v = v?;
    Some(Color {
        r: v.f32("r")?,
        g: v.f32("g")?,
        b: v.f32("b")?,
        a: v.f32("a").unwrap_or(1.0),
    })
}

fn matrix(v: Option<&Value>) -> Option<Affine> {
    let t = v?;
    Some(Affine([
        t.f32("m00")?,
        t.f32("m10")?,
        t.f32("m01")?,
        t.f32("m11")?,
        t.f32("m02")?,
        t.f32("m12")?,
    ]))
}

fn effect(e: &Value) -> Option<Effect> {
    let visible = e.bool("visible").unwrap_or(true);
    let radius = e.f32("radius").unwrap_or(0.0);
    let shadow = || {
        Some(Shadow {
            color: color(e.get("color"))?,
            offset: e.get("offset").map_or(Point::new(0.0, 0.0), |o| {
                Point::new(o.f32("x").unwrap_or(0.0), o.f32("y").unwrap_or(0.0))
            }),
            blur: radius,
            spread: e.f32("spread").unwrap_or(0.0),
            blend: e.enum_("blendMode").map_or(BlendMode::Normal, blend),
            visible,
        })
    };
    Some(match e.enum_("type")? {
        "DROP_SHADOW" => Effect::DropShadow(shadow()?),
        "INNER_SHADOW" => Effect::InnerShadow(shadow()?),
        "FOREGROUND_BLUR" | "LAYER_BLUR" => Effect::LayerBlur { radius, visible },
        "BACKGROUND_BLUR" => Effect::BackgroundBlur { radius, visible },
        _ => return None,
    })
}

fn blend(b: &str) -> BlendMode {
    match b {
        "PASS_THROUGH" => BlendMode::PassThrough,
        "DARKEN" => BlendMode::Darken,
        "MULTIPLY" => BlendMode::Multiply,
        "LINEAR_BURN" => BlendMode::LinearBurn,
        "COLOR_BURN" => BlendMode::ColorBurn,
        "LIGHTEN" => BlendMode::Lighten,
        "SCREEN" => BlendMode::Screen,
        "LINEAR_DODGE" => BlendMode::LinearDodge,
        "COLOR_DODGE" => BlendMode::ColorDodge,
        "OVERLAY" => BlendMode::Overlay,
        "SOFT_LIGHT" => BlendMode::SoftLight,
        "HARD_LIGHT" => BlendMode::HardLight,
        "DIFFERENCE" => BlendMode::Difference,
        "EXCLUSION" => BlendMode::Exclusion,
        "HUE" => BlendMode::Hue,
        "SATURATION" => BlendMode::Saturation,
        "COLOR" => BlendMode::Color,
        "LUMINOSITY" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

#[cfg(test)]
mod tests;
