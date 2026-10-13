//! Adobe XD import (`docs/format-strategy.md` §3). An `.xd` file is a UCF ZIP: `manifest`
//! lists artboards, each artboard's objects live in
//! `artwork/<artboard>/graphics/graphicContent.agc` (JSON), shared gradients and symbols in
//! `resources/graphics/graphicContent.agc`, and images in `resources/<uid>`.
//!
//! Mapped: artboards (frames), pasteboard objects, groups, symbols (expanded), rectangles,
//! ellipses, circles, lines, polygons, paths and compound paths, text with ranged styles,
//! solid/gradient/image fills, strokes, drop shadows, blurs, corner radii and opacity.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use ude_core::assets::{AssetEntry, AssetId, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::PathBuilder;
use ude_core::scene::{CornerRadii, NodeFlags, VectorData, VectorPath};
use ude_core::styles::{
    BlendMode, Color, Effect, Gradient, GradientKind, GradientStop, ImageScaleMode, Paint,
    PaintKind, Paints, Shadow, Stroke, StrokeAlign,
};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextAlignV, TextCase, TextData, TextDecoration, TextRun,
    TextSizing, TextStyle,
};
use ude_core::{Affine, Document, FillRule, Node, NodeId, NodeKind, Point, SourceFormat, Symbol};
use ude_format_zip::Zip;

const MAX_ENTRY: usize = 512 << 20;
const MAX_DEPTH: usize = 64;
/// Ascent used to place point text, whose origin is the first baseline (fraction of size).
const ASCENT: f32 = 0.8;

pub struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
}

pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    if !Zip::is_zip(bytes) {
        return Err("not an Adobe XD file (expected a ZIP container)".into());
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
    let manifest = json("manifest")?.ok_or("the file has no manifest: not an Adobe XD file")?;
    let resources = json("resources/graphics/graphicContent.agc")?.unwrap_or(Value::Null);
    let res = resources.get("resources").unwrap_or(&Value::Null);

    // Artboards: manifest entries named artboard-*, bounds from the manifest or resources.
    let mut boards = Vec::new();
    collect_artboards(&manifest, &mut boards);
    let res_boards = resources.get("artboards").or_else(|| res.get("artboards"));

    let mut b = Builder {
        doc: Document::new(),
        zip: &zip,
        gradients: res.get("gradients").cloned().unwrap_or(Value::Null),
        symbols: HashMap::new(),
        images: HashMap::new(),
        warnings: Warnings::default(),
        board: Affine::IDENTITY,
        parts: HashMap::new(),
        expanding: 0,
    };
    if let Some(list) = res
        .get("meta")
        .and_then(|m| m.get("ux"))
        .and_then(|u| u.get("symbols"))
        .and_then(Value::as_array)
    {
        for s in list {
            for key in ["id", "syncSourceGuid"] {
                if let Some(id) = s.get(key).and_then(Value::as_str) {
                    b.symbols.insert(id.to_owned(), s.clone());
                }
            }
            index_parts(s, &mut b.parts);
        }
    }
    b.doc.meta.title = name.to_owned();
    b.doc.meta.source = Some(SourceFormat::Xd);
    let root = b.doc.root();
    let page_name = b.doc.strings.intern("Page 1");
    let page = b
        .doc
        .build_child(root, Node::new(NodeKind::Page, page_name))
        .map_err(|e| e.to_string())?;

    for board in &boards {
        let mut bounds = board.bounds;
        if let Some(r) = res_boards.and_then(|a| a.get(&board.path)) {
            bounds = rect_of(r).or(bounds);
        }
        let agc = json(&format!(
            "artwork/{}/graphics/graphicContent.agc",
            board.path
        ))?;
        let ab = agc.as_ref().and_then(|a| {
            a.get("children")?
                .as_array()?
                .iter()
                .find(|c| c.get("type").and_then(Value::as_str) == Some("artboard"))
        });
        let (x, y, w, h) = bounds.unwrap_or((0.0, 0.0, 375.0, 812.0));
        let name = b.doc.strings.intern(&board.name);
        let mut frame = Node::new(NodeKind::Frame, name)
            .with_transform(Affine::translate(x, y))
            .with_size(w, h);
        frame.flags.set(NodeFlags::CLIPS_CONTENT, true);
        frame.fills = match ab.and_then(|a| a.get("style")).and_then(|s| s.get("fill")) {
            Some(fill) => b.paints(fill, (w, h)),
            None => Paints::new(vec![Paint::solid(Color::WHITE)]),
        };
        let id = b.doc.build_child(page, frame).map_err(|e| e.to_string())?;
        let kids = ab
            .and_then(|a| a.get("artboard"))
            .and_then(|a| a.get("children"))
            .and_then(Value::as_array);
        // Artboard contents are stored in canvas coordinates: make them artboard-relative.
        b.board = Affine::translate(-x, -y);
        for c in kids.into_iter().flatten() {
            b.object(c, id, 0)?;
        }
    }
    b.board = Affine::IDENTITY;
    // Objects on the pasteboard, outside every artboard.
    if let Some(p) = json("artwork/pasteboard/graphics/graphicContent.agc")? {
        for c in p
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            b.object(c, page, 0)?;
        }
    }
    if b.doc.children(page).is_empty() && boards.is_empty() {
        return Err("the file has no artboards".into());
    }
    let warnings = b.warnings.finish();
    let mut doc = b.doc;
    let _ = doc.take_changes();
    Ok(Imported { doc, warnings })
}

