//! Kuduy design tokens applied to egui (docs/ui-design-system.md §2). Dark uses the Kuduy lime
//! accent, light uses Kuduy blue; neutrals are the Kuduy near-black / white scale.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
    HighContrast,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [Self::System, Self::Dark, Self::Light, Self::HighContrast];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::HighContrast => "High contrast",
        }
    }

    /// Tokens for this choice; `system_dark` is the OS preference.
    pub fn tokens(self, system_dark: bool) -> &'static Tokens {
        match self {
            Self::System if system_dark => &DARK,
            Self::System | Self::Light => &LIGHT,
            Self::Dark => &DARK,
            Self::HighContrast => &HIGH_CONTRAST,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub bg_canvas: Color32,
    /// Top bar and home sidebar.
    pub bg_app: Color32,
    pub bg_panel: Color32,
    /// Inputs, cards, menus.
    pub bg_raised: Color32,
    pub bg_hover: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    /// Selection, focus, active tool, primary buttons (lime in dark, blue in light).
    pub accent: Color32,
    /// Text and icons drawn on `accent`.
    pub on_accent: Color32,
    /// Selected rows and soft highlights.
    pub accent_soft: Color32,
    pub accent_component: Color32,
    /// Dev Mode, measurements.
    pub dev: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Kuduy lime and blue.
pub const LIME: Color32 = hex(0xD4FF00);
pub const BLUE: Color32 = hex(0x2563EB);

pub const DARK: Tokens = Tokens {
    dark: true,
    bg_canvas: hex(0x181818),
    bg_app: hex(0x0A0A0A),
    bg_panel: hex(0x111111),
    bg_raised: hex(0x1C1C1C),
    bg_hover: hex(0x262626),
    border_subtle: hex(0x262626),
    border_strong: hex(0x3A3A3A),
    text_primary: hex(0xF5F5F5),
    text_secondary: hex(0xC7C7C7),
    text_muted: hex(0x8E8E8E),
    accent: LIME,
    on_accent: hex(0x0A0A0A),
    accent_soft: Color32::from_rgba_premultiplied(30, 36, 0, 60),
    accent_component: hex(0xF472B6),
    dev: hex(0x67E8F9),
    success: hex(0x4ADE80),
    warning: hex(0xFBBF24),
    error: hex(0xF87171),
};

pub const LIGHT: Tokens = Tokens {
    dark: false,
    bg_canvas: hex(0xF1F1F1),
    bg_app: hex(0xFFFFFF),
    bg_panel: hex(0xFFFFFF),
    bg_raised: hex(0xF5F5F5),
    bg_hover: hex(0xEDEDED),
    border_subtle: hex(0xE9E9E9),
    border_strong: hex(0xBFBFBF),
    text_primary: hex(0x0A0A0A),
    text_secondary: hex(0x4A4A4A),
    text_muted: hex(0x6B6B6B),
    accent: BLUE,
    on_accent: hex(0xFFFFFF),
    accent_soft: Color32::from_rgba_premultiplied(14, 39, 92, 40),
    accent_component: hex(0xDB2777),
    dev: hex(0x0891B2),
    success: hex(0x16A34A),
    warning: hex(0xB45309),
    error: hex(0xDC2626),
};

pub const HIGH_CONTRAST: Tokens = Tokens {
    dark: true,
    bg_canvas: hex(0x000000),
    bg_app: hex(0x000000),
    bg_panel: hex(0x000000),
    bg_raised: hex(0x0A0A0A),
    bg_hover: hex(0x262626),
    border_subtle: hex(0xC7C7C7),
    border_strong: hex(0xFFFFFF),
    text_primary: hex(0xFFFFFF),
    text_secondary: hex(0xFFFFFF),
    text_muted: hex(0xEDEDED),
    accent: LIME,
    on_accent: hex(0x000000),
    accent_soft: Color32::from_rgba_premultiplied(42, 51, 0, 80),
    accent_component: hex(0xF9A8D4),
    dev: hex(0xA5F3FC),
    success: hex(0x86EFAC),
    warning: hex(0xFDE68A),
    error: hex(0xFCA5A5),
};

/// The icon font family name (see [`install_fonts`]).
pub const ICONS: &str = "lucide";

/// Adds the Lucide icon font; call once at startup.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        ICONS.into(),
        egui::FontData::from_static(include_bytes!("../../../assets/lucide/lucide.ttf")).into(),
    );
    fonts
        .families
        .insert(FontFamily::Name(ICONS.into()), vec![ICONS.into()]);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, t: &Tokens) {
    let mut v = if t.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = t.bg_panel;
    v.window_fill = t.bg_raised;
    v.extreme_bg_color = t.bg_raised;
    v.faint_bg_color = t.bg_app;
    v.code_bg_color = t.bg_raised;
    v.window_stroke = Stroke::new(1.0_f32, t.border_subtle);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(4);
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, t.accent);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    let radius = CornerRadius::same(6);
    for (w, bg) in [
        (&mut v.widgets.noninteractive, t.bg_panel),
        (&mut v.widgets.inactive, t.bg_raised),
        (&mut v.widgets.hovered, t.bg_hover),
        (&mut v.widgets.active, t.bg_hover),
        (&mut v.widgets.open, t.bg_hover),
    ] {
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
        w.corner_radius = radius;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.border_subtle);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text_secondary);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, t.border_strong);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, t.accent);
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(12.5)),
            (TextStyle::Button, FontId::proportional(12.5)),
            (TextStyle::Heading, FontId::proportional(18.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
    });
}

/// WCAG 2.x contrast ratio.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let ch = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/ui-design-system.md §2: primary/secondary ≥ 4.5:1, muted ≥ 3:1 on panel and raised
    /// surfaces; text on the accent ≥ 4.5:1.
    #[test]
    fn tokens_meet_wcag_aa() {
        for t in [&DARK, &LIGHT, &HIGH_CONTRAST] {
            for bg in [t.bg_panel, t.bg_raised] {
                assert!(contrast(t.text_primary, bg) >= 4.5, "primary on {bg:?}");
                assert!(contrast(t.text_secondary, bg) >= 4.5, "secondary on {bg:?}");
                assert!(contrast(t.text_muted, bg) >= 3.0, "muted on {bg:?}");
            }
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "text on accent");
        }
    }
}
