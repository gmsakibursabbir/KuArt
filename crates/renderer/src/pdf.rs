//! Vector PDF export through the [`Painter`] interface, so the PDF shows exactly what the
//! canvas draws: paths, fills and strokes, clipping, opacity and blend modes, images (with
//! alpha) and text as glyph outlines (exact without the fonts installed; not selectable yet).
//! Gradients are written as their middle colour and blurred shadows are omitted (reported).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;

use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use vello::kurbo::{Affine, BezPath, PathEl, Rect};
use vello::peniko::{BlendMode, Color, Fill, FontData, Mix};

use crate::images::DecodedImage;
use crate::painter::{BrushSpec, Geom, GlyphPos, Painter};

/// One PDF page being recorded.
pub struct PdfPainter {
    content: String,
    /// Graphics states by (alpha, blend) → name.
    states: Vec<(u16, &'static str)>,
    images: Vec<ImageObject>,
    image_ids: HashMap<usize, usize>,
    /// Fallbacks used, for the export report.
    pub approximations: Vec<&'static str>,
}

struct ImageObject {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

fn n(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

fn mix_name(b: BlendMode) -> &'static str {
    match b.mix {
        Mix::Multiply => "Multiply",
        Mix::Screen => "Screen",
        Mix::Overlay => "Overlay",
        Mix::Darken => "Darken",
        Mix::Lighten => "Lighten",
        Mix::ColorDodge => "ColorDodge",
        Mix::ColorBurn => "ColorBurn",
        Mix::HardLight => "HardLight",
        Mix::SoftLight => "SoftLight",
        Mix::Difference => "Difference",
        Mix::Exclusion => "Exclusion",
        Mix::Hue => "Hue",
        Mix::Saturation => "Saturation",
        Mix::Color => "Color",
        Mix::Luminosity => "Luminosity",
        _ => "Normal",
    }
}

impl PdfPainter {
    fn new() -> Self {
        Self {
            content: String::new(),
            states: Vec::new(),
            images: Vec::new(),
            image_ids: HashMap::new(),
            approximations: Vec::new(),
        }
    }

    fn note(&mut self, what: &'static str) {
        if !self.approximations.contains(&what) {
            self.approximations.push(what);
        }
    }

    /// Name of a graphics state with this opacity and blend mode.
    fn state(&mut self, alpha: f32, blend: &'static str) -> String {
        let key = ((alpha.clamp(0.0, 1.0) * 1000.0).round() as u16, blend);
        let i = match self.states.iter().position(|s| *s == key) {
            Some(i) => i,
            None => {
                self.states.push(key);
                self.states.len() - 1
            }
        };
        format!("/G{i} gs\n")
    }

    fn cm(&mut self, t: Affine) {
        let c = t.as_coeffs();
        let _ = writeln!(
            self.content,
            "{} {} {} {} {} {} cm",
            n(c[0]),
            n(c[1]),
            n(c[2]),
            n(c[3]),
            n(c[4]),
            n(c[5])
        );
    }

    fn path(&mut self, p: &BezPath) {
        let mut last = (0.0, 0.0);
        for el in p.elements() {
            let s = &mut self.content;
            let _ = match *el {
                PathEl::MoveTo(a) => {
                    last = (a.x, a.y);
                    writeln!(s, "{} {} m", n(a.x), n(a.y))
                }
                PathEl::LineTo(a) => {
                    last = (a.x, a.y);
                    writeln!(s, "{} {} l", n(a.x), n(a.y))
                }
                PathEl::QuadTo(c, a) => {
                    // Quadratic → cubic.
                    let c1 = (
                        last.0 + 2.0 / 3.0 * (c.x - last.0),
                        last.1 + 2.0 / 3.0 * (c.y - last.1),
                    );
                    let c2 = (a.x + 2.0 / 3.0 * (c.x - a.x), a.y + 2.0 / 3.0 * (c.y - a.y));
                    last = (a.x, a.y);
                    writeln!(
                        s,
                        "{} {} {} {} {} {} c",
                        n(c1.0),
                        n(c1.1),
                        n(c2.0),
                        n(c2.1),
                        n(a.x),
                        n(a.y)
                    )
                }
                PathEl::CurveTo(c1, c2, a) => {
                    last = (a.x, a.y);
                    writeln!(
                        s,
                        "{} {} {} {} {} {} c",
                        n(c1.x),
                        n(c1.y),
                        n(c2.x),
                        n(c2.y),
                        n(a.x),
                        n(a.y)
                    )
                }
                PathEl::ClosePath => writeln!(s, "h"),
            };
        }
    }

