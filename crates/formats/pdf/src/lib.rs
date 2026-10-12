//! PDF and Adobe Illustrator import. Illustrator saves `.ai` files with an embedded PDF
//! ("Create PDF Compatible File", the default), which this importer reads; PostScript-only `.ai`
//! files are refused with a hint.
//!
//! Each page becomes a page with one artboard frame (the crop box). Page content streams are
//! interpreted: paths with fill/stroke colours (gray, RGB, CMYK), opacity from graphics states,
//! transforms, form XObjects, JPEG and 8-bit images (with soft masks), and text (one node per
//! show operation, decoded through ToUnicode maps). Clipping, patterns and shadings are reported.

mod text;

use std::collections::HashMap;
use std::sync::Arc;

use lopdf::{Dictionary, Object, ObjectId};
use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::PathBuilder;
use ude_core::scene::{NodeFlags, VectorData, VectorPath};
use ude_core::styles::{Color, Paint, Paints, Stroke};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextAlignV, TextCase, TextData, TextDecoration, TextRun,
    TextSizing, TextStyle, parse_font_style,
};
use ude_core::{Affine, Document, FillRule, Node, NodeId, NodeKind, Point, SourceFormat, Symbol};

const MAX_FORM_DEPTH: usize = 16;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;
/// Ascent used to place text from its baseline (fraction of the font size).
const ASCENT: f32 = 0.8;

pub struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
}

/// Imports a PDF. `illustrator` names pages "Artboard N" and gives AI-specific errors.
pub fn import(bytes: &[u8], name: &str, illustrator: bool) -> Result<Imported, String> {
    let head = &bytes[..bytes.len().min(1024)];
    if !head.windows(5).any(|w| w == b"%PDF-") {
        return Err(if illustrator && head.starts_with(b"%!PS") {
            "this Illustrator file has no PDF data; re-save it in Illustrator with \
             \u{201c}Create PDF Compatible File\u{201d} enabled"
                .into()
        } else {
            "not a PDF file".into()
        });
    }
    let pdf = lopdf::Document::load_mem(bytes).map_err(|e| format!("cannot read PDF: {e}"))?;
    if pdf.is_encrypted() {
        return Err("encrypted PDF files are not supported".into());
    }
    let mut b = Builder {
        pdf: &pdf,
        doc: Document::new(),
        warnings: Warnings::default(),
        fonts: HashMap::new(),
    };
    b.doc.meta.title = name.to_owned();
    b.doc.meta.source = Some(if illustrator {
        SourceFormat::Illustrator
    } else {
        SourceFormat::Pdf
    });
    let root = b.doc.root();
    for (i, (_, page_id)) in pdf.get_pages().into_iter().enumerate() {
        let label = if illustrator { "Artboard" } else { "Page" };
        let page_name = b.doc.strings.intern(&format!("{label} {}", i + 1));
        let page = b
            .doc
            .build_child(root, Node::new(NodeKind::Page, page_name))
            .map_err(|e| e.to_string())?;
        let page_dict = pdf.get_dictionary(page_id).map_err(|e| e.to_string())?;
        let [x0, y0, x1, y1] = b
            .page_box(page_id, b"CropBox")
            .or_else(|| b.page_box(page_id, b"MediaBox"))
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
        let mut frame = Node::new(NodeKind::Frame, page_name)
            .with_size(w, h)
            .with_fills(Paints::new(vec![Paint::solid(Color::WHITE)]));
        frame.flags.set(NodeFlags::CLIPS_CONTENT, true);
        let frame = b.doc.build_child(page, frame).map_err(|e| e.to_string())?;
        // PDF space is y-up from the box corner; frame space is y-down from its top-left.
        let flip = Affine([1.0, 0.0, 0.0, -1.0, -x0.min(x1), y0.max(y1)]);
        let content = pdf.get_page_content(page_id).map_err(|e| e.to_string())?;
        let resources = b.resources(page_dict);
        let mut st = State::new(flip);
        b.run(&content, &resources, frame, &mut st, 0)?;
    }
    if b.doc.children(root).is_empty() {
        return Err("the PDF has no pages".into());
    }
    let warnings = b.warnings.finish();
    let mut doc = b.doc;
    let _ = doc.take_changes();
    Ok(Imported { doc, warnings })
}

