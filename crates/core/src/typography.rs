//! Text content with styled runs. Shaping lives in the renderer (parley).

use std::ops::Range;
use std::sync::Arc;

use crate::intern::Symbol;
use crate::styles::Paints;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LineHeight {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LetterSpacing {
    #[default]
    Normal,
    Px(f32),
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    Strikethrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextCase {
    #[default]
    Original,
    Upper,
    Lower,
    Title,
    SmallCaps,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: Symbol,
    pub weight: u16,
    pub italic: bool,
    pub size: f32,
    pub line_height: LineHeight,
    pub letter_spacing: LetterSpacing,
    pub decoration: TextDecoration,
    pub case: TextCase,
    pub fills: Paints,
}

/// A byte range of `TextData::content` using `styles[style]`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub range: Range<u32>,
    pub style: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextSizing {
    AutoWidth,
    #[default]
    AutoHeight,
    Fixed,
    Truncate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlignH {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlignV {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextData {
    pub content: Box<str>,
    pub runs: Box<[TextRun]>,
    pub styles: Arc<[TextStyle]>,
    pub sizing: TextSizing,
    pub align_h: TextAlignH,
    pub align_v: TextAlignV,
    pub paragraph_spacing: f32,
}

impl TextData {
    /// Single-style text.
    pub fn plain(content: &str, style: TextStyle) -> Self {
        let len = u32::try_from(content.len()).expect("text longer than 4 GiB");
        Self {
            content: content.into(),
            runs: Box::new([TextRun {
                range: 0..len,
                style: 0,
            }]),
            styles: Arc::new([style]),
            sizing: TextSizing::default(),
            align_h: TextAlignH::default(),
            align_v: TextAlignV::default(),
            paragraph_spacing: 0.0,
        }
    }

    /// Runs must be ordered, contiguous, on char boundaries and reference existing styles.
    pub fn validate(&self) -> bool {
        let mut pos = 0u32;
        for run in self.runs.iter() {
            if run.range.start != pos
                || run.range.end < run.range.start
                || !self.content.is_char_boundary(run.range.end as usize)
                || run.style as usize >= self.styles.len()
            {
                return false;
            }
            pos = run.range.end;
        }
        pos as usize == self.content.len()
    }

    pub fn heap_bytes(&self) -> usize {
        self.content.len()
            + self.runs.len() * size_of::<TextRun>()
            + (self.styles.len() * size_of::<TextStyle>()) / Arc::strong_count(&self.styles)
    }
}

/// Font style name ("Semi Bold Italic", "ExtraLight") → (weight, italic), as design files
/// name faces.
pub fn parse_font_style(style: &str) -> (u16, bool) {
    let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let italic = s.contains("italic") || s.contains("oblique");
    // Compound names first: "semibold" also contains "bold".
    let weight = [
        ("thin", 100),
        ("hairline", 100),
        ("extralight", 200),
        ("ultralight", 200),
        ("semibold", 600),
        ("demibold", 600),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("heavy", 800),
        ("black", 900),
        ("light", 300),
        ("medium", 500),
        ("bold", 700),
    ]
    .iter()
    .find(|(k, _)| s.contains(k))
    .map_or(400, |&(_, w)| w);
    (weight, italic)
}
