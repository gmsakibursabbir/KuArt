//! Photoshop import (`docs/format-strategy.md` §4): `.psd` and `.psb`, 8/16-bit RGB and
//! grayscale. The layer tree (groups, artboards) is rebuilt without flattening. Layer pixels
//! stay compressed as in the file ([`Codec::PsdChannels`]) and are decoded on demand by the
//! renderer with [`decode_rgba`]; raster layer masks are applied at decode time.
//!
//! Text, smart-object and shape layers display Photoshop's stored pixels. Adjustment layers,
//! layer effects and vector masks are kept as preserved blocks and reported.

mod channels;

use ude_core::assets::{AssetEntry, AssetId, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::scene::{NodeFlags, Preserved};
use ude_core::styles::BlendMode;
use ude_core::{Affine, Document, Node, NodeKind, SourceFormat};

pub use channels::decode_rgba;

/// Refuse canvases or layers above this many pixels (untrusted-input guard).
pub const MAX_PIXELS: u64 = 300_000_000;
const MAX_SIDE: u32 = 300_000;
const MAX_LAYERS: usize = 20_000;

pub struct Imported {
    pub doc: Document,
    /// What was not imported exactly, one line per kind of issue.
    pub warnings: Vec<String>,
}

/// Imports a PSD or PSB file. `name` becomes the document title.
pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != b"8BPS" {
        return Err("not a Photoshop file".into());
    }
    let psb = match r.u16()? {
        1 => false,
        2 => true,
        v => return Err(format!("unsupported PSD version {v}")),
    };
    r.skip(6)?;
    let channel_count = r.u16()?;
    let (height, width) = (r.u32()?, r.u32()?);
    let depth = r.u16()?;
    let mode = r.u16()?;
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(format!("image is too large ({width}x{height})"));
    }
    let gray = match mode {
        1 => true,
        3 => false,
        2 => return Err("indexed-colour PSD files are not supported yet".into()),
        4 => {
            return Err("CMYK PSD files are not supported yet; convert to RGB in Photoshop".into());
        }
        9 => return Err("Lab PSD files are not supported yet".into()),
        m => return Err(format!("unsupported PSD colour mode {m}")),
    };
    if !matches!(depth, 8 | 16) {
        return Err(format!(
            "{depth}-bit PSD files are not supported yet (8 and 16-bit are)"
        ));
    }
    let skip_section = |r: &mut Reader| -> Result<(), String> {
        let len = r.u32()?;
        r.skip(len as usize)
    };
    skip_section(&mut r)?; // colour mode data
    skip_section(&mut r)?; // image resources

    let fmt = Fmt { psb, depth, gray };
    let lm_len = r.len(psb)?;
    let lm_end = r
        .pos
        .checked_add(lm_len)
        .filter(|&e| e <= bytes.len())
        .ok_or("truncated layer section")?;
    let mut layers = Vec::new();
    let mut notes = Vec::new();
    if lm_len > 0 {
        let li_len = r.len(psb)?;
        let li_start = r.pos;
        if li_len > 0 {
            layers = parse_layer_info(&bytes[li_start..(li_start + li_len).min(lm_end)], fmt)?;
        }
        r.pos = li_start + li_len;
        if r.pos + 4 <= lm_end {
            // Global tagged blocks: 16/32-bit layers (`Lr16`…) and notes (`Anno`).
            let gm = r.u32()? as usize;
            r.skip(gm)?;
            while r.pos + 12 <= lm_end {
                let sig = r.take(4)?;
                if sig != b"8BIM" && sig != b"8B64" {
                    break;
                }
                let key: [u8; 4] = r.take(4)?.try_into().map_err(|_| "bad key")?;
                let len = if psb && LONG_KEYS.contains(&&key) {
                    r.u64()? as usize
                } else {
                    r.u32()? as usize
                };
                let data = r.take(len.min(lm_end - r.pos))?;
                if layers.is_empty() && matches!(&key, b"Lr16" | b"Lr32" | b"Layr") {
                    layers = parse_layer_info(data, fmt)?;
                } else if &key == b"Anno" {
                    notes = annotations(data);
                }
                r.pos += (4 - len % 4) % 4;
            }
        }
    }
    r.pos = lm_end;

    let mut b = Builder {
        doc: Document::new(),
        warnings: Warnings::default(),
        fmt,
    };
    b.doc.meta.title = name.to_owned();
    b.doc.meta.source = Some(SourceFormat::Psd);
    let root = b.doc.root();
    let page_name = b.doc.strings.intern("Page 1");
    let page = b
        .doc
        .build_child(root, Node::new(NodeKind::Page, page_name))
        .map_err(|e| e.to_string())?;
    let canvas_name = b.doc.strings.intern(name);
    let mut canvas = Node::new(NodeKind::Frame, canvas_name).with_size(width as f32, height as f32);
    canvas.flags.set(NodeFlags::CLIPS_CONTENT, true);
    let canvas = b.doc.build_child(page, canvas).map_err(|e| e.to_string())?;

    if layers.is_empty() {
        // Flat file: the merged image is the only layer.
        let rgba = channels::decode_merged(&bytes[r.pos..], width, height, channel_count, fmt)?;
        let asset = b.png_asset(width, height, rgba)?;
        let layer_name = b.doc.strings.intern("Background");
        b.doc
            .build_child(
                canvas,
                Node::new(NodeKind::Raster { asset }, layer_name)
                    .with_size(width as f32, height as f32),
            )
            .map_err(|e| e.to_string())?;
    } else {
        let tree = build_tree(layers)?;
        b.children(canvas, &tree, (0, 0))?;
    }
    b.doc.comments = notes;
    let warnings = b.warnings.finish();
    let mut doc = b.doc;
    let _ = doc.take_changes();
    Ok(Imported { doc, warnings })
}