    /// Sets the colour (and alpha state) for `brush`; `stroke` selects the stroke colour.
    fn color(&mut self, brush: &BrushSpec, stroke: bool) {
        let c: Color = match brush {
            BrushSpec::Solid(c) => *c,
            BrushSpec::Gradient(g) => {
                self.note("gradients are written as a solid colour");
                g.stops
                    .get(g.stops.len() / 2)
                    .map_or(Color::BLACK, |s| s.color.to_alpha_color())
            }
        };
        let [r, g, b, a] = c.components;
        let op = if stroke { "RG" } else { "rg" };
        let _ = writeln!(
            self.content,
            "{} {} {} {op}",
            n(f64::from(r)),
            n(f64::from(g)),
            n(f64::from(b))
        );
        if a < 1.0 {
            let gs = self.state(a, "Normal");
            self.content.push_str(&gs);
        }
    }

    /// Builds the PDF file from recorded pages (`(width, height, painter)`).
    fn write(pages: Vec<(f64, f64, PdfPainter)>) -> Vec<u8> {
        let mut objects: Vec<Vec<u8>> = Vec::new();
        // 1 = catalog, 2 = pages; the rest are allocated as we go.
        objects.push(Vec::new());
        objects.push(Vec::new());
        let mut kids = Vec::new();
        for (w, h, p) in pages {
            let mut image_refs = String::new();
            for (i, img) in p.images.iter().enumerate() {
                let smask = img.alpha.as_ref().map(|a| {
                    objects.push(stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8", img.width, img.height), a));
                    objects.len()
                });
                let extra = smask.map_or(String::new(), |id| format!(" /SMask {id} 0 R"));
                objects.push(stream(
                    &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8{extra}", img.width, img.height),
                    &img.rgb,
                ));
                let _ = write!(image_refs, "/Im{i} {} 0 R ", objects.len());
            }
            let mut states = String::new();
            for (i, (alpha, blend)) in p.states.iter().enumerate() {
                let a = f64::from(*alpha) / 1000.0;
                let _ = write!(
                    states,
                    "/G{i} << /ca {} /CA {} /BM /{blend} >> ",
                    n(a),
                    n(a)
                );
            }
            objects.push(stream("", p.content.as_bytes()));
            let content_id = objects.len();
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {content_id} 0 R /Resources << /ExtGState << {states}>> /XObject << {image_refs}>> >> >>",
                    n(w),
                    n(h)
                )
                .into_bytes(),
            );
            kids.push(objects.len());
        }
        objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
        objects[1] = format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.iter()
                .map(|k| format!("{k} 0 R"))
                .collect::<Vec<_>>()
                .join(" "),
            kids.len()
        )
        .into_bytes();

        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R /Info << /Producer (KuArt) >> >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
        );
        out
    }
}

/// A Flate-compressed stream object.
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = z.write_all(data);
    let packed = z.finish().unwrap_or_default();
    let mut out = format!(
        "<< {dict} /Filter /FlateDecode /Length {} >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    out.extend_from_slice(&packed);
    out.extend_from_slice(b"\nendstream");
    out
}

struct Pen<'a> {
    path: &'a mut BezPath,
    x: f64,
    y: f64,
}

