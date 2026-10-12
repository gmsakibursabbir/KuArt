//! Sketch import (`.sketch`, format v43+): a ZIP with `document.json`, one `pages/<id>.json`
//! per page and `images/<hash>`. Layer frames are relative to the parent; shape paths are
//! curve points normalised to the layer frame.
//!
//! Mapped: pages, artboards, groups, shape groups (combined paths), rectangles, ovals,
//! polygons/stars/triangles/paths, text with attributed runs, bitmaps, symbol masters and
//! instances (expanded, with text overrides), fills (colour, gradient, image), borders,
//! shadows, blurs, opacity, blend modes, rotation and flips.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use ude_core::assets::{AssetEntry, AssetId, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::PathBuilder;
use ude_core::scene::{
    ComponentInfo, ComponentRef, InstanceData, NodeFlags, VectorData, VectorPath,
};
use ude_core::styles::{
    BlendMode, Color, Effect, Gradient, GradientKind, GradientStop, ImageScaleMode, Paint,
    PaintKind, Paints, Shadow, Stroke, StrokeAlign,
};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextAlignV, TextCase, TextData, TextDecoration, TextRun,
    TextSizing, TextStyle, parse_font_style,
};
use ude_core::{Affine, Document, FillRule, Node, NodeId, NodeKind, Point, SourceFormat, Symbol};
use ude_format_zip::Zip;

const MAX_ENTRY: usize = 512 << 20;
const MAX_DEPTH: usize = 64;

pub struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
}

pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    if !Zip::is_zip(bytes) {
        return Err("not a Sketch file (legacy pre-43 Sketch files are not supported)".into());
    }
    let zip = Zip::open(bytes)?;
    let json = |path: &str| -> Result<Option<Value>, String> {
        match zip.read(path, MAX_ENTRY)? {
            Some(b) => serde_json::from_slice(&b)
                .map(Some)
                .map_err(|e| format!("{path}: {e}")),
            None => Ok(None),
        }
    };
    let document =
        json("document.json")?.ok_or("the file has no document.json: not a Sketch file")?;
    let mut pages = Vec::new();
    for r in document
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(path) = r.get("_ref").and_then(Value::as_str) else {
            continue;
        };
        let path = if path.ends_with(".json") {
            path.to_owned()
        } else {
            format!("{path}.json")
        };
        if let Some(p) = json(&path)? {
            pages.push(p);
        }
    }
    if pages.is_empty() {
        return Err("the file has no pages".into());
    }

    let mut b = Builder {
        doc: Document::new(),
        zip: &zip,
        masters: HashMap::new(),
        master_nodes: HashMap::new(),
        instances: Vec::new(),
        images: HashMap::new(),
        warnings: Warnings::default(),
    };
    // Symbol masters from every page plus library symbols, by symbolID.
    for p in &pages {
        collect_masters(p, &mut b.masters);
    }
    for f in document
        .get("foreignSymbols")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(m) = f.get("symbolMaster") {
            collect_masters(m, &mut b.masters);
        }
    }
    b.doc.meta.title = name.to_owned();
    b.doc.meta.source = Some(SourceFormat::Sketch);
    let root = b.doc.root();
    for p in &pages {
        let name = b.doc.strings.intern(text(p, "name").unwrap_or("Page"));
        let page = b
            .doc
            .build_child(root, Node::new(NodeKind::Page, name))
            .map_err(|e| e.to_string())?;
        for l in layers(p) {
            b.layer(l, page, 0, &HashMap::new())?;
        }
    }
    b.link_instances();
    let warnings = b.warnings.finish();
    let mut doc = b.doc;
    let _ = doc.take_changes();
    Ok(Imported { doc, warnings })
}

fn collect_masters(v: &Value, out: &mut HashMap<String, Value>) {
    if text(v, "_class") == Some("symbolMaster")
        && let Some(id) = text(v, "symbolID")
    {
        out.insert(id.to_owned(), v.clone());
    }
    for l in layers(v) {
        collect_masters(l, out);
    }
}

fn layers(v: &Value) -> impl Iterator<Item = &Value> {
    v.get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key)?.as_str()
}

fn num(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|f| f as f32)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Sketch point string "{0.5, 1}".
fn point(v: Option<&Value>) -> Option<(f32, f32)> {
    let s = v?
        .as_str()?
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}');
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[derive(Default)]
struct Warnings {
    unresolved_symbols: usize,
    missing_images: usize,
    boolean_groups: usize,
    unknown: HashMap<String, usize>,
}

