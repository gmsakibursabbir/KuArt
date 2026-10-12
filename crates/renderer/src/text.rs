//! Text layout with parley (shaping, bidi, wrapping, font fallback) and glyph drawing
//! through the [`Painter`], so both backends draw the same glyphs.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, StyleProperty,
};
use ude_core::typography::{self, LetterSpacing, TextAlignH, TextData, TextDecoration, TextSizing};
use ude_core::{Interner, styles::PaintKind};
use vello::kurbo::Affine;

use crate::convert;
use crate::painter::{BrushSpec, GlyphPos, Painter};

/// Layouts kept between frames; cleared wholesale when exceeded (simple bound, no LRU).
const MAX_CACHED_LAYOUTS: usize = 4096;

pub struct TextEngine {
    fonts: FontContext,
    layouts: LayoutContext<u16>,
    cache: HashMap<u64, Layout<u16>>,
    /// Missing faces as (family, weight, italic), checked once each.
    checked: HashSet<(String, u16, bool)>,
    missing: Vec<(String, u16, bool)>,
}

impl Default for TextEngine {
    /// System fonts plus the user fonts folder ([`ude_core::paths::user_fonts_dir`]).
    fn default() -> Self {
        let mut engine = Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            cache: HashMap::new(),
            checked: HashSet::new(),
            missing: Vec::new(),
        };
        if let Some(dir) = ude_core::paths::user_fonts_dir()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for path in entries.flatten().map(|e| e.path()) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(ext.as_str(), "ttf" | "otf" | "ttc")
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    engine.add_font_data(bytes);
                }
            }
        }
        engine
    }
}

impl TextEngine {
    /// Missing font faces as display names, e.g. "Inter SemiBold Italic". Text using them
    /// is drawn with a fallback; the original font name is kept in the document.
    pub fn missing_fonts(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .missing
            .iter()
            .map(|(f, w, i)| face_name(f, *w, *i))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Families with at least one missing face (what a font download would fetch).
    pub fn missing_families(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.missing.iter().map(|(f, _, _)| f.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Text layouts kept between frames.
    pub fn cached_layouts(&self) -> usize {
        self.cache.len()
    }

    /// Registers font file bytes (TTF/OTF/TTC). Returns the number of faces added.
    /// Missing-font checks and cached layouts are reset so text re-shapes with the new font.
    pub fn add_font_data(&mut self, bytes: Vec<u8>) -> usize {
        let added: usize = self
            .fonts
            .collection
            .register_fonts(
                parley::fontique::Blob::new(std::sync::Arc::new(bytes)),
                None,
            )
            .iter()
            .map(|(_, faces)| faces.len())
            .sum();
        if added > 0 {
            self.checked.clear();
            self.missing.clear();
            self.cache.clear();
        }
        added
    }

    /// Is this face available (exact weight within ±50, a variable weight axis, matching style)?
    fn has_face(&mut self, family: &str, weight: u16, italic: bool) -> bool {
        let Some(info) = self.fonts.collection.family_by_name(family) else {
            return false;
        };
        info.fonts().iter().any(|f| {
            let weight_ok =
                f.has_weight_axis() || (f.weight().value() - f32::from(weight)).abs() <= 50.0;
            let style_ok = italic != matches!(f.style(), parley::fontique::FontStyle::Normal);
            weight_ok && style_ok
        })
    }

    /// Laid-out size of `text` for `box_width` (used for auto-sized text boxes).
    pub fn measure(&mut self, text: &TextData, strings: &Interner, box_width: f32) -> (f32, f32) {
        let layout = self.layout(text, strings, box_width);
        (layout.width(), layout.height())
    }

    fn layout(&mut self, text: &TextData, strings: &Interner, box_width: f32) -> &Layout<u16> {
        let wraps = !matches!(text.sizing, TextSizing::AutoWidth);
        let key = layout_key(text, strings, if wraps { box_width } else { -1.0 });
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= MAX_CACHED_LAYOUTS {
                self.cache.clear();
            }
            for s in text.styles.iter() {
                let family = strings.resolve(s.family);
                let face = (family.to_owned(), s.weight, s.italic);
                if !family.is_empty() && !self.checked.contains(&face) {
                    if !self.has_face(family, s.weight, s.italic) {
                        self.missing.push(face.clone());
                    }
                    self.checked.insert(face);
                }
            }
            let mut b = self
                .layouts
                .ranged_builder(&mut self.fonts, &text.content, 1.0, true);
            for run in text.runs.iter() {
                let Some(s) = text.styles.get(run.style as usize) else {
                    continue;
                };
                let range = run.range.start as usize..run.range.end as usize;
                // Named family first; the generic fallback keeps text visible when it is missing.
                let family = format!("\"{}\", sans-serif", strings.resolve(s.family));
                b.push(
                    StyleProperty::FontFamily(FontFamily::Source(family.into())),
                    range.clone(),
                );
                b.push(StyleProperty::FontSize(s.size), range.clone());
                b.push(
                    StyleProperty::FontWeight(FontWeight::new(f32::from(s.weight))),
                    range.clone(),
                );
                if s.italic {
                    b.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
                }
                let line_height = match s.line_height {
                    typography::LineHeight::Auto => LineHeight::MetricsRelative(1.0),
                    typography::LineHeight::Px(px) => LineHeight::Absolute(px),
                    typography::LineHeight::Percent(p) => LineHeight::FontSizeRelative(p / 100.0),
                };
                b.push(StyleProperty::LineHeight(line_height), range.clone());
                let spacing = match s.letter_spacing {
                    LetterSpacing::Normal => 0.0,
                    LetterSpacing::Px(px) => px,
                    LetterSpacing::Percent(p) => s.size * p / 100.0,
                };
                b.push(StyleProperty::LetterSpacing(spacing), range.clone());
                b.push(
                    StyleProperty::Underline(s.decoration == TextDecoration::Underline),
                    range.clone(),
                );
                b.push(
                    StyleProperty::Strikethrough(s.decoration == TextDecoration::Strikethrough),
                    range.clone(),
                );
                b.push(StyleProperty::Brush(run.style), range);
            }
            let mut layout = b.build(&text.content);
            layout.break_all_lines(wraps.then_some(box_width));
            let align = match text.align_h {
                TextAlignH::Left => Alignment::Start,
                TextAlignH::Center => Alignment::Center,
                TextAlignH::Right => Alignment::End,
                TextAlignH::Justify => Alignment::Justify,
            };
            layout.align(align, AlignmentOptions::default());
            self.cache.insert(key, layout);
        }
        &self.cache[&key]
    }

    /// Laid-out lines as (text, x offset, baseline y) in box coordinates, including vertical
    /// alignment: what an exporter needs to reproduce wrapping and alignment exactly.
    pub fn lines(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
    ) -> Vec<(String, f32, f32)> {
        let layout = self.layout(text, strings, box_size.0);
        let dy = match text.align_v {
            typography::TextAlignV::Top => 0.0,
            typography::TextAlignV::Center => (box_size.1 - layout.height()) / 2.0,
            typography::TextAlignV::Bottom => box_size.1 - layout.height(),
        };
        layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                let s = text.content.get(line.text_range()).unwrap_or_default();
                (
                    s.trim_end_matches(['\n', '\r']).to_owned(),
                    m.offset,
                    m.baseline + dy,
                )
            })
            .collect()
    }