/// Tagged blocks whose length is 8 bytes in PSB files.
const LONG_KEYS: [&[u8; 4]; 13] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
    b"FEid", b"FXid", b"PxSD",
];

#[derive(Clone, Copy)]
pub(crate) struct Fmt {
    pub psb: bool,
    pub depth: u16,
    pub gray: bool,
}

pub(crate) struct Reader<'a> {
    pub b: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or("unexpected end of file")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    pub fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok((u64::from(self.u32()?) << 32) | u64::from(self.u32()?))
    }
    /// Section length: 4 bytes in PSD, 8 in PSB.
    pub fn len(&mut self, psb: bool) -> Result<usize, String> {
        let v = if psb {
            self.u64()?
        } else {
            u64::from(self.u32()?)
        };
        usize::try_from(v).map_err(|_| "length out of range".into())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct IRect {
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}

impl IRect {
    fn read(r: &mut Reader) -> Result<Self, String> {
        Ok(Self {
            top: r.i32()?,
            left: r.i32()?,
            bottom: r.i32()?,
            right: r.i32()?,
        })
    }
    pub fn width(self) -> u32 {
        (i64::from(self.right) - i64::from(self.left)).clamp(0, i64::from(u32::MAX)) as u32
    }
    pub fn height(self) -> u32 {
        (i64::from(self.bottom) - i64::from(self.top)).clamp(0, i64::from(u32::MAX)) as u32
    }
    fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    Layer,
    OpenFolder,
    ClosedFolder,
    Divider,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Special {
    #[default]
    None,
    Text,
    SmartObject,
    Adjustment,
    Shape,
}

pub(crate) struct ChannelRef<'a> {
    pub id: i16,
    pub compression: u16,
    pub data: &'a [u8],
}

pub(crate) struct LayerMaskInfo {
    pub rect: IRect,
    pub default_color: u8,
    pub disabled: bool,
}

struct Layer<'a> {
    rect: IRect,
    name: String,
    blend: BlendMode,
    opacity: u8,
    fill_opacity: u8,
    clipping: bool,
    hidden: bool,
    section: Section,
    section_blend: Option<BlendMode>,
    artboard: bool,
    special: Special,
    effects: bool,
    vector_mask: bool,
    mask: Option<LayerMaskInfo>,
    channels: Vec<ChannelRef<'a>>,
    /// Tagged blocks kept verbatim (effects, text engine data, adjustments, …).
    preserved: Vec<([u8; 4], &'a [u8])>,
}

fn parse_layer_info(data: &[u8], fmt: Fmt) -> Result<Vec<Layer<'_>>, String> {
    let mut r = Reader::new(data);
    let count = r.i16()?.unsigned_abs() as usize;
    if count > MAX_LAYERS {
        return Err(format!("too many layers ({count})"));
    }
    let mut layers = Vec::with_capacity(count);
    let mut lens: Vec<Vec<(i16, usize)>> = Vec::with_capacity(count);
    for _ in 0..count {
        let rect = IRect::read(&mut r)?;
        let n = r.u16()? as usize;
        if n > 56 {
            return Err("corrupt layer record (too many channels)".into());
        }
        let mut chans = Vec::with_capacity(n);
        for _ in 0..n {
            let id = r.i16()?;
            chans.push((id, r.len(fmt.psb)?));
        }
        if r.take(4)? != b"8BIM" {
            return Err("corrupt layer record (blend signature)".into());
        }
        let blend = blend_mode(r.take(4)?);
        let opacity = r.u8()?;
        let clipping = r.u8()? != 0;
        let flags = r.u8()?;
        r.skip(1)?;
        let extra_len = r.u32()? as usize;
        let extra_end = r
            .pos
            .checked_add(extra_len)
            .filter(|&e| e <= data.len())
            .ok_or("truncated layer record")?;

        let mask_len = r.u32()? as usize;
        let mask_start = r.pos;
        let mask = if mask_len >= 18 {
            let rect = IRect::read(&mut r)?;
            let default_color = r.u8()?;
            let mflags = r.u8()?;
            Some(LayerMaskInfo {
                rect,
                default_color,
                disabled: mflags & 0x02 != 0,
            })
        } else {
            None
        };
        r.pos = mask_start + mask_len;
        let ranges = r.u32()? as usize;
        r.skip(ranges)?;
        let name_len = r.u8()? as usize;
        let mut name = String::from_utf8_lossy(r.take(name_len)?).into_owned();
        r.skip((4 - (name_len + 1) % 4) % 4)?;

        let mut layer = Layer {
            rect,
            name: String::new(),
            blend,
            opacity,
            fill_opacity: 255,
            clipping,
            hidden: flags & 0x02 != 0,
            section: Section::Layer,
            section_blend: None,
            artboard: false,
            special: Special::None,
            effects: false,
            vector_mask: false,
            mask,
            channels: Vec::new(),
            preserved: Vec::new(),
        };
        while r.pos + 12 <= extra_end {
            let sig = r.take(4)?;
            if sig != b"8BIM" && sig != b"8B64" {
                // Some writers pad blocks; resynchronise on the next signature.
                r.pos -= 3;
                continue;
            }
            let key: [u8; 4] = r.take(4)?.try_into().map_err(|_| "bad key")?;
            let len = if fmt.psb && LONG_KEYS.contains(&&key) {
                r.u64()? as usize
            } else {
                r.u32()? as usize
            };
            let block = r.take(len.min(extra_end.saturating_sub(r.pos)))?;
            let mut b = Reader::new(block);
            match &key {
                b"luni" => {
                    let n = b.u32()? as usize;
                    let units: Vec<u16> = (0..n.min(block.len() / 2))
                        .filter_map(|_| b.u16().ok())
                        .collect();
                    name = String::from_utf16_lossy(&units)
                        .trim_end_matches('\0')
                        .to_owned();
                }
                b"lsct" | b"lsdk" => {
                    layer.section = match b.u32()? {
                        1 => Section::OpenFolder,
                        2 => Section::ClosedFolder,
                        3 => Section::Divider,
                        _ => Section::Layer,
                    };
                    if block.len() >= 12 {
                        b.skip(4)?;
                        layer.section_blend = Some(blend_mode(b.take(4)?));
                    }
                }
                b"iOpa" => layer.fill_opacity = b.u8()?,
                b"artb" | b"artd" | b"abdd" => layer.artboard = true,
                b"TySh" | b"tySh" => layer.special = Special::Text,
                b"SoLd" | b"PlLd" | b"SoLE" => layer.special = Special::SmartObject,
                b"vogk" | b"vscg" | b"SoCo" | b"GdFl" | b"PtFl"
                    if layer.special == Special::None =>
                {
                    layer.special = Special::Shape
                }
                b"levl" | b"curv" | b"hue2" | b"hue " | b"brit" | b"blnc" | b"selc" | b"mixr"
                | b"grdm" | b"phfl" | b"expA" | b"vibA" | b"nvrt" | b"post" | b"thrs" | b"CgEd"
                | b"clrL" | b"blwh" => layer.special = Special::Adjustment,
                b"lfx2" | b"lrFX" | b"lfxs" | b"lmfx" => layer.effects = true,
                b"vmsk" | b"vsms" => layer.vector_mask = true,
                _ => {}
            }
            if !matches!(
                &key,
                b"luni"
                    | b"lsct"
                    | b"lsdk"
                    | b"iOpa"
                    | b"lyid"
                    | b"clbl"
                    | b"infx"
                    | b"knko"
                    | b"lspf"
                    | b"lclr"
                    | b"fxrp"
            ) {
                layer.preserved.push((key, block));
            }
        }
        r.pos = extra_end;
        layer.name = name;
        // Fill layers (solid colour, gradient, pattern) are shapes for display purposes.
        layers.push(layer);
        lens.push(chans);
    }
    // Channel image data follows all records, in the same order.
    for (layer, chans) in layers.iter_mut().zip(&lens) {
        for &(id, len) in chans {
            let block = r.take(len)?;
            if block.len() < 2 {
                continue;
            }
            let compression = u16::from_be_bytes([block[0], block[1]]);
            layer.channels.push(ChannelRef {
                id,
                compression,
                data: &block[2..],
            });
        }
    }
    Ok(layers)
}

