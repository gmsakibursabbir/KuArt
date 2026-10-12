//! `design-editor-cli mcp`: a Model Context Protocol server over stdio, so AI agents
//! (Claude Code, Claude Desktop, any MCP client) can open, inspect, edit, render and save
//! KuArt documents. One document is open per session; every edit is undoable.
//!
//! Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout. Logs go to stderr only.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};
use ude_core::styles::{Color, Paint, PaintKind};
use ude_core::typography::{TextRun, TextSizing};
use ude_core::{
    Affine, Document, History, Node, NodeFlags, NodeId, NodeKind, Op, Paints, Prop, Size,
    Transaction,
};
use ude_editor::{Tool, default_node};
use ude_renderer::RenderCaches;

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Longest side of a `render_preview` image, in pixels.
const PREVIEW_MAX_PX: f64 = 1600.0;

struct Session {
    doc: Document,
    history: History,
    path: Option<PathBuf>,
    caches: RenderCaches,
}

type ToolResult = Result<Vec<Value>, String>;

pub fn run() -> Result<(), String> {
    let mut session = Session {
        doc: blank("Untitled", None),
        history: History::default(),
        path: None,
        caches: RenderCaches::default(),
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => session.handle(&msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}").map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

impl Session {
    /// Answers one JSON-RPC message; notifications get no reply.
    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "kuart", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "KuArt design documents. Call get_tree first to see node ids. \
                    Coordinates are pixels relative to the parent. Colors are #RRGGBB or #RRGGBBAA.",
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": tool_list()}),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call(name, &args) {
                    Ok(content) => json!({"content": content, "isError": false}),
                    Err(e) => json!({"content": [text(e)], "isError": true}),
                }
            }
            _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn call(&mut self, name: &str, a: &Value) -> ToolResult {
        match name {
            "open_document" => {
                let path = PathBuf::from(req_str(a, "path")?);
                self.doc = crate::open(&path)?;
                self.history = History::default();
                self.path = ude_format_native::is_native_extension(&crate::extension(&path))
                    .then_some(path);
                Ok(vec![text(self.summary())])
            }
            "new_document" => {
                let title = opt_str(a, "title").unwrap_or("Untitled");
                let artboard = match (opt_f32(a, "width"), opt_f32(a, "height")) {
                    (Some(w), Some(h)) => Some((w, h)),
                    _ => None,
                };
                self.doc = blank(title, artboard);
                self.history = History::default();
                self.path = None;
                Ok(vec![text(self.summary())])
            }
            "get_tree" => {
                let depth = a.get("max_depth").and_then(Value::as_u64).unwrap_or(64) as usize;
                let mut out = self.summary();
                out.push('\n');
                for &page in self.doc.children(self.doc.root()) {
                    self.tree_lines(page, 0, depth, &mut out);
                }
                Ok(vec![text(out)])
            }
            "get_node" => {
                let id = self.node_id(a)?;
                Ok(vec![text(self.describe(id))])
            }
            "create_node" => self.create(a),
            "update_node" => {
                let id = self.node_id(a)?;
                self.update(id, a)?;
                Ok(vec![text(self.describe(id))])
            }
            "delete_node" => {
                let id = self.node_id(a)?;
                if matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Page)) {
                    return Err("pages cannot be deleted".into());
                }
                let mut tx = Transaction::new("Delete");
                tx.apply(&mut self.doc, Op::Detach { id })
                    .map_err(|e| e.to_string())?;
                self.history.commit(&mut self.doc, tx);
                Ok(vec![text(format!("deleted {}", id.index()))])
            }
            "undo" | "redo" => {
                let done = if name == "undo" {
                    self.history.undo(&mut self.doc)
                } else {
                    self.history.redo(&mut self.doc)
                }
                .map_err(|e| e.to_string())?;
                Ok(vec![text(if done {
                    format!("{name} done")
                } else {
                    format!("nothing to {name}")
                })])
            }
            "save_document" => {
                let path = match opt_str(a, "path") {
                    Some(p) => PathBuf::from(p),
                    None => self
                        .path
                        .clone()
                        .ok_or("no path: pass \"path\" (a .krt file)")?,
                };
                if !ude_format_native::is_native_extension(&crate::extension(&path)) {
                    return Err("save_document writes .krt files; use export for PNG/SVG".into());
                }
                ude_format_native::save_to_path(&self.doc, &path).map_err(|e| e.to_string())?;
                self.path = Some(path.clone());
                Ok(vec![text(format!("saved {}", path.display()))])
            }
            "export" => {
                let path = PathBuf::from(req_str(a, "path")?);
                let id = self.target(a)?;
                let scale = opt_f32(a, "scale").unwrap_or(1.0).clamp(0.01, 16.0);
                let bytes = match crate::extension(&path).as_str() {
                    ext @ ("png" | "jpg" | "jpeg" | "webp") => {
                        let format = ude_renderer::RasterFormat::from_extension(ext)
                            .unwrap_or(ude_renderer::RasterFormat::Png);
                        ude_renderer::export_raster(
                            &self.doc,
                            id,
                            f64::from(scale),
                            format,
                            &mut self.caches,
                        )?
                    }
                    "pdf" => {
                        // Without an id: one PDF page per artboard.
                        let targets = if a.get("id").is_some() {
                            vec![id]
                        } else {
                            crate::pdf_targets(&self.doc)
                        };
                        ude_renderer::export_pdf(&self.doc, &targets, &mut self.caches)?.0
                    }
                    "svg" => self.svg(id)?.into_bytes(),
                    other => {
                        return Err(format!(
                            "export supports .png, .jpg, .webp, .svg and .pdf, not '.{other}'"
                        ));
                    }
                };
                std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(vec![text(format!("wrote {}", path.display()))])
            }
            "export_svg_code" => {
                let id = self.target(a)?;
                Ok(vec![text(self.svg(id)?)])
            }
            "generate_code" => {
                let id = self.target(a)?;
                let name = opt_str(a, "target").unwrap_or("html");
                let target = ude_codegen::Target::parse(name).ok_or_else(|| {
                    format!("unknown target '{name}' (html, css, tailwind, react, vue, tokens)")
                })?;
                let out = ude_codegen::generate(&self.doc, id, target)?;
                let mut content = vec![text(out.code)];
                if !out.notes.is_empty() {
                    content.push(text(format!(
                        "Notes:
- {}",
                        out.notes.join(
                            "
- "
                        )
                    )));
                }
                Ok(content)
            }
            "render_preview" => {
                let id = self.target(a)?;
                let bounds =
                    ude_renderer::export_bounds(&self.doc, id).ok_or("nothing to render")?;
                let scale =
                    (PREVIEW_MAX_PX / bounds.width().max(bounds.height()).max(1.0)).min(2.0);
                let png = ude_renderer::export_png(&self.doc, id, scale, &mut self.caches)?;
                let mut out =
                    vec![json!({"type": "image", "data": base64(&png), "mimeType": "image/png"})];
                let missing = self.caches.text.missing_fonts();
                if !missing.is_empty() {
                    out.push(text(format!(
                        "Missing fonts (drawn with a fallback): {}",
                        missing.join(", ")
                    )));
                }
                Ok(out)
            }
            _ => Err(format!("unknown tool '{name}'")),
        }
    }

    fn summary(&self) -> String {
        let pages = self.doc.children(self.doc.root()).len();
        let nodes = self.doc.descendants(self.doc.root()).count();
        let path = self
            .path
            .as_deref()
            .map_or("not saved".into(), |p| p.display().to_string());
        format!(
            "\"{}\" ({path}): {pages} page(s), {nodes} nodes",
            self.doc.meta.title
        )
    }

    fn tree_lines(&self, id: NodeId, depth: usize, max: usize, out: &mut String) {
        let Some(n) = self.doc.node(id) else { return };
        let t = n.transform.translation();
        out.push_str(&format!(
            "{}[{}] {} \"{}\" x={} y={} {}x{}{}\n",
            "  ".repeat(depth),
            id.index(),
            crate::kind_name(&n.kind),
            self.doc.strings.resolve(n.name),
            t.x,
            t.y,
            n.size.w,
            n.size.h,
            if n.flags.contains(NodeFlags::VISIBLE) {
                ""
            } else {
                " (hidden)"
            },
        ));
        if depth + 1 < max {
            for &c in &n.children {
                self.tree_lines(c, depth + 1, max, out);
            }
        } else if !n.children.is_empty() {
            out.push_str(&format!(
                "{}… {} children\n",
                "  ".repeat(depth + 1),
                n.children.len()
            ));
        }
    }

    fn describe(&self, id: NodeId) -> String {
        let Some(n) = self.doc.node(id) else {
            return format!("node {} not found", id.index());
        };
        let t = n.transform.translation();
        let mut v = json!({
            "id": id.index(),
            "kind": crate::kind_name(&n.kind),
            "name": self.doc.strings.resolve(n.name),
            "parent": n.parent.map(NodeId::index),
            "x": t.x, "y": t.y, "width": n.size.w, "height": n.size.h,
            "transform": n.transform.0,
            "opacity": n.opacity,
            "corner_radius": n.corner_radius,
            "visible": n.flags.contains(NodeFlags::VISIBLE),
            "fills": n.fills.as_slice().iter().map(|p| match &p.kind {
                PaintKind::Solid(c) => json!(c.to_hex()),
                _ => json!("non-solid paint"),
            }).collect::<Vec<_>>(),
            "children": n.children.iter().map(|c| c.index()).collect::<Vec<_>>(),
        });
        if let NodeKind::Text(t) = &n.kind {
            v["text"] = json!(&*t.content);
            if let Some(s) = t.styles.first() {
                v["font_family"] = json!(self.doc.strings.resolve(s.family));
                v["font_size"] = json!(s.size);
                v["font_weight"] = json!(s.weight);
                v["italic"] = json!(s.italic);
            }
        }
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// `id` argument → live node.
    fn node_id(&self, a: &Value) -> Result<NodeId, String> {
        let n = a
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("missing numeric \"id\"")?;
        self.find(n)
    }

    fn find(&self, index: u64) -> Result<NodeId, String> {
        self.doc
            .descendants(self.doc.root())
            .find(|id| u64::from(id.index()) == index)
            .ok_or_else(|| format!("no node with id {index} (call get_tree)"))
    }

    /// Optional `id`, else the first page.
    fn target(&self, a: &Value) -> Result<NodeId, String> {
        match a.get("id").and_then(Value::as_u64) {
            Some(n) => self.find(n),
            None => self
                .doc
                .children(self.doc.root())
                .first()
                .copied()
                .ok_or_else(|| "document has no pages".into()),
        }
    }

    fn svg(&mut self, id: NodeId) -> Result<String, String> {
        let (doc, text) = (&self.doc, &mut self.caches.text);
        ude_format_svg::export_with(doc, id, &mut |t, size| text.lines(t, &doc.strings, size))
    }

    fn create(&mut self, a: &Value) -> ToolResult {
        let tool = match req_str(a, "kind")? {
            "frame" => Tool::Frame,
            "rectangle" => Tool::Rectangle,
            "ellipse" => Tool::Ellipse,
            "polygon" => Tool::Polygon,
            "star" => Tool::Star,
            "line" => Tool::Line,
            "text" => Tool::Text,
            k => {
                return Err(format!(
                    "unknown kind '{k}' (frame, rectangle, ellipse, polygon, star, line, text)"
                ));
            }
        };
        let parent = match a.get("parent").and_then(Value::as_u64) {
            Some(p) => self.find(p)?,
            None => self.target(&json!({}))?,
        };
        let mut node = default_node(&mut self.doc, tool);
        node.transform = Affine::translate(
            opt_f32(a, "x").unwrap_or(0.0),
            opt_f32(a, "y").unwrap_or(0.0),
        );
        if tool != Tool::Text {
            node.size = Size::new(
                opt_f32(a, "width").unwrap_or(100.0),
                opt_f32(a, "height").unwrap_or(100.0),
            );
        }
        let id = self.doc.create(node);
        let index = self.doc.children(parent).len() as u32;
        let mut tx = Transaction::new("Create");
        tx.apply(&mut self.doc, Op::Attach { id, parent, index })
            .map_err(|e| e.to_string())?;
        self.history.commit(&mut self.doc, tx);
        // Remaining fields (name, fill, text, …) go through the same path as update_node.
        self.update(id, a)?;
        Ok(vec![text(self.describe(id))])
    }

    fn update(&mut self, id: NodeId, a: &Value) -> Result<(), String> {
        let n = self.doc.node(id).ok_or("node not found")?.clone();
        let mut ops = Vec::new();
        if let Some(name) = opt_str(a, "name") {
            ops.push(Prop::Name(self.doc.strings.intern(name)));
        }
        let (x, y) = (opt_f32(a, "x"), opt_f32(a, "y"));
        if x.is_some() || y.is_some() {
            let mut m = n.transform.0;
            m[4] = x.unwrap_or(m[4]);
            m[5] = y.unwrap_or(m[5]);
            ops.push(Prop::Transform(Affine(m)));
        }
        let (w, h) = (opt_f32(a, "width"), opt_f32(a, "height"));
        if w.is_some() || h.is_some() {
            ops.push(Prop::Size(Size::new(
                w.unwrap_or(n.size.w).max(0.0),
                h.unwrap_or(n.size.h).max(0.0),
            )));
        }
        if let Some(o) = opt_f32(a, "opacity") {
            ops.push(Prop::Opacity(o.clamp(0.0, 1.0)));
        }
        if let Some(r) = opt_f32(a, "corner_radius") {
            ops.push(Prop::CornerRadius(r.max(0.0)));
        }
        if let Some(v) = a.get("visible").and_then(Value::as_bool) {
            let mut flags = n.flags;
            flags.set(NodeFlags::VISIBLE, v);
            ops.push(Prop::Flags(flags));
        }
        if let Some(fill) = opt_str(a, "fill") {
            let c = parse_color(fill)?;
            if let NodeKind::Text(t) = &n.kind {
                let mut t = t.clone();
                let mut styles = t.styles.to_vec();
                for s in &mut styles {
                    s.fills = Paints::new(vec![Paint::solid(c)]);
                }
                t.styles = styles.into();
                ops.push(Prop::Kind(NodeKind::Text(t)));
            } else {
                ops.push(Prop::Fills(Paints::new(vec![Paint::solid(c)])));
            }
        }
        let text_edit = ["text", "font_family", "font_size", "font_weight"]
            .iter()
            .any(|k| a.get(*k).is_some());
        if text_edit {
            // Start from a pending fill edit, if any, so both apply.
            let current = ops.iter().rev().find_map(|p| match p {
                Prop::Kind(NodeKind::Text(t)) => Some(t.clone()),
                _ => None,
            });
            let Some(mut t) = current.or(match &n.kind {
                NodeKind::Text(t) => Some(t.clone()),
                _ => None,
            }) else {
                return Err("text fields apply to text nodes only".into());
            };
            if let Some(content) = opt_str(a, "text") {
                let len = u32::try_from(content.len()).map_err(|_| "text too long")?;
                t.content = content.into();
                t.runs = Box::new([TextRun {
                    range: 0..len,
                    style: 0,
                }]);
            }
            let mut styles = t.styles.to_vec();
            if let Some(s) = styles.first_mut() {
                if let Some(f) = opt_str(a, "font_family") {
                    s.family = self.doc.strings.intern(f);
                }
                if let Some(v) = opt_f32(a, "font_size") {
                    s.size = v.clamp(1.0, 4096.0);
                }
                if let Some(v) = a.get("font_weight").and_then(Value::as_u64) {
                    s.weight = v.clamp(1, 1000) as u16;
                }
            }
            t.styles = styles.into();
            if w.is_some() && t.sizing == TextSizing::AutoWidth {
                t.sizing = TextSizing::AutoHeight; // an explicit width wraps
            }
            ops.retain(|p| !matches!(p, Prop::Kind(_)));
            ops.push(Prop::Kind(NodeKind::Text(t)));
        }
        if ops.is_empty() {
            return Ok(());
        }
        let mut tx = Transaction::new("Edit");
        for value in ops {
            tx.apply(&mut self.doc, Op::Set { id, value })
                .map_err(|e| e.to_string())?;
        }
        self.history.commit(&mut self.doc, tx);
        self.autosize_text(id);
        Ok(())
    }

    /// Fits an auto-sized text box to its content (as the app does after a text edit).
    fn autosize_text(&mut self, id: NodeId) {
        let Some(n) = self.doc.node(id) else { return };
        let NodeKind::Text(t) = &n.kind else { return };
        let size = match t.sizing {
            TextSizing::AutoWidth => {
                let (w, h) = self
                    .caches
                    .text
                    .measure(t, &self.doc.strings, f32::INFINITY);
                Size::new(w.ceil(), h.ceil())
            }
            TextSizing::AutoHeight => {
                let (_, h) = self.caches.text.measure(t, &self.doc.strings, n.size.w);
                Size::new(n.size.w, h.ceil())
            }
            TextSizing::Fixed | TextSizing::Truncate => return,
        };
        if size != n.size {
            let mut tx = Transaction::new("Fit text");
            if tx
                .apply(
                    &mut self.doc,
                    Op::Set {
                        id,
                        value: Prop::Size(size),
                    },
                )
                .is_ok()
            {
                self.history.commit(&mut self.doc, tx);
            }
        }
    }
}

