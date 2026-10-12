//! Small building blocks shared by the panels: icons, icon buttons, sections, segmented
//! controls and primary buttons, all styled from the [`Tokens`].

use egui::{
    Align2, Color32, CornerRadius, DragValue, FontFamily, FontId, Response, RichText, Sense,
    Stroke, Ui, vec2,
};

use crate::theme::{ICONS, Tokens};

/// A Lucide icon as text.
pub fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph)
        .family(FontFamily::Name(ICONS.into()))
        .size(size)
        .color(color)
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// Square icon button; `active` fills it with the accent (selected tool, toggled mode).
pub fn icon_button(ui: &mut Ui, t: &Tokens, glyph: &str, tooltip: &str, active: bool) -> Response {
    icon_button_sized(ui, t, glyph, tooltip, active, 30.0, 16.0)
}

pub fn icon_button_sized(
    ui: &mut Ui,
    t: &Tokens,
    glyph: &str,
    tooltip: &str,
    active: bool,
    side: f32,
    icon_size: f32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let enabled = ui.is_enabled();
    let p = ui.painter();
    let (bg, fg) = if active {
        (t.accent, t.on_accent)
    } else if resp.hovered() && enabled {
        (t.bg_hover, t.text_primary)
    } else {
        (
            Color32::TRANSPARENT,
            if enabled {
                t.text_secondary
            } else {
                t.text_muted.gamma_multiply(0.6)
            },
        )
    };
    p.rect_filled(rect, CornerRadius::same(6), bg);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        icon_font(icon_size),
        fg,
    );
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Filled accent button with an optional leading icon.
pub fn primary_button(ui: &mut Ui, t: &Tokens, glyph: Option<&str>, label: &str) -> Response {
    let font = FontId::proportional(12.5);
    let text_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(label.to_owned(), font.clone(), t.on_accent)
            .size()
            .x
    });
    let icon_w = if glyph.is_some() { 20.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + icon_w + 24.0, 30.0), Sense::click());
    let p = ui.painter();
    let bg = if resp.hovered() {
        t.accent.gamma_multiply(0.88)
    } else {
        t.accent
    };
    p.rect_filled(rect, CornerRadius::same(7), bg);
    let mut x = rect.left() + 12.0;
    if let Some(g) = glyph {
        p.text(
            egui::pos2(x + 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            g,
            icon_font(15.0),
            t.on_accent,
        );
        x += icon_w;
    }
    p.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        t.on_accent,
    );
    resp
}

/// Outlined secondary button with an optional leading icon.
pub fn secondary_button(ui: &mut Ui, t: &Tokens, glyph: Option<&str>, label: &str) -> Response {
    let font = FontId::proportional(12.5);
    let text_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(label.to_owned(), font.clone(), t.text_primary)
            .size()
            .x
    });
    let icon_w = if glyph.is_some() { 20.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + icon_w + 24.0, 30.0), Sense::click());
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(7),
        if resp.hovered() {
            t.bg_hover
        } else {
            t.bg_raised
        },
        Stroke::new(1.0_f32, t.border_subtle),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 12.0;
    if let Some(g) = glyph {
        p.text(
            egui::pos2(x + 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            g,
            icon_font(15.0),
            t.text_secondary,
        );
        x += icon_w;
    }
    p.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        t.text_primary,
    );
    resp
}

/// Section with a header row (title + optional trailing content) and a body.
pub fn section<R>(
    ui: &mut Ui,
    t: &Tokens,
    title: &str,
    trailing: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .size(11.5)
                .strong()
                .color(t.text_primary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), trailing);
    });
    ui.add_space(4.0);
    let r = body(ui);
    ui.add_space(8.0);
    let w = ui.available_width();
    let (line, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, t.border_subtle);
    r
}

/// Segmented control; returns true when the selection changed.
pub fn segmented(ui: &mut Ui, t: &Tokens, options: &[&str], selected: &mut usize) -> bool {
    let mut changed = false;
    let font = FontId::proportional(12.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|o| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap((*o).to_owned(), font.clone(), t.text_primary)
                    .size()
                    .x
            }) + 18.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(7), t.bg_raised);
    let mut x = rect.left() + 2.0;
    for (i, (opt, w)) in options.iter().zip(&widths).enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, 24.0));
        let resp = ui.interact(r, ui.id().with(("seg", i, *opt)), Sense::click());
        let on = *selected == i;
        if on {
            ui.painter()
                .rect_filled(r, CornerRadius::same(6), t.bg_panel);
            ui.painter().rect_stroke(
                r,
                CornerRadius::same(6),
                Stroke::new(1.0_f32, t.border_subtle),
                egui::StrokeKind::Inside,
            );
        }
        let color = if on {
            t.text_primary
        } else if resp.hovered() {
            t.text_secondary
        } else {
            t.text_muted
        };
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, *opt, font.clone(), color);
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Small muted label used in property grids.
pub fn field_label(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.label(RichText::new(text).size(11.0).color(t.text_muted))
}

/// Display unit for lengths in the inspector. Documents store pixels (CSS px, 96 per inch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Px,
    Pt,
    In,
    Cm,
    Mm,
}

impl Unit {
    pub const ALL: [Unit; 5] = [Unit::Px, Unit::Pt, Unit::In, Unit::Cm, Unit::Mm];

    pub fn label(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Pt => "pt",
            Unit::In => "in",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
        }
    }

    /// Pixels per unit.
    pub fn px(self) -> f64 {
        match self {
            Unit::Px => 1.0,
            Unit::Pt => 96.0 / 72.0,
            Unit::In => 96.0,
            Unit::Cm => 96.0 / 2.54,
            Unit::Mm => 96.0 / 25.4,
        }
    }

    fn decimals(self) -> usize {
        match self {
            Unit::Px | Unit::Pt | Unit::Mm => 2,
            Unit::In | Unit::Cm => 3,
        }
    }
}

/// Parses a length typed by the user, in pixels: a number in `unit`, or with its own unit
/// ("2in", "10 mm", "12pt", "5px").
pub fn parse_length(s: &str, unit: Unit) -> Option<f64> {
    let s = s.trim().to_lowercase();
    let (num, u) = match Unit::ALL.iter().find(|u| s.ends_with(u.label())) {
        Some(u) => (s[..s.len() - 2].trim().to_owned(), *u),
        None => (s, unit),
    };
    num.replace(',', ".")
        .parse::<f64>()
        .ok()
        .map(|v| v * u.px())
}

/// A drag field for a length stored in pixels, shown and typed in `unit`.
pub fn length(value: &mut f64, unit: Unit) -> DragValue<'_> {
    let f = unit.px();
    let d = unit.decimals();
    DragValue::new(value)
        .custom_formatter(move |px, _| {
            let v = format!("{:.*}", d, px / f);
            let v = v.trim_end_matches('0').trim_end_matches('.');
            format!("{v} {}", unit.label())
        })
        .custom_parser(move |text| parse_length(text, unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_accept_any_unit() {
        assert_eq!(parse_length("2in", Unit::Px), Some(192.0));
        assert_eq!(parse_length("12", Unit::Pt), Some(16.0));
        assert_eq!(parse_length("25.4 mm", Unit::Px), Some(96.0));
        assert_eq!(parse_length("1,5", Unit::In), Some(144.0));
        assert_eq!(parse_length("abc", Unit::Px), None);
    }
}