#[derive(Default)]
struct Warnings {
    clips: usize,
    patterns: usize,
    shadings: usize,
    images: usize,
    type3: usize,
}

impl Warnings {
    fn finish(self) -> Vec<String> {
        let mut w = Vec::new();
        let mut add = |n: usize, msg: &str| {
            if n > 0 {
                w.push(format!("{n} {msg}"));
            }
        };
        add(
            self.clips,
            "clipping path(s) ignored (content may extend past clip groups)",
        );
        add(
            self.patterns,
            "pattern or gradient fill(s) drawn as a flat colour",
        );
        add(self.shadings, "shading(s) (smooth gradients) skipped");
        add(self.images, "image(s) in unsupported encodings skipped");
        add(self.type3, "Type 3 font text run(s) skipped");
        w
    }
}

#[derive(Clone)]
struct Gs {
    ctm: Affine,
    fill: Color,
    stroke: Color,
    fill_alpha: f32,
    stroke_alpha: f32,
    line_width: f32,
    fill_pattern: bool,
    // Text state.
    font: Option<Vec<u8>>,
    size: f32,
    char_spacing: f32,
    leading: f32,
}

struct State {
    gs: Gs,
    stack: Vec<Gs>,
    path: PathBuilder,
    has_path: bool,
    /// Current point of the path being built (page space).
    cur: Point,
    text_matrix: Affine,
    line_matrix: Affine,
}

impl State {
    fn new(ctm: Affine) -> Self {
        Self {
            gs: Gs {
                ctm,
                fill: Color::BLACK,
                stroke: Color::BLACK,
                fill_alpha: 1.0,
                stroke_alpha: 1.0,
                line_width: 1.0,
                fill_pattern: false,
                font: None,
                size: 12.0,
                char_spacing: 0.0,
                leading: 0.0,
            },
            stack: Vec::new(),
            path: PathBuilder::default(),
            has_path: false,
            cur: Point::new(0.0, 0.0),
            text_matrix: Affine::IDENTITY,
            line_matrix: Affine::IDENTITY,
        }
    }
    fn point(&self, x: f32, y: f32) -> Point {
        self.gs.ctm.apply(Point::new(x, y))
    }
}

struct Builder<'p> {
    pdf: &'p lopdf::Document,
    doc: Document,
    warnings: Warnings,
    fonts: HashMap<ObjectId, text::Font>,
}

