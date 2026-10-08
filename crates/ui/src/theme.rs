//! Kuduy design tokens (docs/ui-design-system.md §2) applied to egui.

use egui::{Color32, CornerRadius, Stroke, Visuals};

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
    pub bg_app: Color32,
    pub bg_panel: Color32,
    pub bg_raised: Color32,
    pub bg_hover: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_component: Color32,
    pub warning: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const DARK: Tokens = Tokens {
    dark: true,
    bg_canvas: hex(0x070911),
    bg_app: hex(0x0A0D18),
    bg_panel: hex(0x1A1E30),
    bg_raised: hex(0x1E2336),
    bg_hover: hex(0x23283C),
    border_subtle: hex(0x2A3048),
    border_strong: hex(0x3B4160),
    text_primary: hex(0xEEF1FA),
    text_secondary: hex(0xC4C9DD),
    text_muted: hex(0x8D94AF),
    accent: hex(0xA78BFA),
    accent_component: hex(0xF472B6),
    warning: hex(0xFBBF24),
};

pub const LIGHT: Tokens = Tokens {
    dark: false,
    bg_canvas: hex(0xE9EBF2),
    bg_app: hex(0xF6F7FB),
    bg_panel: hex(0xFFFFFF),
    bg_raised: hex(0xFFFFFF),
    bg_hover: hex(0xEEF1FA),
    border_subtle: hex(0xE1E4EE),
    border_strong: hex(0xC4C9DD),
    text_primary: hex(0x0A0D18),
    text_secondary: hex(0x2A3048),
    text_muted: hex(0x5F6684),
    accent: hex(0x7C5CF0),
    accent_component: hex(0xDB2777),
    warning: hex(0xB45309),
};

pub const HIGH_CONTRAST: Tokens = Tokens {
    dark: true,
    bg_canvas: hex(0x000000),
    bg_app: hex(0x000000),
    bg_panel: hex(0x000000),
    bg_raised: hex(0x0A0D18),
    bg_hover: hex(0x23283C),
    border_subtle: hex(0xC4C9DD),
    border_strong: hex(0xFFFFFF),
    text_primary: hex(0xFFFFFF),
    text_secondary: hex(0xFFFFFF),
    text_muted: hex(0xEEF1FA),
    accent: hex(0xC4B5FD),
    accent_component: hex(0xF9A8D4),
    warning: hex(0xFDE68A),
};

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
    v.window_stroke = Stroke::new(1.0_f32, t.border_subtle);
    v.selection.bg_fill = t.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, t.accent);
    v.hyperlink_color = t.accent;
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
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.border_subtle);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text_secondary);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, t.border_subtle);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, t.border_strong);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, t.accent);
    ctx.set_visuals(v);
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

    /// docs/ui-design-system.md §2: primary/secondary ≥ 4.5:1, muted ≥ 3:1 on panel and raised surfaces.
    #[test]
    fn tokens_meet_wcag_aa() {
        for t in [&DARK, &LIGHT, &HIGH_CONTRAST] {
            for bg in [t.bg_panel, t.bg_raised] {
                assert!(contrast(t.text_primary, bg) >= 4.5, "primary on {bg:?}");
                assert!(contrast(t.text_secondary, bg) >= 4.5, "secondary on {bg:?}");
                assert!(contrast(t.text_muted, bg) >= 3.0, "muted on {bg:?}");
            }
        }
    }
}
