//! Paints, strokes, effects and blend modes.

use std::sync::Arc;

use crate::assets::AssetId;
use crate::geometry::{Affine, Point};

/// Non-premultiplied sRGB colour, components in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b), q(self.a)]
    }

    /// `#RRGGBB`, or `#RRGGBBAA` when not fully opaque.
    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 {
            format!("#{r:02X}{g:02X}{b:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum BlendMode {
    /// Groups only: children blend with the backdrop directly (Photoshop/Figma default for groups).
    PassThrough,
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
    Angular,
    Diamond,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Gradient in the unit square, mapped onto the node box by `transform`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Box<[GradientStop]>,
    pub transform: Affine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ImageScaleMode {
    #[default]
    Fill,
    Fit,
    Crop,
    Tile,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaintKind {
    Solid(Color),
    Gradient(Arc<Gradient>),
    Image {
        asset: AssetId,
        mode: ImageScaleMode,
        transform: Option<Box<Affine>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    pub kind: PaintKind,
    pub opacity: f32,
    pub blend: BlendMode,
    pub visible: bool,
}

impl Paint {
    pub fn solid(color: Color) -> Self {
        Self {
            kind: PaintKind::Solid(color),
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: true,
        }
    }
}

/// Ordered paint stack. Empty lists allocate nothing; non-empty lists are shared.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Paints(Option<Arc<[Paint]>>);

impl Paints {
    pub fn new(paints: Vec<Paint>) -> Self {
        if paints.is_empty() {
            Self(None)
        } else {
            Self(Some(paints.into()))
        }
    }

    pub fn as_slice(&self) -> &[Paint] {
        self.0.as_deref().unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Heap bytes attributed to this holder (shared storage is split between owners).
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            None => 0,
            Some(a) => {
                let own = size_of_val::<[Paint]>(a) + 2 * size_of::<usize>();
                own / Arc::strong_count(a)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrokeAlign {
    Inside,
    #[default]
    Center,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stroke {
    pub paints: Paints,
    pub width: f32,
    pub align: StrokeAlign,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dashes: Box<[f32]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub blend: BlendMode,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    DropShadow(Shadow),
    InnerShadow(Shadow),
    LayerBlur { radius: f32, visible: bool },
    BackgroundBlur { radius: f32, visible: bool },
}

impl Effect {
    /// How far the effect can paint outside the node bounds (for culling and dirty rects).
    pub fn outset(&self) -> f32 {
        match self {
            Effect::DropShadow(s) => {
                s.blur * 1.5 + s.spread.max(0.0) + s.offset.x.abs().max(s.offset.y.abs())
            }
            Effect::LayerBlur { radius, .. } => radius * 1.5,
            Effect::InnerShadow(_) | Effect::BackgroundBlur { .. } => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Color::from_rgba8(0x25, 0x63, 0xEB, 255);
        assert_eq!(c.to_hex(), "#2563EB");
        assert_eq!(Color::from_rgba8(0, 0, 0, 128).to_hex(), "#00000080");
    }

    #[test]
    fn empty_paints_do_not_allocate() {
        let p = Paints::new(vec![]);
        assert!(p.is_empty());
        assert_eq!(p.heap_bytes(), 0);
        assert!(
            size_of::<Paint>() <= 32,
            "Paint is {} bytes",
            size_of::<Paint>()
        );
    }
}