fn f(ops: &[Object], i: usize) -> f32 {
    ops.get(i).and_then(|o| o.as_float().ok()).unwrap_or(0.0)
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

fn cmyk(c: f32, m: f32, y: f32, k: f32) -> Color {
    Color {
        r: (1.0 - c) * (1.0 - k),
        g: (1.0 - m) * (1.0 - k),
        b: (1.0 - y) * (1.0 - k),
        a: 1.0,
    }
}

/// Colour from 1 (gray), 3 (RGB) or 4 (CMYK) numeric operands.
fn color_of(ops: &[Object]) -> Option<Color> {
    let n: Vec<f32> = ops.iter().filter_map(|o| o.as_float().ok()).collect();
    match n.as_slice() {
        [g] => Some(Color {
            r: *g,
            g: *g,
            b: *g,
            a: 1.0,
        }),
        [r, g, b] => Some(Color {
            r: *r,
            g: *g,
            b: *b,
            a: 1.0,
        }),
        [c, m, y, k] => Some(cmyk(*c, *m, *y, *k)),
        _ => None,
    }
}

impl<'p> Builder<'p> {
    fn page_box(&self, page: ObjectId, key: &[u8]) -> Option<[f32; 4]> {
        // Boxes are inheritable from parent page-tree nodes.
        let mut dict = self.pdf.get_dictionary(page).ok()?;
        for _ in 0..32 {
            if let Ok(o) = dict.get(key) {
                let a = self.deref(o).as_array().ok()?;
                let v: Vec<f32> = a
                    .iter()
                    .filter_map(|o| self.deref(o).as_float().ok())
                    .collect();
                return (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]]);
            }
            dict = self.deref(dict.get(b"Parent").ok()?).as_dict().ok()?;
        }
        None
    }

    fn deref<'a>(&self, o: &'a Object) -> &'a Object
    where
        'p: 'a,
    {
        self.pdf.dereference(o).map(|(_, o)| o).unwrap_or(o)
    }

    /// The page's resources (inherited through the page tree).
    fn resources(&self, page: &Dictionary) -> Dictionary {
        let mut dict = page;
        for _ in 0..32 {
            if let Ok(r) = dict.get(b"Resources")
                && let Ok(d) = self.deref(r).as_dict()
            {
                return d.clone();
            }
            match dict
                .get(b"Parent")
                .ok()
                .and_then(|p| self.deref(p).as_dict().ok())
            {
                Some(p) => dict = p,
                None => break,
            }
        }
        Dictionary::new()
    }

    fn resource<'a>(
        &self,
        res: &'a Dictionary,
        kind: &[u8],
        name: &[u8],
    ) -> Option<(Option<ObjectId>, &'a Object)>
    where
        'p: 'a,
    {
        let group = self.deref(res.get(kind).ok()?).as_dict().ok()?;
        let o = group.get(name).ok()?;
        let id = o.as_reference().ok();
        Some((id, self.deref(o)))
    }

    fn run(
        &mut self,
        content: &[u8],
        res: &Dictionary,
        parent: NodeId,
        st: &mut State,
        depth: usize,
    ) -> Result<(), String> {
        let ops =
            lopdf::content::Content::decode(content).map_err(|e| format!("page content: {e}"))?;
        for op in &ops.operations {
            let o = &op.operands;
            match op.operator.as_str() {
                "q" => st.stack.push(st.gs.clone()),
                "Q" => {
                    if let Some(g) = st.stack.pop() {
                        st.gs = g;
                    }
                }
                "cm" => {
                    let m = Affine([f(o, 0), f(o, 1), f(o, 2), f(o, 3), f(o, 4), f(o, 5)]);
                    st.gs.ctm = mul(st.gs.ctm, m);
                }
                "w" => st.gs.line_width = f(o, 0),
                "gs" => {
                    if let Some(name) = o.first().and_then(|n| n.as_name().ok())
                        && let Some((_, g)) = self.resource(res, b"ExtGState", name)
                        && let Ok(g) = g.as_dict()
                    {
                        if let Ok(a) = g.get(b"ca").and_then(Object::as_float) {
                            st.gs.fill_alpha = a;
                        }
                        if let Ok(a) = g.get(b"CA").and_then(Object::as_float) {
                            st.gs.stroke_alpha = a;
                        }
                        if let Ok(lw) = g.get(b"LW").and_then(Object::as_float) {
                            st.gs.line_width = lw;
                        }
                    }
                }
                // Colours.
                "g" | "rg" | "k" => {
                    if let Some(c) = color_of(o) {
                        st.gs.fill = c;
                        st.gs.fill_pattern = false;
                    }
                }
                "G" | "RG" | "K" => {
                    if let Some(c) = color_of(o) {
                        st.gs.stroke = c;
                    }
                }
                "sc" | "scn" => match color_of(o) {
                    Some(c) => {
                        st.gs.fill = c;
                        st.gs.fill_pattern = false;
                    }
                    None => st.gs.fill_pattern = true,
                },
                "SC" | "SCN" => {
                    if let Some(c) = color_of(o) {
                        st.gs.stroke = c;
                    }
                }
                "cs" => {
                    st.gs.fill = Color::BLACK;
                    st.gs.fill_pattern =
                        o.first().and_then(|n| n.as_name().ok()) == Some(b"Pattern");
                }
                "CS" => st.gs.stroke = Color::BLACK,
                // Path construction (in user space; stored in page space).
                "m" => {
                    let p = st.point(f(o, 0), f(o, 1));
                    st.path.move_to(p.x, p.y);
                    st.cur = p;
                    st.has_path = true;
                }
                "l" => {
                    let p = st.point(f(o, 0), f(o, 1));
                    st.path.line_to(p.x, p.y);
                    st.cur = p;
                }
                "c" => {
                    let (a, b, c) = (
                        st.point(f(o, 0), f(o, 1)),
                        st.point(f(o, 2), f(o, 3)),
                        st.point(f(o, 4), f(o, 5)),
                    );
                    st.path.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y);
                    st.cur = c;
                }
                "v" | "y" => {
                    // v: first control point = current point; y: second = end point.
                    let (c, e) = (st.point(f(o, 0), f(o, 1)), st.point(f(o, 2), f(o, 3)));
                    let cur = st.cur;
                    st.cur = e;
                    if op.operator == "v" {
                        st.path.cubic_to(cur.x, cur.y, c.x, c.y, e.x, e.y);
                    } else {
                        st.path.cubic_to(c.x, c.y, e.x, e.y, e.x, e.y);
                    }
                }
                "h" => {
                    st.path.close();
                }
                "re" => {
                    let (x, y, w, h) = (f(o, 0), f(o, 1), f(o, 2), f(o, 3));
                    let ps = [
                        st.point(x, y),
                        st.point(x + w, y),
                        st.point(x + w, y + h),
                        st.point(x, y + h),
                    ];
                    st.path.move_to(ps[0].x, ps[0].y);
                    for p in &ps[1..] {
                        st.path.line_to(p.x, p.y);
                    }
                    st.path.close();
                    st.has_path = true;
                }
                // Painting.
                op_name @ ("f" | "F" | "f*" | "S" | "s" | "B" | "B*" | "b" | "b*" | "n") => {
                    let close = matches!(op_name, "s" | "b" | "b*");
                    if close {
                        st.path.close();
                    }
                    let path = std::mem::take(&mut st.path).build();
                    let had = std::mem::replace(&mut st.has_path, false);
                    if !had || op_name == "n" {
                        continue;
                    }
                    let fill = matches!(op_name, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                    let stroke = matches!(op_name, "S" | "s" | "B" | "B*" | "b" | "b*");
                    let rule = if op_name.ends_with('*') {
                        FillRule::EvenOdd
                    } else {
                        FillRule::NonZero
                    };
                    self.paint_path(path, rule, fill, stroke, st, parent)?;
                }
                "W" | "W*" => self.warnings.clips += 1,
                "sh" => self.warnings.shadings += 1,
                "Do" => {
                    let Some(name) = o.first().and_then(|n| n.as_name().ok()) else {
                        continue;
                    };
                    let Some((_, obj)) = self.resource(res, b"XObject", name) else {
                        continue;
                    };
                    let Ok(stream) = obj.as_stream() else {
                        continue;
                    };
                    match stream.dict.get(b"Subtype").and_then(Object::as_name).ok() {
                        Some(b"Form") if depth < MAX_FORM_DEPTH => {
                            let saved = st.gs.clone();
                            if let Ok(m) = stream.dict.get(b"Matrix").and_then(Object::as_array) {
                                let v: Vec<f32> =
                                    m.iter().filter_map(|x| x.as_float().ok()).collect();
                                if v.len() == 6 {
                                    st.gs.ctm = mul(
                                        st.gs.ctm,
                                        Affine([v[0], v[1], v[2], v[3], v[4], v[5]]),
                                    );
                                }
                            }
                            let inner = stream
                                .dict
                                .get(b"Resources")
                                .ok()
                                .and_then(|r| self.deref(r).as_dict().ok())
                                .cloned()
                                .unwrap_or_else(|| res.clone());
                            let data = stream
                                .decompressed_content()
                                .unwrap_or_else(|_| stream.content.clone());
                            self.run(&data, &inner, parent, st, depth + 1)?;
                            st.gs = saved;
                        }
                        Some(b"Image") => self.image(stream, st, parent)?,
                        _ => {}
                    }
                }
                // Text.
                "BT" => {
                    st.text_matrix = Affine::IDENTITY;
                    st.line_matrix = Affine::IDENTITY;
                }
                "Tf" => {
                    st.gs.font = o.first().and_then(|n| n.as_name().ok()).map(<[u8]>::to_vec);
                    st.gs.size = f(o, 1);
                }
                "Tc" => st.gs.char_spacing = f(o, 0),
                "TL" => st.gs.leading = f(o, 0),
                "Td" | "TD" => {
                    if op.operator == "TD" {
                        st.gs.leading = -f(o, 1);
                    }
                    st.line_matrix = mul(st.line_matrix, Affine::translate(f(o, 0), f(o, 1)));
                    st.text_matrix = st.line_matrix;
                }
                "Tm" => {
                    st.line_matrix = Affine([f(o, 0), f(o, 1), f(o, 2), f(o, 3), f(o, 4), f(o, 5)]);
                    st.text_matrix = st.line_matrix;
                }
                "T*" => {
                    st.line_matrix = mul(st.line_matrix, Affine::translate(0.0, -st.gs.leading));
                    st.text_matrix = st.line_matrix;
                }
                "Tj" | "'" | "\"" | "TJ" => {
                    if op.operator != "Tj" && op.operator != "TJ" {
                        st.line_matrix =
                            mul(st.line_matrix, Affine::translate(0.0, -st.gs.leading));
                        st.text_matrix = st.line_matrix;
                    }
                    let strings: Vec<&[u8]> = match op.operator.as_str() {
                        "TJ" => o
                            .first()
                            .and_then(|a| a.as_array().ok())
                            .map(|a| a.iter().filter_map(|x| x.as_str().ok()).collect())
                            .unwrap_or_default(),
                        _ => o.last().and_then(|s| s.as_str().ok()).into_iter().collect(),
                    };
                    self.show_text(&strings, res, st, parent)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn paint_path(
        &mut self,
        path: ude_core::PathData,
        rule: FillRule,
        fill: bool,
        stroke: bool,
        st: &State,
        parent: NodeId,
    ) -> Result<(), String> {
        let b = path.bounds();
        if !(b.x0.is_finite() && b.y0.is_finite()) {
            return Ok(());
        }
        let origin = Point::new(b.x0, b.y0);
        let local = translate_path(&path, -origin.x, -origin.y);
        let mut node = Node::new(
            NodeKind::Vector(Arc::new(VectorData {
                paths: Box::new([VectorPath {
                    path: local,
                    fill_rule: rule,
                }]),
            })),
            Symbol::EMPTY,
        )
        .with_transform(Affine::translate(origin.x, origin.y))
        .with_size(b.width(), b.height());
        if fill {
            if st.gs.fill_pattern {
                self.warnings.patterns += 1;
            }
            let mut c = st.gs.fill;
            c.a *= st.gs.fill_alpha;
            node.fills = Paints::new(vec![Paint::solid(c)]);
        }
        if stroke {
            let mut c = st.gs.stroke;
            c.a *= st.gs.stroke_alpha;
            let scale = st.gs.ctm.determinant().abs().sqrt();
            node.ext_mut().strokes = Some(Box::new(Stroke {
                paints: Paints::new(vec![Paint::solid(c)]),
                width: (st.gs.line_width * scale).max(0.1),
                miter_limit: 10.0,
                ..Default::default()
            }));
        }
        let name = self
            .doc
            .strings
            .intern(if fill { "Path" } else { "Stroke" });
        node.name = name;
        self.doc
            .build_child(parent, node)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn image(&mut self, stream: &lopdf::Stream, st: &State, parent: NodeId) -> Result<(), String> {
        let d = &stream.dict;
        let w = d.get(b"Width").and_then(Object::as_i64).unwrap_or(0).max(0) as u32;
        let h = d
            .get(b"Height")
            .and_then(Object::as_i64)
            .unwrap_or(0)
            .max(0) as u32;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            self.warnings.images += 1;
            return Ok(());
        }
        let filter = match d.get(b"Filter") {
            Ok(Object::Name(n)) => Some(n.clone()),
            Ok(Object::Array(a)) => a.last().and_then(|x| x.as_name().ok()).map(<[u8]>::to_vec),
            _ => None,
        };
        let (bytes, codec) = if filter.as_deref() == Some(b"DCTDecode") && !d.has(b"SMask") {
            (stream.content.clone(), Codec::Jpeg)
        } else {
            let Some(rgba) = self.decode_image(stream, w, h) else {
                self.warnings.images += 1;
                return Ok(());
            };
            let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("image size mismatch")?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            (png, Codec::Png)
        };
        let asset = self.doc.assets.insert(AssetEntry {
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
        // Image space: unit square, first row at the top (y = 1).
        let to_unit = Affine([1.0 / w as f32, 0.0, 0.0, -1.0 / h as f32, 0.0, 1.0]);
        let node = Node::new(NodeKind::Raster { asset }, self.doc.strings.intern("Image"))
            .with_transform(mul(st.gs.ctm, to_unit))
            .with_size(w as f32, h as f32);
        self.doc
            .build_child(parent, node)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 8-bit gray/RGB/CMYK (Flate or uncompressed, or JPEG with a soft mask) → RGBA.
    fn decode_image(&self, stream: &lopdf::Stream, w: u32, h: u32) -> Option<Vec<u8>> {
        let d = &stream.dict;
        let n = (w * h) as usize;
        let jpeg = matches!(d.get(b"Filter"), Ok(Object::Name(f)) if f == b"DCTDecode");
        let mut rgba = if jpeg {
            image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg)
                .ok()?
                .into_rgba8()
                .into_raw()
        } else {
            if d.get(b"BitsPerComponent")
                .and_then(Object::as_i64)
                .unwrap_or(8)
                != 8
            {
                return None;
            }
            let data = stream
                .decompressed_content()
                .ok()
                .or_else(|| (!d.has(b"Filter")).then(|| stream.content.clone()))?;
            let cs = d.get(b"ColorSpace").ok().map(|c| self.deref(c));
            let comps = match cs {
                Some(Object::Name(n)) if n == b"DeviceGray" => 1,
                Some(Object::Name(n)) if n == b"DeviceCMYK" => 4,
                Some(Object::Name(n)) if n == b"DeviceRGB" => 3,
                Some(Object::Array(a))
                    if a.first().and_then(|x| x.as_name().ok()) == Some(b"ICCBased") =>
                {
                    let s = a
                        .get(1)
                        .map(|x| self.deref(x))
                        .and_then(|x| x.as_stream().ok())?;
                    s.dict.get(b"N").and_then(Object::as_i64).unwrap_or(3) as usize
                }
                _ => return None,
            };
            if data.len() < n * comps {
                return None;
            }
            let mut out = Vec::with_capacity(n * 4);
            for px in data.chunks_exact(comps).take(n) {
                let c = match comps {
                    1 => [px[0], px[0], px[0]],
                    4 => {
                        let c = cmyk(
                            f32::from(px[0]) / 255.0,
                            f32::from(px[1]) / 255.0,
                            f32::from(px[2]) / 255.0,
                            f32::from(px[3]) / 255.0,
                        );
                        [
                            (c.r * 255.0) as u8,
                            (c.g * 255.0) as u8,
                            (c.b * 255.0) as u8,
                        ]
                    }
                    _ => [px[0], px[1], px[2]],
                };
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            out
        };
        if let Ok(sm) = d.get(b"SMask")
            && let Ok(mask) = self.deref(sm).as_stream()
            && let Ok(alpha) = mask.decompressed_content()
            && alpha.len() >= n
        {
            for (px, a) in rgba.chunks_exact_mut(4).zip(alpha) {
                px[3] = a;
            }
        }
        Some(rgba)
    }

    fn show_text(
        &mut self,
        strings: &[&[u8]],
        res: &Dictionary,
        st: &mut State,
        parent: NodeId,
    ) -> Result<(), String> {
        let Some(font_name) = st.gs.font.clone() else {
            return Ok(());
        };
        let Some((id, obj)) = self.resource(res, b"Font", &font_name) else {
            return Ok(());
        };
        let Ok(dict) = obj.as_dict() else {
            return Ok(());
        };
        let key = id.unwrap_or((0, 0));
        if !self.fonts.contains_key(&key) {
            let font = text::Font::load(self.pdf, dict);
            self.fonts.insert(key, font);
        }
        let font = &self.fonts[&key];
        if font.type3 {
            self.warnings.type3 += 1;
            return Ok(());
        }
        let content: String = strings.iter().map(|s| font.decode(s)).collect();
        if content.trim().is_empty() {
            return Ok(());
        }
        let (family, face) = (font.family.clone(), font.face.clone());
        // Placement: text space → page via the text matrix and CTM; baseline at the origin.
        let m = mul(st.gs.ctm, st.text_matrix);
        let scale = (m.0[2].powi(2) + m.0[3].powi(2)).sqrt();
        let size = (st.gs.size * scale).max(0.5);
        let origin = m.apply(Point::new(0.0, 0.0));
        let rotation = (-m.0[1]).atan2(m.0[0]);
        let (weight, italic) = parse_font_style(&face);
        let mut fill = st.gs.fill;
        fill.a *= st.gs.fill_alpha;
        let style = TextStyle {
            family: self.doc.strings.intern(&family),
            weight,
            italic,
            size,
            line_height: LineHeight::Auto,
            letter_spacing: if st.gs.char_spacing != 0.0 {
                LetterSpacing::Px(st.gs.char_spacing * scale)
            } else {
                LetterSpacing::Normal
            },
            decoration: TextDecoration::None,
            case: TextCase::Original,
            fills: Paints::new(vec![Paint::solid(fill)]),
        };
        let len = content.len() as u32;
        let t = TextData {
            content: content.into(),
            runs: Box::new([TextRun {
                range: 0..len,
                style: 0,
            }]),
            styles: vec![style].into(),
            sizing: TextSizing::AutoWidth,
            align_h: TextAlignH::Left,
            align_v: TextAlignV::Top,
            paragraph_spacing: 0.0,
        };
        let (s, c) = rotation.sin_cos();
        // Box top-left is one ascent above the baseline, along the text's own up direction.
        // The local y-down axis maps to (s, c); "up" is its opposite.
        let top = Point::new(origin.x - s * size * ASCENT, origin.y - c * size * ASCENT);
        let node = Node::new(NodeKind::Text(Box::new(t)), self.doc.strings.intern("Text"))
            .with_transform(Affine([c, -s, s, c, top.x, top.y]));
        self.doc
            .build_child(parent, node)
            .map_err(|e| e.to_string())?;
        // Advance roughly so consecutive shows on a line do not overlap exactly.
        let advance = strings.iter().map(|s| s.len()).sum::<usize>() as f32 * st.gs.size * 0.5;
        st.text_matrix = mul(st.text_matrix, Affine::translate(advance, 0.0));
        Ok(())
    }
}

fn translate_path(p: &ude_core::PathData, dx: f32, dy: f32) -> ude_core::PathData {
    let mut b = PathBuilder::default();
    let mut pts = p.points().iter().map(|q| Point::new(q.x + dx, q.y + dy));
    for v in p.verbs() {
        let mut next = || pts.next().unwrap_or(Point::new(0.0, 0.0));
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

#[cfg(test)]
mod tests;