    /// Draws `text` in its `box_size` box with `transform` (local → device).
    pub fn draw(
        &mut self,
        p: &mut impl Painter,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        transform: Affine,
        alpha: f32,
    ) {
        let colors: Vec<BrushSpec> = text
            .styles
            .iter()
            .map(|s| {
                let c = s
                    .fills
                    .as_slice()
                    .iter()
                    .find_map(|p| match &p.kind {
                        PaintKind::Solid(c) if p.visible => Some(*c),
                        _ => None,
                    })
                    .unwrap_or(ude_core::styles::Color::BLACK);
                BrushSpec::Solid(convert::color(c, alpha))
            })
            .collect();
        let layout = self.layout(text, strings, box_size.0);
        let dy = match text.align_v {
            typography::TextAlignV::Top => 0.0,
            typography::TextAlignV::Center => (box_size.1 - layout.height()) / 2.0,
            typography::TextAlignV::Bottom => box_size.1 - layout.height(),
        };
        let t = transform * Affine::translate((0.0, f64::from(dy)));
        let mut glyphs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                glyphs.clear();
                glyphs.extend(run.positioned_glyphs().map(|g| GlyphPos {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                }));
                let brush = colors
                    .get(run.style().brush as usize)
                    .unwrap_or(&BrushSpec::Solid(vello::peniko::Color::BLACK));
                p.glyphs(t, run.run().font(), run.run().font_size(), brush, &glyphs);
            }
        }
    }
}

/// Caret and selection inside a text layer being edited (byte offsets into the content).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSelection(parley::Selection);

impl TextSelection {
    /// Byte range of the selected text (empty for a caret).
    pub fn range(&self) -> std::ops::Range<usize> {
        self.0.text_range()
    }
    pub fn is_caret(&self) -> bool {
        self.0.is_collapsed()
    }
    /// Byte offset of the moving end.
    pub fn focus(&self) -> usize {
        self.0.focus().index()
    }
}

/// Caret line (x, y0, y1) and selection rectangles [x0, y0, x1, y1], in box coordinates.
pub type SelectionGeometry = (Option<(f32, f32, f32)>, Vec<[f32; 4]>);

/// Caret movements for editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretMove {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
}