/// New document with one page and, optionally, a white artboard of `size`.
fn blank(title: &str, artboard: Option<(f32, f32)>) -> Document {
    let mut doc = Document::new();
    doc.meta.title = title.to_owned();
    let root = doc.root();
    let name = doc.strings.intern("Page 1");
    let page = doc
        .build_child(root, Node::new(NodeKind::Page, name))
        .expect("fresh document");
    if let Some((w, h)) = artboard {
        let mut frame = default_node(&mut doc, Tool::Frame).with_size(w, h);
        frame.name = doc.strings.intern("Frame 1");
        doc.build_child(page, frame).expect("fresh page");
    }
    doc.take_changes();
    doc
}

fn text(s: impl Into<String>) -> Value {
    json!({"type": "text", "text": s.into()})
}

fn req_str<'a>(a: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(a, key).ok_or_else(|| format!("missing string \"{key}\""))
}

fn opt_str<'a>(a: &'a Value, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}

fn opt_f32(a: &Value, key: &str) -> Option<f32> {
    a.get(key)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
fn parse_color(s: &str) -> Result<Color, String> {
    let h = s.trim().trim_start_matches('#');
    let bad = || format!("bad color '{s}' (use #RRGGBB or #RRGGBBAA)");
    let byte =
        |i: usize| u8::from_str_radix(h.get(i..i + 2).ok_or_else(bad)?, 16).map_err(|_| bad());
    match h.len() {
        3 => {
            let d = |i: usize| {
                u8::from_str_radix(&h[i..=i], 16)
                    .map(|v| v * 17)
                    .map_err(|_| bad())
            };
            Ok(Color::from_rgba8(d(0)?, d(1)?, d(2)?, 255))
        }
        6 => Ok(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Ok(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => Err(bad()),
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= c.len() {
                T[(n >> shift) as usize & 63] as char
            } else {
                '='
            });
        }
    }
    out
}

fn tool_list() -> Value {
    let id = json!({"type": "integer", "description": "Node id from get_tree"});
    let geometry = json!({
        "x": {"type": "number"}, "y": {"type": "number"},
        "width": {"type": "number"}, "height": {"type": "number"},
        "name": {"type": "string"},
        "fill": {"type": "string", "description": "#RRGGBB or #RRGGBBAA"},
        "opacity": {"type": "number", "minimum": 0, "maximum": 1},
        "corner_radius": {"type": "number"},
        "visible": {"type": "boolean"},
        "text": {"type": "string", "description": "Text nodes only"},
        "font_family": {"type": "string"}, "font_size": {"type": "number"},
        "font_weight": {"type": "integer", "description": "100–900"},
    });
    let mut create = geometry.clone();
    create["kind"] = json!({"type": "string", "enum": ["frame", "rectangle", "ellipse", "polygon", "star", "line", "text"]});
    create["parent"] =
        json!({"type": "integer", "description": "Parent node id (default: first page)"});
    let mut update = geometry;
    update["id"] = id.clone();
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({"name": name, "description": description,
               "inputSchema": {"type": "object", "properties": properties, "required": required}})
    };
    json!([
        tool(
            "generate_code",
            "Turn a node (default: first page) into code: html (page with CSS), css, tailwind, react (JSX + CSS), vue (SFC) or tokens (CSS variables + JSON). Auto layout and evenly stacked layers become flexbox.",
            json!({"id": id, "target": {"type": "string", "enum": ["html", "css", "tailwind", "react", "vue", "tokens"]}}),
            &[],
        ),
        tool(
            "open_document",
            "Open a .krt, Figma (.fig/.jam/.deck/…), .sketch, .xd, .psd, .ai, .pdf, .svg or image file (replaces the open document).",
            json!({"path": {"type": "string"}}),
            &["path"]
        ),
        tool(
            "new_document",
            "Start a new document; with width and height it gets one artboard (frame).",
            json!({"title": {"type": "string"}, "width": {"type": "number"}, "height": {"type": "number"}}),
            &[]
        ),
        tool(
            "get_tree",
            "Layer tree: id, kind, name, position and size of every node.",
            json!({"max_depth": {"type": "integer"}}),
            &[]
        ),
        tool(
            "get_node",
            "All properties of one node as JSON.",
            json!({"id": id}),
            &["id"]
        ),
        tool(
            "create_node",
            "Create a shape, frame or text node.",
            create,
            &["kind"]
        ),
        tool(
            "update_node",
            "Change properties of a node; omitted fields stay unchanged.",
            update,
            &["id"]
        ),
        tool(
            "delete_node",
            "Delete a node and its children.",
            json!({"id": id}),
            &["id"]
        ),
        tool("undo", "Undo the last edit.", json!({}), &[]),
        tool("redo", "Redo the last undone edit.", json!({}), &[]),
        tool(
            "save_document",
            "Save as .krt (default: the path it was opened from).",
            json!({"path": {"type": "string"}}),
            &[]
        ),
        tool(
            "export",
            "Export a node (default: first page) to .png, .jpg, .webp, .svg or .pdf (PDF without id: one page per artboard).",
            json!({"path": {"type": "string"}, "id": id, "scale": {"type": "number"}}),
            &["path"]
        ),
        tool(
            "export_svg_code",
            "SVG markup of a node (default: first page).",
            json!({"id": id}),
            &[]
        ),
        tool(
            "render_preview",
            "PNG image of a node (default: first page) to look at the design.",
            json!({"id": id}),
            &[]
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(s: &mut Session, method: &str, params: Value) -> Value {
        s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("reply")
    }

    #[test]
    fn create_edit_render_round_trip() {
        let mut s = Session {
            doc: blank("t", Some((200.0, 100.0))),
            history: History::default(),
            path: None,
            caches: RenderCaches::default(),
        };
        let init = call(
            &mut s,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(
            call(&mut s, "tools/list", json!({}))["result"]["tools"]
                .as_array()
                .unwrap()
                .len()
                >= 10
        );

        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "create_node", "arguments":
            {"kind": "rectangle", "x": 10, "y": 20, "width": 30, "height": 40, "fill": "#ff0000"}}),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        let node: Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(node["fills"][0], "#FF0000");
        let id = node["id"].as_u64().unwrap();

        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "update_node", "arguments": {"id": id, "x": 50}}),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "undo", "arguments": {}}),
        );
        assert_eq!(r["result"]["isError"], false);
        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "get_node", "arguments": {"id": id}}),
        );
        let node: Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(node["x"], 10.0);

        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "render_preview", "arguments": {}}),
        );
        assert_eq!(r["result"]["content"][0]["type"], "image", "{r}");

        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "update_node", "arguments": {"id": 99999}}),
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(
            s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .is_none()
        );
    }

    #[test]
    fn generates_code() {
        let mut s = Session {
            doc: blank("t", Some((200.0, 100.0))),
            history: History::default(),
            path: None,
            caches: RenderCaches::default(),
        };
        let frame = s.doc.children(s.doc.children(s.doc.root())[0])[0].index();
        let r = call(
            &mut s,
            "tools/call",
            json!({"name": "generate_code", "arguments": {"id": frame, "target": "react"}}),
        );
        let code = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(code.contains("export default function"), "{r}");
    }

    #[test]
    fn helpers() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(parse_color("#0f0").unwrap().to_hex(), "#00FF00");
        assert!(parse_color("nope").is_err());
    }
}