impl OutlinePen for Pen<'_> {
    // Font outlines are y-up; the page content is y-down.
    fn move_to(&mut self, x: f32, y: f32) {
        self.path
            .move_to((self.x + f64::from(x), self.y - f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path
            .line_to((self.x + f64::from(x), self.y - f64::from(y)));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.path.quad_to(
            (self.x + f64::from(cx), self.y - f64::from(cy)),
            (self.x + f64::from(x), self.y - f64::from(y)),
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (self.x + f64::from(cx0), self.y - f64::from(cy0)),
            (self.x + f64::from(cx1), self.y - f64::from(cy1)),
            (self.x + f64::from(x), self.y - f64::from(y)),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

impl Painter for PdfPainter {
    fn fill(
        &mut self,
        transform: Affine,
        geom: &Geom<'_>,
        rule: Fill,
        brush: &BrushSpec,
        _brush_transform: Option<Affine>,
    ) {
        self.content.push_str("q\n");
        self.color(brush, false);
        self.cm(transform);
        self.path(&geom.to_path());
        self.content.push_str(if rule == Fill::EvenOdd {
            "f*\nQ\n"
        } else {
            "f\nQ\n"
        });
    }

    fn stroke(&mut self, transform: Affine, geom: &Geom<'_>, width: f64, brush: &BrushSpec) {
        self.content.push_str("q\n");
        self.color(brush, true);
        self.cm(transform);
        let _ = writeln!(self.content, "{} w", n(width));
        self.path(&geom.to_path());
        self.content.push_str("S\nQ\n");
    }

    fn shadow(
        &mut self,
        _transform: Affine,
        _rect: Rect,
        _color: Color,
        _radius: f64,
        _std_dev: f64,
    ) {
        self.note("blurred shadows are omitted");
    }

    fn push_layer(&mut self, transform: Affine, clip: &Geom<'_>, blend: BlendMode, alpha: f32) {
        self.content.push_str("q\n");
        let gs = self.state(alpha, mix_name(blend));
        self.content.push_str(&gs);
        // Clip in the layer's space, then return to page space for the children.
        self.cm(transform);
        self.path(&clip.to_path());
        self.content.push_str("W n\n");
        self.cm(transform.inverse());
    }

    fn pop_layer(&mut self) {
        self.content.push_str("Q\n");
    }

    fn image(&mut self, transform: Affine, image: &DecodedImage, alpha: f32) {
        let key = std::sync::Arc::as_ptr(&image.cpu) as usize;
        let index = match self.image_ids.get(&key) {
            Some(&i) => i,
            None => {
                let px = image.cpu.data_as_u8_slice();
                let mut rgb = Vec::with_capacity(px.len() / 4 * 3);
                let mut a = Vec::with_capacity(px.len() / 4);
                for p in px.chunks_exact(4) {
                    // Premultiplied → straight.
                    let al = u16::from(p[3]);
                    let un = |c: u8| {
                        (u16::from(c) * 255 + al / 2)
                            .checked_div(al)
                            .map_or(0, |v| v.min(255) as u8)
                    };
                    rgb.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2])]);
                    a.push(p[3]);
                }
                let opaque = a.iter().all(|&v| v == 255);
                self.images.push(ImageObject {
                    width: image.width,
                    height: image.height,
                    rgb,
                    alpha: (!opaque).then_some(a),
                });
                self.image_ids.insert(key, self.images.len() - 1);
                self.images.len() - 1
            }
        };
        self.content.push_str("q\n");
        if alpha < 1.0 {
            let gs = self.state(alpha, "Normal");
            self.content.push_str(&gs);
        }
        // Image space is the unit square with the first row at the top.
        let (w, h) = (f64::from(image.width), f64::from(image.height));
        self.cm(transform * Affine::new([w, 0.0, 0.0, -h, 0.0, h]));
        let _ = writeln!(self.content, "/Im{index} Do\nQ");
    }

    fn glyphs(
        &mut self,
        transform: Affine,
        font: &FontData,
        size: f32,
        brush: &BrushSpec,
        glyphs: &[GlyphPos],
    ) {
        let Ok(f) = skrifa::FontRef::from_index(font.data.as_ref(), font.index) else {
            return;
        };
        let outlines = f.outline_glyphs();
        let mut path = BezPath::new();
        for g in glyphs {
            if let Some(o) = outlines.get(skrifa::GlyphId::new(g.id)) {
                let mut pen = Pen {
                    path: &mut path,
                    x: f64::from(g.x),
                    y: f64::from(g.y),
                };
                let _ = o.draw(
                    DrawSettings::unhinted(Size::new(size), LocationRef::default()),
                    &mut pen,
                );
            }
        }
        if !path.elements().is_empty() {
            self.fill(transform, &Geom::Path(&path), Fill::NonZero, brush, None);
        }
    }
}

/// Exports each node (page or layer) as one PDF page. Returns the file and any approximations.
pub fn export_pdf(
    doc: &ude_core::Document,
    ids: &[ude_core::NodeId],
    caches: &mut crate::RenderCaches,
) -> Result<(Vec<u8>, Vec<&'static str>), String> {
    let mut pages = Vec::new();
    let mut notes: Vec<&'static str> = Vec::new();
    for &id in ids {
        let bounds = crate::export_bounds(doc, id).ok_or("nothing to export (empty layer)")?;
        let mut p = PdfPainter::new();
        // PDF is y-up; flip once so all drawing uses canvas (y-down) coordinates.
        p.cm(Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, bounds.height()]));
        let view = Affine::translate((-bounds.x0, -bounds.y0));
        crate::paint_for_export(
            doc,
            id,
            view,
            Rect::new(0.0, 0.0, bounds.width(), bounds.height()),
            &mut p,
            caches,
        );
        for a in &p.approximations {
            if !notes.contains(a) {
                notes.push(a);
            }
        }
        pages.push((bounds.width(), bounds.height(), p));
    }
    Ok((PdfPainter::write(pages), notes))
}