fn blend_mode(key: &[u8]) -> BlendMode {
    match key {
        b"pass" => BlendMode::PassThrough,
        b"dark" => BlendMode::Darken,
        b"mul " => BlendMode::Multiply,
        b"idiv" => BlendMode::ColorBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"lite" => BlendMode::Lighten,
        b"scrn" => BlendMode::Screen,
        b"div " => BlendMode::ColorDodge,
        b"lddg" => BlendMode::LinearDodge,
        b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Color,
        b"lum " => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// Photoshop notes (`Anno` block, Photoshop 6+): text annotations with author and position.
/// Sound annotations are skipped.
fn annotations(data: &[u8]) -> Vec<ude_core::comments::Comment> {
    fn pascal(r: &mut Reader) -> Option<String> {
        let n = usize::from(r.u8().ok()?);
        let s = String::from_utf8_lossy(r.take(n).ok()?).into_owned();
        // Padded to an even length including the length byte.
        if n % 2 == 0 {
            r.skip(1).ok()?;
        }
        Some(s)
    }
    let mut out = Vec::new();
    let mut r = Reader::new(data);
    let Ok(_) = r.skip(4) else { return out }; // major and minor version
    let Ok(count) = r.u32() else { return out };
    for _ in 0..count.min(10_000) {
        let start = r.pos;
        let Ok(len) = r.u32() else { break };
        let end = start + len as usize;
        if len < 8 || end > data.len() {
            break;
        }
        let parsed = (|| -> Option<ude_core::comments::Comment> {
            let kind = r.take(4).ok()?;
            r.skip(4).ok()?; // open, flags, optional blocks
            let icon = IRect::read(&mut r).ok()?;
            r.skip(16 + 10).ok()?; // popup rect, colour
            let author = pascal(&mut r)?;
            let _name = pascal(&mut r)?;
            let _date = pascal(&mut r)?;
            if kind != b"txtA" {
                return None;
            }
            r.skip(4).ok()?; // data length
            if r.take(4).ok()? != b"txtC" {
                return None;
            }
            let body = data.get(r.pos..end)?;
            let text = if let Some(utf16) = body.strip_prefix(&[0xFE, 0xFF]) {
                let units: Vec<u16> = utf16
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                String::from_utf8_lossy(body).into_owned()
            };
            let text = text
                .trim_end_matches(['\0', '\r', '\n'])
                .replace('\r', "\n");
            Some(ude_core::comments::Comment {
                x: icon.left as f32,
                y: icon.top as f32,
                author,
                text,
                source: "Photoshop note".into(),
                ..Default::default()
            })
        })();
        out.extend(parsed.filter(|c| !c.text.trim().is_empty()));
        r.pos = end;
    }
    out
}

enum Item<'a> {
    Layer(Layer<'a>),
    Group(Layer<'a>, Vec<Item<'a>>),
}

/// Records are bottom-to-top: a divider opens a group, the folder record closes it.
fn build_tree(layers: Vec<Layer<'_>>) -> Result<Vec<Item<'_>>, String> {
    let mut stack: Vec<Vec<Item>> = vec![Vec::new()];
    for l in layers {
        match l.section {
            Section::Divider => stack.push(Vec::new()),
            Section::OpenFolder | Section::ClosedFolder => {
                let children = if stack.len() > 1 {
                    stack.pop().unwrap_or_default()
                } else {
                    Vec::new()
                };
                stack
                    .last_mut()
                    .ok_or("bad group nesting")?
                    .push(Item::Group(l, children));
            }
            Section::Layer => stack
                .last_mut()
                .ok_or("bad group nesting")?
                .push(Item::Layer(l)),
        }
    }
    // Unclosed groups (corrupt files): keep their layers at the outer level.
    while stack.len() > 1 {
        let inner = stack.pop().unwrap_or_default();
        stack.last_mut().ok_or("bad group nesting")?.extend(inner);
    }
    Ok(stack.pop().unwrap_or_default())
}

#[derive(Default)]
struct Warnings {
    text: usize,
    smart: usize,
    shape: usize,
    adjustment: usize,
    effects: usize,
    vector_masks: usize,
    blend: usize,
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
            self.text,
            "text layer(s) shown as Photoshop's pixels; editable PSD text arrives in phase 2",
        );
        add(
            self.smart,
            "smart object(s) shown as placed pixels; the embedded file is preserved",
        );
        add(
            self.shape,
            "shape/fill layer(s) shown as pixels; editable vectors arrive in phase 2",
        );
        add(
            self.adjustment,
            "adjustment layer(s) cannot be displayed yet; preserved for export",
        );
        add(
            self.effects,
            "layer(s) with effects (shadows, strokes, …): effects are preserved, not drawn",
        );
        add(self.vector_masks, "vector mask(s) preserved, not applied");
        add(
            self.blend,
            "layer(s) use blend modes without an exact match (shown approximately)",
        );
        w
    }
}

struct Builder {
    doc: Document,
    warnings: Warnings,
    fmt: Fmt,
}

impl Builder {
    /// Adds `items` (bottom to top) under `parent`, whose top-left is at `origin` in canvas space.
    fn children(
        &mut self,
        parent: ude_core::NodeId,
        items: &[Item],
        origin: (i32, i32),
    ) -> Result<(), String> {
        let mut base: Option<&Layer> = None;
        for item in items {
            match item {
                Item::Layer(l) => {
                    if !l.clipping {
                        base = Some(l);
                    }
                    let clip_base = if l.clipping { base } else { None };
                    if let Some(node) = self.layer(l, clip_base, origin)? {
                        self.doc
                            .build_child(parent, node)
                            .map_err(|e| e.to_string())?;
                    }
                }
                Item::Group(l, kids) => {
                    base = None;
                    let bounds = union(kids).unwrap_or(IRect {
                        top: origin.1,
                        left: origin.0,
                        bottom: origin.1,
                        right: origin.0,
                    });
                    let kind = if l.artboard {
                        NodeKind::Frame
                    } else {
                        NodeKind::Group
                    };
                    let mut node = Node::new(kind, self.doc.strings.intern(&l.name))
                        .with_transform(Affine::translate(
                            (bounds.left - origin.0) as f32,
                            (bounds.top - origin.1) as f32,
                        ))
                        .with_size(bounds.width() as f32, bounds.height() as f32);
                    node.blend = l.section_blend.unwrap_or(l.blend);
                    node.opacity = f32::from(l.opacity) / 255.0;
                    node.flags.set(NodeFlags::VISIBLE, !l.hidden);
                    if l.artboard {
                        node.flags.set(NodeFlags::CLIPS_CONTENT, true);
                    }
                    self.preserve(&mut node, l);
                    let id = self
                        .doc
                        .build_child(parent, node)
                        .map_err(|e| e.to_string())?;
                    self.children(id, kids, (bounds.left, bounds.top))?;
                }
            }
        }
        Ok(())
    }

    fn layer(
        &mut self,
        l: &Layer,
        clip_base: Option<&Layer>,
        origin: (i32, i32),
    ) -> Result<Option<Node>, String> {
        match l.special {
            Special::Text => self.warnings.text += 1,
            Special::SmartObject => self.warnings.smart += 1,
            Special::Shape => self.warnings.shape += 1,
            Special::Adjustment => self.warnings.adjustment += 1,
            Special::None => {}
        }
        self.warnings.effects += usize::from(l.effects);
        self.warnings.vector_masks += usize::from(l.vector_mask);
        if matches!(l.blend, BlendMode::LinearBurn | BlendMode::HardLight) {
            self.warnings.blend += 1;
        }
        if l.rect.is_empty() {
            return Ok(None);
        }
        let (w, h) = (l.rect.width(), l.rect.height());
        if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(format!(
                "layer \u{201c}{}\u{201d} is too large ({w}x{h})",
                l.name
            ));
        }
        let asset = match clip_base {
            // Clipped layers are baked: their alpha is multiplied by the base layer's alpha.
            Some(base) => {
                let mut rgba = channels::decode_rgba(&channels::encode_blob(
                    l.rect,
                    &l.channels,
                    l.mask.as_ref(),
                    self.fmt,
                ))?
                .2;
                if !base.rect.is_empty() {
                    let base_rgba = channels::decode_rgba(&channels::encode_blob(
                        base.rect,
                        &base.channels,
                        base.mask.as_ref(),
                        self.fmt,
                    ))?
                    .2;
                    clip_alpha(&mut rgba, l.rect, &base_rgba, base.rect);
                } else {
                    rgba.iter_mut().skip(3).step_by(4).for_each(|a| *a = 0);
                }
                self.png_asset(w, h, rgba)?
            }
            None => {
                let blob = channels::encode_blob(l.rect, &l.channels, l.mask.as_ref(), self.fmt);
                self.doc.assets.insert(AssetEntry {
                    hash: ContentHash(*blake3::hash(&blob).as_bytes()),
                    kind: AssetKind::Image {
                        width: w,
                        height: h,
                    },
                    source: AssetSource::Memory {
                        bytes: blob.into(),
                        codec: Codec::PsdChannels,
                    },
                })
            }
        };
        let mut node = Node::new(NodeKind::Raster { asset }, self.doc.strings.intern(&l.name))
            .with_transform(Affine::translate(
                (l.rect.left - origin.0) as f32,
                (l.rect.top - origin.1) as f32,
            ))
            .with_size(w as f32, h as f32);
        node.blend = if l.blend == BlendMode::PassThrough {
            BlendMode::Normal
        } else {
            l.blend
        };
        // Without effects, fill opacity scales the pixels exactly like opacity.
        node.opacity = f32::from(l.opacity) / 255.0 * f32::from(l.fill_opacity) / 255.0;
        let base_hidden = clip_base.is_some_and(|b| b.hidden);
        node.flags.set(
            NodeFlags::VISIBLE,
            !l.hidden && !base_hidden && l.special != Special::Adjustment,
        );
        self.preserve(&mut node, l);
        Ok(Some(node))
    }

    fn preserve(&mut self, node: &mut Node, l: &Layer) {
        for (key, data) in &l.preserved {
            let asset = self.blob(data);
            let key = self.doc.strings.intern(&String::from_utf8_lossy(key));
            node.ext_mut().preserved.push(Preserved {
                source: SourceFormat::Psd,
                key,
                data: asset,
            });
        }
    }

    fn blob(&mut self, data: &[u8]) -> AssetId {
        self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(data).as_bytes()),
            kind: AssetKind::Blob,
            source: AssetSource::Memory {
                bytes: data.into(),
                codec: Codec::Raw,
            },
        })
    }

    fn png_asset(&mut self, w: u32, h: u32, rgba: Vec<u8>) -> Result<AssetId, String> {
        let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("pixel buffer size mismatch")?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(&png).as_bytes()),
            kind: AssetKind::Image {
                width: w,
                height: h,
            },
            source: AssetSource::Memory {
                bytes: png.into(),
                codec: Codec::Png,
            },
        }))
    }
}

