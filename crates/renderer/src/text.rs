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
    missing: HashSet<String>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            cache: HashMap::new(),
            missing: HashSet::new(),
        }
    }
}

impl TextEngine {
    /// Font families referenced by the document but not installed (drawn with a fallback).
    pub fn missing_fonts(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.missing.iter().map(String::as_str).collect();
        v.sort_unstable();
        v
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
                if !family.is_empty()
                    && !self.missing.contains(family)
                    && self.fonts.collection.family_by_name(family).is_none()
                {
                    self.missing.insert(family.to_owned());
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