impl Warnings {
    fn finish(self) -> Vec<String> {
        let mut w = Vec::new();
        if self.unresolved_symbols > 0 {
            w.push(format!(
                "{} symbol instance(s) without a master in the file",
                self.unresolved_symbols
            ));
        }
        if self.missing_images > 0 {
            w.push(format!(
                "{} image(s) missing from the file",
                self.missing_images
            ));
        }
        if self.boolean_groups > 0 {
            w.push(format!(
                "{} shape group(s) with subtract/intersect/difference drawn as combined paths",
                self.boolean_groups
            ));
        }
        let mut unknown: Vec<_> = self.unknown.into_iter().collect();
        unknown.sort();
        for (kind, n) in unknown {
            w.push(format!(
                "{n} unsupported \u{201c}{kind}\u{201d} layer(s) skipped"
            ));
        }
        w
    }
}

struct Builder<'z> {
    doc: Document,
    zip: &'z Zip<'z>,
    masters: HashMap<String, Value>,
    master_nodes: HashMap<String, NodeId>,
    instances: Vec<(NodeId, String)>,
    images: HashMap<String, AssetId>,
    warnings: Warnings,
}

/// Text overrides inside a symbol instance: layer objectID → string.
type Overrides = HashMap<String, String>;

impl Builder<'_> {
    fn layer(
        &mut self,
        l: &Value,
        parent: NodeId,
        depth: usize,
        overrides: &Overrides,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let class = text(l, "_class").unwrap_or("");
        let frame = l.get("frame").unwrap_or(&Value::Null);
        let (x, y) = (
            num(frame, "x").unwrap_or(0.0),
            num(frame, "y").unwrap_or(0.0),
        );
        let (w, h) = (
            num(frame, "width").unwrap_or(0.0).max(0.0),
            num(frame, "height").unwrap_or(0.0).max(0.0),
        );
        let style = l.get("style").unwrap_or(&Value::Null);
        let mut children: Vec<&Value> = Vec::new();
        let mut instance_of = None;
        let mut node = match class {
            "artboard" | "symbolMaster" => {
                let kind = if class == "symbolMaster" {
                    NodeKind::Component(Box::new(ComponentInfo {
                        key: self.doc.strings.intern(text(l, "symbolID").unwrap_or("")),
                        description: Symbol::EMPTY,
                        variant_props: Box::new([]),
                    }))
                } else {
                    NodeKind::Frame
                };
                let mut n = Node::new(kind, Symbol::EMPTY);
                n.flags.set(NodeFlags::CLIPS_CONTENT, true);
                if flag(l, "hasBackgroundColor")
                    && let Some(c) = l.get("backgroundColor").and_then(color)
                {
                    n.fills = Paints::new(vec![Paint::solid(c)]);
                }
                children = layers(l).collect();
                n
            }
            "group" => {
                children = layers(l).collect();
                Node::new(NodeKind::Group, Symbol::EMPTY)
            }
            "symbolInstance" => {
                let id = text(l, "symbolID").unwrap_or("").to_owned();
                if let Some(m) = self.masters.get(&id).cloned() {
                    let n = Node::new(
                        NodeKind::Instance(Box::new(InstanceData {
                            component: ComponentRef::External(Symbol::EMPTY),
                            overrides: Box::new([]),
                        })),
                        Symbol::EMPTY,
                    );
                    instance_of = Some((id, m));
                    n
                } else {
                    self.warnings.unresolved_symbols += 1;
                    Node::new(NodeKind::Group, Symbol::EMPTY)
                }
            }
            "rectangle" => {
                let mut n = Node::new(NodeKind::Rectangle, Symbol::EMPTY);
                n.corner_radius = num(l, "fixedRadius")
                    .or_else(|| {
                        l.pointer("/points/0/cornerRadius")
                            .and_then(Value::as_f64)
                            .map(|v| v as f32)
                    })
                    .unwrap_or(0.0)
                    .max(0.0);
                n
            }
            "oval" => Node::new(NodeKind::Ellipse(None), Symbol::EMPTY),
            "shapePath" | "polygon" | "star" | "triangle" => {
                let path = curve_path(l, w, h).unwrap_or_default();
                vector(vec![VectorPath {
                    path,
                    fill_rule: winding(style),
                }])
            }
            "shapeGroup" => {
                let mut paths = Vec::new();
                for c in layers(l) {
                    let op = c
                        .get("booleanOperation")
                        .and_then(Value::as_i64)
                        .unwrap_or(-1);
                    if op > 0 {
                        self.warnings.boolean_groups += 1;
                    }
                    let f = c.get("frame").unwrap_or(&Value::Null);
                    let (cx, cy) = (num(f, "x").unwrap_or(0.0), num(f, "y").unwrap_or(0.0));
                    let (cw, ch) = (
                        num(f, "width").unwrap_or(0.0),
                        num(f, "height").unwrap_or(0.0),
                    );
                    let p = match text(c, "_class") {
                        Some("oval") => Some(ellipse_path(cx, cy, cw, ch)),
                        _ => curve_path(c, cw, ch).map(|p| offset(&p, cx, cy)),
                    };
                    if let Some(path) = p {
                        paths.push(VectorPath {
                            path,
                            fill_rule: winding(style),
                        });
                    }
                }
                vector(paths)
            }
            "text" => match self.text(l, overrides) {
                Some(t) => Node::new(NodeKind::Text(Box::new(t)), Symbol::EMPTY),
                None => return Ok(()),
            },
            "bitmap" => {
                let r = l
                    .pointer("/image/_ref")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.image(r) {
                    Some(asset) => Node::new(NodeKind::Raster { asset }, Symbol::EMPTY),
                    None => return Ok(()),
                }
            }
            "slice" | "MSImmutableHotspotLayer" | "hotspot" => return Ok(()),
            other => {
                *self.warnings.unknown.entry(other.to_owned()).or_default() += 1;
                return Ok(());
            }
        };
        node.name = self.doc.strings.intern(text(l, "name").unwrap_or(class));
        node.size = ude_core::Size::new(w, h);
        node.transform = layer_transform(l, x, y, w, h);
        node.flags.set(
            NodeFlags::VISIBLE,
            l.get("isVisible").and_then(Value::as_bool) != Some(false),
        );
        if flag(l, "hasClippingMask") {
            node.flags.set(NodeFlags::IS_MASK, true);
        }
        if let Some(ctx) = style.get("contextSettings") {
            node.opacity = num(ctx, "opacity").unwrap_or(1.0).clamp(0.0, 1.0);
            node.blend = blend(ctx.get("blendMode").and_then(Value::as_i64).unwrap_or(0));
        }
        if !matches!(
            node.kind,
            NodeKind::Text(_) | NodeKind::Group | NodeKind::Raster { .. }
        ) && !matches!(class, "artboard" | "symbolMaster" | "symbolInstance")
        {
            node.fills = self.fills(style);
            if let Some(s) = self.border(style) {
                node.ext_mut().strokes = Some(Box::new(s));
            }
        }
        let effects = effects(style);
        if !effects.is_empty() {
            node.ext_mut().effects = effects.into();
        }
        let id = self
            .doc
            .build_child(parent, node)
            .map_err(|e| e.to_string())?;
        if class == "symbolMaster"
            && let Some(sid) = text(l, "symbolID")
        {
            self.master_nodes.insert(sid.to_owned(), id);
        }
        if let Some((sid, master)) = instance_of {
            self.instances.push((id, sid));
            let mut inner = overrides.clone();
            for o in l
                .get("overrideValues")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(name), Some(value)) = (
                    text(o, "overrideName"),
                    o.get("value").and_then(Value::as_str),
                ) else {
                    continue;
                };
                // "<objectID>_stringValue"; nested paths use "a/b_stringValue" (last id wins).
                if let Some(target) = name.strip_suffix("_stringValue") {
                    let target = target.rsplit('/').next().unwrap_or(target);
                    inner.insert(target.to_owned(), value.to_owned());
                }
            }
            for c in layers(&master) {
                self.layer(c, id, depth + 1, &inner)?;
            }
            return Ok(());
        }
        for c in children {
            self.layer(c, id, depth + 1, overrides)?;
        }
        Ok(())
    }

    fn link_instances(&mut self) {
        for (id, sid) in std::mem::take(&mut self.instances) {
            if let Some(&c) = self.master_nodes.get(&sid) {
                let _ = self.doc.apply(ude_core::Op::Set {
                    id,
                    value: ude_core::Prop::Kind(NodeKind::Instance(Box::new(InstanceData {
                        component: ComponentRef::Local(c),
                        overrides: Box::new([]),
                    }))),
                });
            }
        }
    }

    fn text(&mut self, l: &Value, overrides: &Overrides) -> Option<TextData> {
        let attributed = l.get("attributedString")?;
        let original = text(attributed, "string").unwrap_or("");
        let id = text(l, "do_objectID").unwrap_or("");
        let overridden = overrides.get(id);
        let content = overridden.map_or(original, String::as_str).to_owned();
        let attrs: Vec<&Value> = attributed
            .get("attributes")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let mut styles: Vec<TextStyle> = Vec::new();
        let mut runs = Vec::new();
        let mut align = TextAlignH::Left;
        // Runs are in UTF-16 units; an override replaces the text, so it takes the first style.
        let units: Vec<usize> = content
            .char_indices()
            .flat_map(|(i, c)| std::iter::repeat_n(i, c.len_utf16()))
            .collect();
        let byte_at = |u: usize| units.get(u).copied().unwrap_or(content.len());
        for a in &attrs {
            let at = a.get("attributes").unwrap_or(&Value::Null);
            let s = self.text_style(at);
            if let Some(p) = at.get("paragraphStyle") {
                align = match p.get("alignment").and_then(Value::as_i64) {
                    Some(1) => TextAlignH::Right,
                    Some(2) => TextAlignH::Center,
                    Some(3) => TextAlignH::Justify,
                    _ => TextAlignH::Left,
                };
            }
            let index = match styles.iter().position(|x| *x == s) {
                Some(i) => i,
                None => {
                    styles.push(s);
                    styles.len() - 1
                }
            };
            if overridden.is_some() {
                break;
            }
            let start = a.get("location").and_then(Value::as_u64).unwrap_or(0) as usize;
            let len = a.get("length").and_then(Value::as_u64).unwrap_or(0) as usize;
            let (b0, b1) = (byte_at(start), byte_at(start + len));
            if b1 > b0 {
                runs.push(TextRun {
                    range: b0 as u32..b1 as u32,
                    style: index as u16,
                });
            }
        }
        if styles.is_empty() {
            styles.push(self.text_style(&Value::Null));
        }
        if runs.is_empty() {
            runs.push(TextRun {
                range: 0..content.len() as u32,
                style: 0,
            });
        }
        Some(TextData {
            content: content.into(),
            runs: runs.into(),
            styles: styles.into(),
            sizing: match l.get("textBehaviour").and_then(Value::as_i64) {
                Some(0) => TextSizing::AutoWidth,
                Some(1) => TextSizing::AutoHeight,
                _ => TextSizing::Fixed,
            },
            align_h: align,
            align_v: match l.get("verticalAlignment").and_then(Value::as_i64) {
                Some(1) => TextAlignV::Center,
                Some(2) => TextAlignV::Bottom,
                _ => TextAlignV::Top,
            },
            paragraph_spacing: 0.0,
        })
    }

    fn text_style(&mut self, at: &Value) -> TextStyle {
        let font = at
            .pointer("/MSAttributedStringFontAttribute/attributes")
            .unwrap_or(&Value::Null);
        // PostScript name "Inter-SemiBoldItalic" → family "Inter", style "SemiBoldItalic".
        let ps = text(font, "name").unwrap_or("");
        let (family, face) = ps.split_once('-').unwrap_or((ps, "Regular"));
        let family = split_camel(family);
        let (weight, italic) = parse_font_style(face);
        let fill = at
            .get("MSAttributedStringColorAttribute")
            .and_then(color)
            .unwrap_or(Color::BLACK);
        let line_height = at
            .pointer("/paragraphStyle/maximumLineHeight")
            .and_then(Value::as_f64)
            .filter(|&v| v > 0.0)
            .map_or(LineHeight::Auto, |v| LineHeight::Px(v as f32));
        TextStyle {
            family: self.doc.strings.intern(&family),
            weight,
            italic,
            size: num(font, "size").unwrap_or(12.0),
            line_height,
            letter_spacing: match num(at, "kerning") {
                Some(k) if k != 0.0 => LetterSpacing::Px(k),
                _ => LetterSpacing::Normal,
            },
            decoration: if at
                .get("underlineStyle")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                != 0
            {
                TextDecoration::Underline
            } else if at
                .get("strikethroughStyle")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                != 0
            {
                TextDecoration::Strikethrough
            } else {
                TextDecoration::None
            },
            case: match at
                .get("MSAttributedStringTextTransformAttribute")
                .and_then(Value::as_i64)
            {
                Some(1) => TextCase::Upper,
                Some(2) => TextCase::Lower,
                _ => TextCase::Original,
            },
            fills: Paints::new(vec![Paint::solid(fill)]),
        }
    }

    fn fills(&mut self, style: &Value) -> Paints {
        let v: Vec<Paint> = style
            .get("fills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|f| self.paint(f))
            .collect();
        if v.is_empty() {
            Paints::default()
        } else {
            Paints::new(v)
        }
    }

    fn paint(&mut self, f: &Value) -> Option<Paint> {
        let kind = match f.get("fillType").and_then(Value::as_i64).unwrap_or(0) {
            0 => PaintKind::Solid(color(f.get("color")?)?),
            1 => {
                let g = f.get("gradient")?;
                let stops: Vec<GradientStop> = g
                    .get("stops")?
                    .as_array()?
                    .iter()
                    .filter_map(|s| {
                        Some(GradientStop {
                            offset: num(s, "position")?,
                            color: color(s.get("color")?)?,
                        })
                    })
                    .collect();
                let (x1, y1) = point(g.get("from")).unwrap_or((0.5, 0.0));
                let (x2, y2) = point(g.get("to")).unwrap_or((0.5, 1.0));
                let (kind, transform) = match g.get("gradientType").and_then(Value::as_i64) {
                    Some(1) => {
                        let r = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();
                        (
                            GradientKind::Radial,
                            Affine([2.0 * r, 0.0, 0.0, 2.0 * r, x1 - r, y1 - r]),
                        )
                    }
                    Some(2) => (GradientKind::Angular, Affine::IDENTITY),
                    _ => {
                        let (dx, dy) = (x2 - x1, y2 - y1);
                        (
                            GradientKind::Linear,
                            Affine([dx, dy, -dy, dx, x1 + 0.5 * dy, y1 - 0.5 * dx]),
                        )
                    }
                };
                PaintKind::Gradient(Arc::new(Gradient {
                    kind,
                    stops: stops.into(),
                    transform,
                }))
            }
            4 | 5 => {
                let r = f.pointer("/image/_ref").and_then(Value::as_str)?;
                let asset = self.image(r)?;
                PaintKind::Image {
                    asset,
                    mode: match f.get("patternFillType").and_then(Value::as_i64) {
                        Some(0) => ImageScaleMode::Tile,
                        Some(3) => ImageScaleMode::Fit,
                        _ => ImageScaleMode::Fill,
                    },
                    transform: None,
                }
            }
            _ => return None,
        };
        Some(Paint {
            kind,
            opacity: f
                .pointer("/contextSettings/opacity")
                .and_then(Value::as_f64)
                .unwrap_or(1.0) as f32,
            blend: BlendMode::Normal,
            visible: f.get("isEnabled").and_then(Value::as_bool) != Some(false),
        })
    }

    fn border(&mut self, style: &Value) -> Option<Stroke> {
        let b = style
            .get("borders")?
            .as_array()?
            .iter()
            .find(|b| b.get("isEnabled").and_then(Value::as_bool) != Some(false))?;
        let paint = self.paint(b)?;
        let width = num(b, "thickness").unwrap_or(1.0);
        let dashes: Vec<f32> = style
            .pointer("/borderOptions/dashPattern")
            .and_then(Value::as_array)
            .map(|d| {
                d.iter()
                    .filter_map(Value::as_f64)
                    .map(|v| v as f32)
                    .collect()
            })
            .unwrap_or_default();
        Some(Stroke {
            paints: Paints::new(vec![paint]),
            width,
            align: match b.get("position").and_then(Value::as_i64) {
                Some(1) => StrokeAlign::Inside,
                Some(2) => StrokeAlign::Outside,
                _ => StrokeAlign::Center,
            },
            miter_limit: 4.0,
            dashes: dashes.into(),
            ..Default::default()
        })
    }

    fn image(&mut self, r: &str) -> Option<AssetId> {
        if let Some(&a) = self.images.get(r) {
            return Some(a);
        }
        let candidates = [r.to_owned(), format!("{r}.png")];
        let bytes = candidates
            .iter()
            .find_map(|p| self.zip.read(p, MAX_ENTRY).ok().flatten());
        let Some(bytes) = bytes else {
            self.warnings.missing_images += 1;
            return None;
        };
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?;
        let (bytes, codec) = match reader.format()? {
            image::ImageFormat::Png => (bytes, Codec::Png),
            image::ImageFormat::Jpeg => (bytes, Codec::Jpeg),
            image::ImageFormat::WebP => (bytes, Codec::WebP),
            _ => {
                let img = image::load_from_memory(&bytes).ok()?;
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .ok()?;
                (png, Codec::Png)
            }
        };
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        let id = self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(&bytes).as_bytes()),
            kind: AssetKind::Image {
                width: w,
                height: h,
            },
            source: AssetSource::Memory {
                bytes: bytes.into(),
                codec,
            },
        });
        self.images.insert(r.to_owned(), id);
        Some(id)
    }
}