/// Union of the layer rectangles in `items` (recursively), ignoring empty layers.
fn union(items: &[Item]) -> Option<IRect> {
    let mut out: Option<IRect> = None;
    for item in items {
        let r = match item {
            Item::Layer(l) if !l.rect.is_empty() => Some(l.rect),
            Item::Layer(_) => None,
            Item::Group(_, kids) => union(kids),
        };
        if let Some(r) = r {
            out = Some(match out {
                None => r,
                Some(o) => IRect {
                    top: o.top.min(r.top),
                    left: o.left.min(r.left),
                    bottom: o.bottom.max(r.bottom),
                    right: o.right.max(r.right),
                },
            });
        }
    }
    out
}

/// Multiplies `rgba`'s alpha (covering `rect`) by `base`'s alpha (covering `base_rect`).
fn clip_alpha(rgba: &mut [u8], rect: IRect, base: &[u8], base_rect: IRect) {
    let (w, bw) = (rect.width() as i64, base_rect.width() as i64);
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let x = i as i64 % w + i64::from(rect.left) - i64::from(base_rect.left);
        let y = i as i64 / w + i64::from(rect.top) - i64::from(base_rect.top);
        let a = if x >= 0 && y >= 0 && x < bw && y < i64::from(base_rect.height()) {
            base[((y * bw + x) * 4 + 3) as usize]
        } else {
            0
        };
        px[3] = ((u16::from(px[3]) * u16::from(a) + 127) / 255) as u8;
    }
}

#[cfg(test)]
mod tests;