impl TextEngine {
    /// Vertical offset of the laid-out text inside its box (vertical alignment).
    fn dy(text: &TextData, layout: &Layout<u16>, box_size: (f32, f32)) -> f32 {
        match text.align_v {
            typography::TextAlignV::Top => 0.0,
            typography::TextAlignV::Center => (box_size.1 - layout.height()) / 2.0,
            typography::TextAlignV::Bottom => box_size.1 - layout.height(),
        }
    }

    /// Caret at `point` (box coordinates).
    pub fn select_at(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        point: (f32, f32),
    ) -> TextSelection {
        let layout = self.layout(text, strings, box_size.0);
        let dy = Self::dy(text, layout, box_size);
        TextSelection(parley::Selection::from_point(layout, point.0, point.1 - dy))
    }

    /// Extends `sel` to `point` (drag selection).
    pub fn extend_to(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        sel: TextSelection,
        point: (f32, f32),
    ) -> TextSelection {
        let layout = self.layout(text, strings, box_size.0);
        let dy = Self::dy(text, layout, box_size);
        TextSelection(sel.0.extend_to_point(layout, point.0, point.1 - dy))
    }

    /// Selection from byte offsets (after an edit), clamped to the content.
    pub fn select_range(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        anchor: usize,
        focus: usize,
    ) -> TextSelection {
        let len = text.content.len();
        let layout = self.layout(text, strings, box_size.0);
        let a =
            parley::Cursor::from_byte_index(layout, anchor.min(len), parley::Affinity::Downstream);
        let f =
            parley::Cursor::from_byte_index(layout, focus.min(len), parley::Affinity::Downstream);
        TextSelection(parley::Selection::new(a, f))
    }

    pub fn move_caret(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        sel: TextSelection,
        how: CaretMove,
        extend: bool,
    ) -> TextSelection {
        let layout = self.layout(text, strings, box_size.0);
        let s = sel.0;
        // Without Shift, Left/Right on a range collapse it to the matching edge.
        if !extend && !s.is_collapsed() && matches!(how, CaretMove::Left | CaretMove::Right) {
            let r = s.text_range();
            let at = if how == CaretMove::Left {
                r.start
            } else {
                r.end
            };
            let c = parley::Cursor::from_byte_index(layout, at, parley::Affinity::Downstream);
            return TextSelection(parley::Selection::new(c, c));
        }
        TextSelection(match how {
            CaretMove::Left => s.previous_visual(layout, extend),
            CaretMove::Right => s.next_visual(layout, extend),
            CaretMove::WordLeft => s.previous_visual_word(layout, extend),
            CaretMove::WordRight => s.next_visual_word(layout, extend),
            CaretMove::Up => s.previous_line(layout, extend),
            CaretMove::Down => s.next_line(layout, extend),
            CaretMove::LineStart => s.line_start(layout, extend),
            CaretMove::LineEnd => s.line_end(layout, extend),
        })
    }

    /// Caret line (x, y0, y1) and selection rectangles [x0, y0, x1, y1], in box coordinates.
    pub fn selection_geometry(
        &mut self,
        text: &TextData,
        strings: &Interner,
        box_size: (f32, f32),
        sel: TextSelection,
    ) -> SelectionGeometry {
        let layout = self.layout(text, strings, box_size.0);
        let dy = f64::from(Self::dy(text, layout, box_size));
        let rects = sel
            .0
            .geometry(layout)
            .into_iter()
            .map(|(b, _)| {
                [
                    b.x0 as f32,
                    (b.y0 + dy) as f32,
                    b.x1 as f32,
                    (b.y1 + dy) as f32,
                ]
            })
            .collect();
        let caret = sel.0.is_collapsed().then(|| {
            let b = sel.0.focus().geometry(layout, 1.0);
            (b.x0 as f32, (b.y0 + dy) as f32, (b.y1 + dy) as f32)
        });
        (caret, rects)
    }
}

fn layout_key(text: &TextData, strings: &Interner, width: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.content.hash(&mut h);
    width.to_bits().hash(&mut h);
    (text.align_h as u8, text.sizing as u8).hash(&mut h);
    for r in text.runs.iter() {
        (r.range.start, r.range.end, r.style).hash(&mut h);
    }
    for s in text.styles.iter() {
        strings.resolve(s.family).hash(&mut h);
        (s.weight, s.italic, s.size.to_bits(), s.decoration as u8).hash(&mut h);
        format!("{:?}{:?}", s.line_height, s.letter_spacing).hash(&mut h);
    }
    h.finish()
}

/// "Inter SemiBold Italic"-style face name for user messages.
pub fn face_name(family: &str, weight: u16, italic: bool) -> String {
    let w = match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", false) => family.to_owned(),
        ("Regular", true) => format!("{family} Italic"),
        (w, false) => format!("{family} {w}"),
        (w, true) => format!("{family} {w} Italic"),
    }
}