/// Frame position, then rotation (degrees, counter-clockwise) and flips about the centre.
fn layer_transform(l: &Value, x: f32, y: f32, w: f32, h: f32) -> Affine {
    let rot = num(l, "rotation").unwrap_or(0.0);
    let (fx, fy) = (flag(l, "isFlippedHorizontal"), flag(l, "isFlippedVertical"));
    if rot == 0.0 && !fx && !fy {
        return Affine::translate(x, y);
    }
    let (cx, cy) = (w / 2.0, h / 2.0);
    mul(
        Affine::translate(x + cx, y + cy),
        mul(
            Affine::rotate(-rot.to_radians()),
            mul(
                Affine::scale(if fx { -1.0 } else { 1.0 }, if fy { -1.0 } else { 1.0 }),
                Affine::translate(-cx, -cy),
            ),
        ),
    )
}

/// `a * b` (apply `b` first).
fn mul(a: Affine, b: Affine) -> Affine {
    let [a0, a1, a2, a3, a4, a5] = a.0;
    let [b0, b1, b2, b3, b4, b5] = b.0;
    Affine([
        a0 * b0 + a2 * b1,
        a1 * b0 + a3 * b1,
        a0 * b2 + a2 * b3,
        a1 * b2 + a3 * b3,
        a0 * b4 + a2 * b5 + a4,
        a1 * b4 + a3 * b5 + a5,
    ])
}