/// Indexes every typed object with an id below `v`.
fn index_parts(v: &Value, out: &mut HashMap<String, Value>) {
    match v {
        Value::Object(o) => {
            if let (Some(id), Some(_)) = (
                o.get("id").and_then(Value::as_str),
                o.get("type").and_then(Value::as_str),
            ) {
                out.entry(id.to_owned()).or_insert_with(|| v.clone());
            }
            for c in o.values() {
                index_parts(c, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| index_parts(c, out)),
        _ => {}
    }
}

struct Board {
    path: String,
    name: String,
    bounds: Option<(f32, f32, f32, f32)>,
}

fn collect_artboards(v: &Value, out: &mut Vec<Board>) {
    if let Some(path) = v.get("path").and_then(Value::as_str)
        && path.starts_with("artboard-")
    {
        out.push(Board {
            path: path.to_owned(),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Artboard")
                .to_owned(),
            bounds: v.get("uxdesign#bounds").and_then(rect_of),
        });
    }
    for c in v
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_artboards(c, out);
    }
}

fn rect_of(v: &Value) -> Option<(f32, f32, f32, f32)> {
    Some((
        num(v, "x")?,
        num(v, "y")?,
        num(v, "width")?,
        num(v, "height")?,
    ))
}

fn num(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|f| f as f32)
}

fn text<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key)?.as_str()
}

#[derive(Default)]
struct Warnings {
    repeat_grids: usize,
    masks: usize,
    unresolved_symbols: usize,
    missing_images: usize,
    unknown: HashMap<String, usize>,
}

impl Warnings {
    fn finish(self) -> Vec<String> {
        let mut w = Vec::new();
        let mut add = |n: usize, msg: &str| {
            if n > 0 {
                w.push(format!("{n} {msg}"));
            }
        };
        add(self.repeat_grids, "repeat grid(s) imported as groups");
        add(
            self.masks,
            "mask group(s): the mask shape is kept, masking is applied in phase 2",
        );
        add(
            self.unresolved_symbols,
            "symbol instance(s) without a master in the file",
        );
        add(
            self.missing_images,
            "image fill(s) reference images missing from the file",
        );
        let mut unknown: Vec<_> = self.unknown.into_iter().collect();
        unknown.sort();
        for (kind, n) in unknown {
            w.push(format!(
                "{n} unsupported \u{201c}{kind}\u{201d} object(s) skipped"
            ));
        }
        w
    }
}

struct Builder<'z> {
    doc: Document,
    zip: &'z Zip<'z>,
    gradients: Value,
    symbols: HashMap<String, Value>,
    images: HashMap<String, AssetId>,
    warnings: Warnings,
    /// Maps canvas to artboard coordinates for the artboard being read.
    board: Affine,
    /// Every layer inside the symbol masters (and their states), by id: newer files reference
    /// single master layers from instances.
    parts: HashMap<String, Value>,
    /// Nesting of layer references being expanded (guards against cycles).
    expanding: usize,
}