fn vector(paths: Vec<VectorPath>) -> Node {
    Node::new(
        NodeKind::Vector(Arc::new(VectorData {
            paths: paths.into(),
        })),
        Symbol::EMPTY,
    )
}

fn winding(style: &Value) -> FillRule {
    if style.get("windingRule").and_then(Value::as_i64) == Some(1) {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

/// Curve points (normalised to the frame) → path in layer pixels.
fn curve_path(l: &Value, w: f32, h: f32) -> Option<ude_core::PathData> {
    let pts = l.get("points")?.as_array()?;
    if pts.is_empty() {
        return None;
    }
    let get = |p: &Value, key: &str, has: &str| -> Option<(f32, f32)> {
        if !has.is_empty() && !flag(p, has) {
            return None;
        }
        point(p.get(key)).map(|(x, y)| (x * w, y * h))
    };
    let main = |p: &Value| get(p, "point", "").unwrap_or((0.0, 0.0));
    let mut b = PathBuilder::default();
    let (x0, y0) = main(&pts[0]);
    b.move_to(x0, y0);
    let closed = flag(l, "isClosed");
    let n = pts.len();
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let (a, c) = (&pts[i], &pts[(i + 1) % n]);
        let (end_x, end_y) = main(c);
        match (
            get(a, "curveFrom", "hasCurveFrom"),
            get(c, "curveTo", "hasCurveTo"),
        ) {
            (None, None) => {
                b.line_to(end_x, end_y);
            }
            (c1, c2) => {
                let c1 = c1.unwrap_or_else(|| main(a));
                let c2 = c2.unwrap_or((end_x, end_y));
                b.cubic_to(c1.0, c1.1, c2.0, c2.1, end_x, end_y);
            }
        }
    }
    if closed {
        b.close();
    }
    Some(b.build())
}

fn ellipse_path(x: f32, y: f32, w: f32, h: f32) -> ude_core::PathData {
    // Four cubic arcs (kappa = 0.5523).
    let (rx, ry, cx, cy) = (w / 2.0, h / 2.0, x + w / 2.0, y + h / 2.0);
    let (kx, ky) = (rx * 0.552_284_8, ry * 0.552_284_8);
    let mut b = PathBuilder::default();
    b.move_to(cx + rx, cy);
    b.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    b.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    b.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    b.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    b.close();
    b.build()
}

fn offset(p: &ude_core::PathData, dx: f32, dy: f32) -> ude_core::PathData {
    let mut b = PathBuilder::default();
    let mut pts = p.points().iter().map(|q| Point::new(q.x + dx, q.y + dy));
    for v in p.verbs() {
        let mut next = || pts.next().unwrap_or(Point::new(dx, dy));
        match v {
            ude_core::PathVerb::MoveTo => {
                let q = next();
                b.move_to(q.x, q.y);
            }
            ude_core::PathVerb::LineTo => {
                let q = next();
                b.line_to(q.x, q.y);
            }
            ude_core::PathVerb::QuadTo => {
                let (c, q) = (next(), next());
                b.quad_to(c.x, c.y, q.x, q.y);
            }
            ude_core::PathVerb::CubicTo => {
                let (c1, c2, q) = (next(), next(), next());
                b.cubic_to(c1.x, c1.y, c2.x, c2.y, q.x, q.y);
            }
            ude_core::PathVerb::Close => {
                b.close();
            }
        }
    }
    b.build()
}

/// Sketch colour: `{ "red": 0–1, "green", "blue", "alpha" }`.
fn color(c: &Value) -> Option<Color> {
    Some(Color {
        r: num(c, "red")?,
        g: num(c, "green")?,
        b: num(c, "blue")?,
        a: num(c, "alpha").unwrap_or(1.0),
    })
}

fn effects(style: &Value) -> Vec<Effect> {
    let mut out = Vec::new();
    for (key, inner) in [("shadows", false), ("innerShadows", true)] {
        for s in style
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(c) = s.get("color").and_then(color) else {
                continue;
            };
            let shadow = Shadow {
                color: c,
                offset: Point::new(
                    num(s, "offsetX").unwrap_or(0.0),
                    num(s, "offsetY").unwrap_or(0.0),
                ),
                blur: num(s, "blurRadius").unwrap_or(0.0),
                spread: num(s, "spread").unwrap_or(0.0),
                blend: BlendMode::Normal,
                visible: s.get("isEnabled").and_then(Value::as_bool) != Some(false),
            };
            out.push(if inner {
                Effect::InnerShadow(shadow)
            } else {
                Effect::DropShadow(shadow)
            });
        }
    }
    if let Some(b) = style.get("blur")
        && b.get("isEnabled").and_then(Value::as_bool) == Some(true)
    {
        let radius = num(b, "radius").unwrap_or(0.0);
        out.push(if b.get("type").and_then(Value::as_i64) == Some(3) {
            Effect::BackgroundBlur {
                radius,
                visible: true,
            }
        } else {
            Effect::LayerBlur {
                radius,
                visible: true,
            }
        });
    }
    out
}

/// Sketch blend mode numbers (CGBlendMode order).
fn blend(m: i64) -> BlendMode {
    match m {
        1 => BlendMode::Darken,
        2 => BlendMode::Multiply,
        3 => BlendMode::ColorBurn,
        4 => BlendMode::Lighten,
        5 => BlendMode::Screen,
        6 => BlendMode::ColorDodge,
        7 => BlendMode::Overlay,
        8 => BlendMode::SoftLight,
        9 => BlendMode::HardLight,
        10 => BlendMode::Difference,
        11 => BlendMode::Exclusion,
        12 => BlendMode::Hue,
        13 => BlendMode::Saturation,
        14 => BlendMode::Color,
        15 => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// "OpenSans" → "Open Sans" (PostScript family names drop spaces).
fn split_camel(s: &str) -> String {
    let mut out = String::new();
    let mut prev = ' ';
    for c in s.chars() {
        if c.is_ascii_uppercase() && prev.is_ascii_lowercase() {
            out.push(' ');
        }
        out.push(c);
        prev = c;
    }
    out
}

#[cfg(test)]
mod tests;