impl Builder<'_> {
    fn object(&mut self, o: &Value, parent: NodeId, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let ty = text(o, "type").unwrap_or("");
        let name = text(o, "name").unwrap_or(ty).to_owned();
        let style = o.get("style").unwrap_or(&Value::Null);
        let mut transform = o.get("transform").map_or(Affine::IDENTITY, |t| {
            Affine([
                num(t, "a").unwrap_or(1.0),
                num(t, "b").unwrap_or(0.0),
                num(t, "c").unwrap_or(0.0),
                num(t, "d").unwrap_or(1.0),
                num(t, "tx").unwrap_or(0.0),
                num(t, "ty").unwrap_or(0.0),
            ])
        });
        let mut children: Option<&Vec<Value>> = None;
        let resolved;
        let mut node = match ty {
            "group" | "symbol" => {
                if o.pointer("/meta/ux/repeatGrid").is_some() {
                    self.warnings.repeat_grids += 1;
                }
                children = o.pointer("/group/children").and_then(Value::as_array);
                Node::new(NodeKind::Group, Symbol::EMPTY)
            }
            "syncRef" => {
                let key = text(o, "syncSourceGuid")
                    .or_else(|| o.pointer("/meta/ux/symbolId").and_then(Value::as_str));
                match key.and_then(|k| self.symbols.get(k)) {
                    Some(s) => {
                        resolved = s.clone();
                        children = resolved
                            .pointer("/group/children")
                            .and_then(Value::as_array);
                    }
                    None => match key.and_then(|k| self.parts.get(k)).cloned() {
                        // A reference to one layer of a master: that layer, with the
                        // instance's own fields (children, style, text) on top.
                        Some(mut part) if self.expanding < 16 => {
                            if let (Some(p), Some(own)) = (part.as_object_mut(), o.as_object()) {
                                for (k, v) in own {
                                    if !matches!(k.as_str(), "type" | "syncSourceGuid" | "guid") {
                                        p.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                            self.expanding += 1;
                            let r = self.object(&part, parent, depth);
                            self.expanding -= 1;
                            return r;
                        }
                        _ => self.warnings.unresolved_symbols += 1,
                    },
                }
                Node::new(NodeKind::Group, Symbol::EMPTY)
            }
            "shape" => {
                let Some(shape) = o.get("shape") else {
                    return Ok(());
                };
                match self.shape(shape) {
                    Some((n, offset)) => {
                        transform = mul(transform, Affine::translate(offset.x, offset.y));
                        n
                    }
                    None => {
                        let kind = text(shape, "type").unwrap_or("shape").to_owned();
                        *self.warnings.unknown.entry(kind).or_default() += 1;
                        return Ok(());
                    }
                }
            }
            "text" => match self.text(o, style) {
                Some((n, (dx, dy))) => {
                    transform = mul(transform, Affine::translate(dx, dy));
                    n
                }
                None => return Ok(()),
            },
            "artboard" => return Ok(()),
            other => {
                *self.warnings.unknown.entry(other.to_owned()).or_default() += 1;
                return Ok(());
            }
        };
        node.name = self.doc.strings.intern(&name);
        // Top-level objects of an artboard: canvas → artboard coordinates.
        node.transform = if depth == 0 {
            mul(self.board, transform)
        } else {
            transform
        };
        node.flags.set(
            NodeFlags::VISIBLE,
            o.get("visible").and_then(Value::as_bool) != Some(false),
        );
        node.opacity = num(style, "opacity").unwrap_or(1.0).clamp(0.0, 1.0);
        if let Some(b) = text(style, "blendMode") {
            node.blend = blend(b);
        }
        if !matches!(node.kind, NodeKind::Group | NodeKind::Text(_)) {
            let size = (node.size.w, node.size.h);
            // Video fills keep their poster frame as an image fill (the video is not played).
            let media = o.pointer("/shape/meta/ux/mediaFill");
            if let Some(fill) = style.get("fill").or(media) {
                node.fills = self.paints(fill, size);
            }
            // Frosted glass: a background blur's fill opacity makes the fill see-through.
            let glass = style
                .get("filters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|f| {
                    text(f, "type") == Some("uxdesign#blur")
                        && f.get("visible").and_then(Value::as_bool) != Some(false)
                        && f.pointer("/params/backgroundEffect")
                            .and_then(Value::as_bool)
                            == Some(true)
                })
                .and_then(|f| f.get("params").and_then(|p| num(p, "fillOpacity")));
            if let Some(o) = glass {
                let mut paints = node.fills.as_slice().to_vec();
                for p in &mut paints {
                    p.opacity *= o;
                }
                node.fills = Paints::new(paints);
            }
            if let Some(s) = self.stroke(style.get("stroke"), size) {
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
        if let Some(mask) = o.get("mask") {
            // Keep the mask shape (flagged) so it survives editing and export.
            self.warnings.masks += 1;
            let before = self.doc.children(id).len();
            self.object(mask, id, depth + 1)?;
            if let Some(&m) = self.doc.children(id).get(before) {
                let mut flags = self.doc.node(m).map(|n| n.flags).unwrap_or_default();
                flags.set(NodeFlags::IS_MASK, true);
                let _ = self.doc.apply(ude_core::Op::Set {
                    id: m,
                    value: ude_core::Prop::Flags(flags),
                });
            }
        }
        for c in children.into_iter().flatten() {
            self.object(c, id, depth + 1)?;
        }
        Ok(())
    }

    /// Shape node and the offset of its box inside the object's coordinate space.
    fn shape(&mut self, s: &Value) -> Option<(Node, Point)> {
        let (kind, x, y, w, h) = match text(s, "type")? {
            "rect" => {
                let n = (
                    NodeKind::Rectangle,
                    num(s, "x").unwrap_or(0.0),
                    num(s, "y").unwrap_or(0.0),
                );
                (n.0, n.1, n.2, num(s, "width")?, num(s, "height")?)
            }
            "ellipse" => {
                let (rx, ry) = (num(s, "rx")?, num(s, "ry")?);
                let (cx, cy) = (num(s, "cx").unwrap_or(rx), num(s, "cy").unwrap_or(ry));
                (
                    NodeKind::Ellipse(None),
                    cx - rx,
                    cy - ry,
                    rx * 2.0,
                    ry * 2.0,
                )
            }
            "circle" => {
                let r = num(s, "r")?;
                let (cx, cy) = (num(s, "cx").unwrap_or(r), num(s, "cy").unwrap_or(r));
                (NodeKind::Ellipse(None), cx - r, cy - r, r * 2.0, r * 2.0)
            }
            "line" => {
                let mut p = PathBuilder::default();
                p.move_to(num(s, "x1")?, num(s, "y1")?);
                p.line_to(num(s, "x2")?, num(s, "y2")?);
                return Some(vector(p.build(), FillRule::NonZero));
            }
            "polygon" if s.get("points").is_some() => {
                let mut p = PathBuilder::default();
                for (i, pt) in s.get("points")?.as_array()?.iter().enumerate() {
                    let (x, y) = (num(pt, "x")?, num(pt, "y")?);
                    if i == 0 {
                        p.move_to(x, y);
                    } else {
                        p.line_to(x, y);
                    }
                }
                p.close();
                return Some(vector(p.build(), FillRule::NonZero));
            }
            "path" | "compound" | "polygon" => {
                let d = text(s, "path")?;
                let rule = if text(s, "fillRule") == Some("evenodd") {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
                return Some(vector(parse_path(d)?, rule));
            }
            _ => return None,
        };
        let mut n = Node::new(kind, Symbol::EMPTY).with_size(w.max(0.0), h.max(0.0));
        match s.get("r") {
            Some(Value::Array(r)) => {
                let r: Vec<f32> = r
                    .iter()
                    .filter_map(Value::as_f64)
                    .map(|v| v as f32)
                    .collect();
                if let [tl, tr, br, bl] = r[..] {
                    if tl == tr && tr == br && br == bl {
                        n.corner_radius = tl.max(0.0);
                    } else {
                        n.ext_mut().corners = Some(CornerRadii {
                            radii: [tl, tr, br, bl],
                            smoothing: 0.0,
                        });
                    }
                }
            }
            Some(v) => n.corner_radius = v.as_f64().unwrap_or(0.0).max(0.0) as f32,
            None => {}
        }
        Some((n, Point::new(x, y)))
    }

    /// Text node and the vertical offset from the object origin to the box top.
    fn text(&mut self, o: &Value, style: &Value) -> Option<(Node, (f32, f32))> {
        let t = o.get("text")?;
        let content = text(t, "rawText").unwrap_or("").to_owned();
        let font = style.get("font").unwrap_or(&Value::Null);
        let attrs = style.get("textAttributes").unwrap_or(&Value::Null);
        let base = self.text_style(font, style.get("fill"), attrs, None);
        let mut styles = vec![base];
        let mut runs = Vec::new();
        let ranged = o
            .pointer("/meta/ux/rangedStyles")
            .and_then(Value::as_array)
            .filter(|r| r.len() > 1);
        if let Some(ranged) = ranged {
            // Lengths count UTF-16 units.
            let mut start_byte = 0usize;
            let mut chars = content.char_indices().peekable();
            for r in ranged {
                let mut units = r.get("length").and_then(Value::as_u64).unwrap_or(0) as usize;
                let mut end_byte = start_byte;
                while units > 0 {
                    let Some((i, c)) = chars.next() else { break };
                    end_byte = i + c.len_utf8();
                    units = units.saturating_sub(c.len_utf16());
                }
                if end_byte > start_byte {
                    let s = self.ranged_style(r, &styles[0]);
                    let index = match styles.iter().position(|x| *x == s) {
                        Some(i) => i,
                        None => {
                            styles.push(s);
                            styles.len() - 1
                        }
                    };
                    runs.push(TextRun {
                        range: start_byte as u32..end_byte as u32,
                        style: index as u16,
                    });
                    start_byte = end_byte;
                }
            }
            if start_byte < content.len() {
                runs.push(TextRun {
                    range: start_byte as u32..content.len() as u32,
                    style: 0,
                });
            }
        } else {
            runs.push(TextRun {
                range: 0..content.len() as u32,
                style: 0,
            });
        }
        let frame = t.get("frame").unwrap_or(&Value::Null);
        // "autoHeight" text has a fixed width and grows downwards; like area text, its origin
        // is the top-left corner.
        let auto_height = text(frame, "type") == Some("autoHeight");
        let area = auto_height || text(frame, "type") == Some("area");
        let size = styles[0].size;
        let first_baseline = t
            .pointer("/paragraphs/0/lines/0/0/y")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        let data = TextData {
            content: content.into(),
            runs: runs.into(),
            styles: styles.into(),
            sizing: if auto_height {
                TextSizing::AutoHeight
            } else if area {
                TextSizing::Fixed
            } else {
                TextSizing::AutoWidth
            },
            align_h: match text(attrs, "paragraphAlign") {
                Some("center") => TextAlignH::Center,
                Some("right") => TextAlignH::Right,
                Some("justify") => TextAlignH::Justify,
                _ => TextAlignH::Left,
            },
            align_v: TextAlignV::Top,
            paragraph_spacing: num(attrs, "paragraphAfterSpacing").unwrap_or(0.0),
        };
        let (w, h) = if auto_height {
            // Height from the last laid-out baseline XD stored, plus the descent.
            let last = t
                .get("paragraphs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|p| {
                    p.get("lines")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .flat_map(|l| l.as_array().into_iter().flatten())
                .filter_map(|run| num(run, "y"))
                .fold(0.0_f32, f32::max);
            (num(frame, "width").unwrap_or(0.0), last + size * 0.4)
        } else if area {
            (
                num(frame, "width").unwrap_or(0.0),
                num(frame, "height").unwrap_or(0.0),
            )
        } else {
            // Point text has no stored box; estimate it so it is drawn, hit-tested and exported
            // (the editor refits it with real font metrics when edited).
            let lines = data.content.split('\n').collect::<Vec<_>>();
            let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
            let line_h = match data.styles[0].line_height {
                ude_core::typography::LineHeight::Px(px) => px,
                ude_core::typography::LineHeight::Percent(p) => size * p / 100.0,
                ude_core::typography::LineHeight::Auto => size * 1.2,
            };
            (longest * size * 0.6, lines.len() as f32 * line_h)
        };
        // Point text: the origin is on the first baseline. Area text: the origin is the top.
        let dy = if area {
            0.0
        } else {
            first_baseline - size * ASCENT
        };
        // Centred and right-aligned point text is anchored at its centre or right edge.
        let dx = match (area, data.align_h) {
            (false, TextAlignH::Center) => -w / 2.0,
            (false, TextAlignH::Right) => -w,
            _ => 0.0,
        };
        Some((
            Node::new(NodeKind::Text(Box::new(data)), Symbol::EMPTY).with_size(w, h),
            (dx, dy),
        ))
    }

    fn text_style(
        &mut self,
        font: &Value,
        fill: Option<&Value>,
        attrs: &Value,
        base: Option<&TextStyle>,
    ) -> TextStyle {
        let family = text(font, "family").map(|f| self.doc.strings.intern(f));
        let face = text(font, "style");
        let (weight, italic) = face.map_or((400, false), ude_core::typography::parse_font_style);
        let size = num(font, "size");
        let fills = match fill {
            Some(f) => self.paints(f, (0.0, 0.0)),
            None => Paints::default(),
        };
        let d = TextStyle {
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
        let b = base.unwrap_or(&d);
        TextStyle {
            family: family.unwrap_or(b.family),
            weight: if face.is_some() { weight } else { b.weight },
            italic: if face.is_some() { italic } else { b.italic },
            size: size.unwrap_or(b.size),
            line_height: match num(attrs, "lineHeight") {
                Some(px) if px > 0.0 => LineHeight::Px(px),
                _ => b.line_height,
            },
            // XD letter spacing is in 1/1000 em.
            letter_spacing: match num(attrs, "letterSpacing") {
                Some(v) if v != 0.0 => LetterSpacing::Percent(v / 10.0),
                _ => b.letter_spacing,
            },
            decoration: match (
                attrs.get("underline").and_then(Value::as_bool),
                attrs.get("strikethrough").and_then(Value::as_bool),
            ) {
                (Some(true), _) => TextDecoration::Underline,
                (_, Some(true)) => TextDecoration::Strikethrough,
                _ => b.decoration,
            },
            case: match text(attrs, "textTransform") {
                Some("uppercase") => TextCase::Upper,
                Some("lowercase") => TextCase::Lower,
                Some("titlecase") => TextCase::Title,
                _ => b.case,
            },
            fills: if fills.is_empty() {
                b.fills.clone()
            } else {
                fills
            },
        }
    }

    fn ranged_style(&mut self, r: &Value, base: &TextStyle) -> TextStyle {
        let font = serde_json::json!({
            "family": r.get("fontFamily"),
            "style": r.get("fontStyle"),
            "size": r.get("fontSize"),
        });
        let attrs = serde_json::json!({
            "letterSpacing": r.get("charSpacing"),
            "underline": r.get("underline"),
            "strikethrough": r.get("strikethrough"),
            "textTransform": r.get("textTransform"),
        });
        self.text_style(&font, r.get("fill"), &attrs, Some(base))
    }

    fn paints(&mut self, fill: &Value, size: (f32, f32)) -> Paints {
        match self.paint(fill, size) {
            Some(p) => Paints::new(vec![p]),
            None => Paints::default(),
        }
    }

    fn paint(&mut self, fill: &Value, size: (f32, f32)) -> Option<Paint> {
        let kind = match text(fill, "type")? {
            "solid" => PaintKind::Solid(color(fill.get("color")?)?),
            "gradient" => {
                let g = fill.get("gradient")?;
                let def = self.gradients.get(text(g, "ref")?)?;
                let stops: Vec<GradientStop> = def
                    .get("stops")?
                    .as_array()?
                    .iter()
                    .filter_map(|s| {
                        Some(GradientStop {
                            offset: num(s, "offset")?,
                            color: color(s.get("color")?)?,
                        })
                    })
                    .collect();
                // Coordinates are fractions of the box unless in user space (pixels).
                let user = text(g, "units") == Some("userSpaceOnUse");
                let (sx, sy) = if user {
                    (size.0.max(1e-3), size.1.max(1e-3))
                } else {
                    (1.0, 1.0)
                };
                let (kind, transform) = if text(def, "type") == Some("radial") {
                    let (cx, cy) = (
                        num(g, "cx").unwrap_or(0.5) / sx,
                        num(g, "cy").unwrap_or(0.5) / sy,
                    );
                    let (rx, ry) = (
                        num(g, "r").unwrap_or(0.5) / sx,
                        num(g, "r").unwrap_or(0.5) / sy,
                    );
                    (
                        GradientKind::Radial,
                        Affine([2.0 * rx, 0.0, 0.0, 2.0 * ry, cx - rx, cy - ry]),
                    )
                } else {
                    let (x1, y1) = (
                        num(g, "x1").unwrap_or(0.0) / sx,
                        num(g, "y1").unwrap_or(0.0) / sy,
                    );
                    let (x2, y2) = (
                        num(g, "x2").unwrap_or(1.0) / sx,
                        num(g, "y2").unwrap_or(0.0) / sy,
                    );
                    let (dx, dy) = (x2 - x1, y2 - y1);
                    // Maps (0, 0.5) → start and (1, 0.5) → end.
                    (
                        GradientKind::Linear,
                        Affine([dx, dy, -dy, dx, x1 + 0.5 * dy, y1 - 0.5 * dx]),
                    )
                };
                PaintKind::Gradient(Arc::new(Gradient {
                    kind,
                    stops: stops.into(),
                    transform,
                }))
            }
            "pattern" => {
                let p = fill.get("pattern")?;
                let uid = p.pointer("/meta/ux/uid").and_then(Value::as_str)?;
                let asset = self.image(uid)?;
                PaintKind::Image {
                    asset,
                    mode: match p.pointer("/meta/ux/scaleBehavior").and_then(Value::as_str) {
                        Some("fit") => ImageScaleMode::Fit,
                        _ => ImageScaleMode::Fill,
                    },
                    transform: None,
                }
            }
            _ => return None,
        };
        Some(Paint {
            kind,
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: fill.get("enabled").and_then(Value::as_bool) != Some(false),
        })
    }

    fn stroke(&mut self, s: Option<&Value>, size: (f32, f32)) -> Option<Stroke> {
        let s = s?;
        let width = num(s, "width").unwrap_or(1.0);
        if width <= 0.0 {
            return None;
        }
        let paint = self.paint(s, size)?;
        let dashes: Vec<f32> = s
            .get("dash")
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
            align: match text(s, "align") {
                Some("inside") => StrokeAlign::Inside,
                Some("outside") => StrokeAlign::Outside,
                _ => StrokeAlign::Center,
            },
            miter_limit: num(s, "miterLimit").unwrap_or(4.0),
            dashes: dashes.into(),
            ..Default::default()
        })
    }

    fn image(&mut self, uid: &str) -> Option<AssetId> {
        if let Some(&a) = self.images.get(uid) {
            return Some(a);
        }
        let bytes = match self.zip.read(&format!("resources/{uid}"), MAX_ENTRY) {
            Ok(Some(b)) => b,
            _ => {
                self.warnings.missing_images += 1;
                return None;
            }
        };
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?;
        let (bytes, codec) = match reader.format()? {
            image::ImageFormat::Png => (bytes, Codec::Png),
            image::ImageFormat::Jpeg => (bytes, Codec::Jpeg),
            image::ImageFormat::WebP => (bytes, Codec::WebP),
            image::ImageFormat::Gif => {
                let img = image::load_from_memory(&bytes).ok()?;
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .ok()?;
                (png, Codec::Png)
            }
            _ => return None,
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
        self.images.insert(uid.to_owned(), id);
        Some(id)
    }
}

/// Vector node for `path` (in object space): the box starts at the path bounds.
fn vector(path: ude_core::PathData, rule: FillRule) -> (Node, Point) {
    let b = path.bounds();
    let origin = Point::new(b.x0, b.y0);
    let mut moved = PathBuilder::default();
    let mut pts = path.points().iter();
    for verb in path.verbs() {
        let mut next = || pts.next().copied().unwrap_or(origin);
        match verb {
            ude_core::PathVerb::MoveTo => {
                let p = next();
                moved.move_to(p.x - origin.x, p.y - origin.y);
            }
            ude_core::PathVerb::LineTo => {
                let p = next();
                moved.line_to(p.x - origin.x, p.y - origin.y);
            }
            ude_core::PathVerb::QuadTo => {
                let (c, p) = (next(), next());
                moved.quad_to(
                    c.x - origin.x,
                    c.y - origin.y,
                    p.x - origin.x,
                    p.y - origin.y,
                );
            }
            ude_core::PathVerb::CubicTo => {
                let (c1, c2, p) = (next(), next(), next());
                moved.cubic_to(
                    c1.x - origin.x,
                    c1.y - origin.y,
                    c2.x - origin.x,
                    c2.y - origin.y,
                    p.x - origin.x,
                    p.y - origin.y,
                );
            }
            ude_core::PathVerb::Close => {
                moved.close();
            }
        }
    }
    let node = Node::new(
        NodeKind::Vector(Arc::new(VectorData {
            paths: Box::new([VectorPath {
                path: moved.build(),
                fill_rule: rule,
            }]),
        })),
        Symbol::EMPTY,
    )
    .with_size(b.width(), b.height());
    (node, origin)
}

fn parse_path(d: &str) -> Option<ude_core::PathData> {
    use svgtypes::SimplePathSegment as S;
    let mut p = PathBuilder::default();
    let mut any = false;
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        match seg.ok()? {
            S::MoveTo { x, y } => {
                p.move_to(x as f32, y as f32);
                any = true;
            }
            S::LineTo { x, y } => {
                p.line_to(x as f32, y as f32);
            }
            S::Quadratic { x1, y1, x, y } => {
                p.quad_to(x1 as f32, y1 as f32, x as f32, y as f32);
            }
            S::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                p.cubic_to(
                    x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32,
                );
            }
            S::ClosePath => {
                p.close();
            }
        }
    }
    any.then(|| p.build())
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

/// XD colour: `{ "mode": "RGB", "value": { "r": 0–255, … }, "alpha": 0–1 }`.
fn color(c: &Value) -> Option<Color> {
    let v = c.get("value")?;
    let ch = |k| num(v, k).map(|x| x / 255.0);
    Some(Color {
        r: ch("r")?,
        g: ch("g")?,
        b: ch("b")?,
        a: num(c, "alpha").unwrap_or(1.0),
    })
}

fn effects(style: &Value) -> Vec<Effect> {
    let mut out = Vec::new();
    for f in style
        .get("filters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let visible = f.get("visible").and_then(Value::as_bool) != Some(false);
        let params = f.get("params").unwrap_or(&Value::Null);
        match text(f, "type") {
            Some("dropShadow") => {
                for s in params
                    .get("dropShadows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(c) = s.get("color").and_then(color) {
                        out.push(Effect::DropShadow(Shadow {
                            color: c,
                            offset: Point::new(
                                num(s, "dx").unwrap_or(0.0),
                                num(s, "dy").unwrap_or(0.0),
                            ),
                            blur: num(s, "r").unwrap_or(0.0),
                            spread: 0.0,
                            blend: BlendMode::Normal,
                            visible,
                        }));
                    }
                }
            }
            Some("uxdesign#blur") => {
                let radius = num(params, "blurAmount").unwrap_or(0.0);
                out.push(
                    if params.get("backgroundEffect").and_then(Value::as_bool) == Some(true) {
                        Effect::BackgroundBlur { radius, visible }
                    } else {
                        Effect::LayerBlur { radius, visible }
                    },
                );
            }
            _ => {}
        }
    }
    out
}

fn blend(b: &str) -> BlendMode {
    match b {
        "pass-through" => BlendMode::PassThrough,
        "darken" => BlendMode::Darken,
        "multiply" => BlendMode::Multiply,
        "color-burn" => BlendMode::ColorBurn,
        "lighten" => BlendMode::Lighten,
        "screen" => BlendMode::Screen,
        "color-dodge" => BlendMode::ColorDodge,
        "overlay" => BlendMode::Overlay,
        "soft-light" => BlendMode::SoftLight,
        "hard-light" => BlendMode::HardLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

#[cfg(test)]
mod tests;
