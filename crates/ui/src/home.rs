//! Home screen: recent files with thumbnails, search, and New / Open / Import, shown when no
//! document is open (like Figma's file browser).

use std::path::PathBuf;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::theme::{ThemeChoice, Tokens};
use crate::widgets::{icon, icon_font, primary_button, secondary_button};
use crate::{Action, icons};

/// A recently opened file, as the home screen shows it.
pub struct Recent {
    pub path: PathBuf,
    pub name: String,
    /// Lowercase extension, e.g. "fig".
    pub ext: String,
    /// "2 hours ago", "Yesterday", …
    pub modified: String,
    pub thumbnail: Option<egui::TextureHandle>,
}

#[derive(Default)]
pub struct Home {
    search: String,
}

const FORMATS: [&str; 12] = [
    "KRT", "FIG", "JAM", "DECK", "SKETCH", "XD", "PSD", "AI", "PDF", "SVG", "PNG", "JPG",
];

fn format_icon(ext: &str) -> &'static str {
    match ext {
        "png" | "jpg" | "jpeg" | "webp" | "psd" | "psb" => icons::FILE_IMAGE,
        "svg" | "pdf" | "ai" => icons::FILE_CODE,
        _ => icons::LAYOUT_TEMPLATE,
    }
}

impl Home {
    pub fn ui(
        &mut self,
        root: &mut Ui,
        recents: &[Recent],
        theme: &mut ThemeChoice,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        egui::Panel::left("home_nav")
            .exact_size(232.0)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_app)
                    .inner_margin(egui::Margin::symmetric(14, 16)),
            )
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(8), t.accent);
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        "K",
                        FontId::proportional(16.0),
                        t.on_accent,
                    );
                    ui.label(
                        RichText::new("KuArt")
                            .size(17.0)
                            .strong()
                            .color(t.text_primary),
                    );
                });
                ui.add_space(18.0);
                nav_item(ui, t, icons::CLOCK, "Recents", true);
                if nav_item(ui, t, icons::FILE_PLUS, "New design", false).clicked() {
                    actions.push(Action::New);
                }
                if nav_item(ui, t, icons::FOLDER_OPEN, "Open file…", false).clicked() {
                    actions.push(Action::Open);
                }
                if nav_item(ui, t, icons::FILE_INPUT, "Import…", false).clicked() {
                    actions.push(Action::Import);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(concat!("KuArt ", env!("CARGO_PKG_VERSION"), " · offline"))
                            .size(11.0)
                            .color(t.text_muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for (c, glyph, tip) in [
                            (ThemeChoice::System, icons::SUN_MOON, "System theme"),
                            (ThemeChoice::Light, icons::SUN, "Light (Kuduy blue)"),
                            (ThemeChoice::Dark, icons::MOON, "Dark (Kuduy lime)"),
                            (ThemeChoice::HighContrast, icons::CONTRAST, "High contrast"),
                        ] {
                            if crate::widgets::icon_button(ui, t, glyph, tip, *theme == c).clicked()
                            {
                                *theme = c;
                            }
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.bg_panel)
                    .inner_margin(egui::Margin::symmetric(32, 24)),
            )
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Recent files")
                            .size(22.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, t, Some(icons::PLUS), "New design").clicked() {
                            actions.push(Action::New);
                        }
                        if secondary_button(ui, t, Some(icons::FILE_INPUT), "Import").clicked() {
                            actions.push(Action::Import);
                        }
                        if secondary_button(ui, t, Some(icons::FOLDER_OPEN), "Open").clicked() {
                            actions.push(Action::Open);
                        }
                        ui.add_space(8.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search files")
                                .desired_width(220.0),
                        );
                        ui.label(icon(icons::SEARCH, 14.0, t.text_muted));
                    });
                });
                ui.add_space(18.0);

                let q = self.search.to_lowercase();
                let shown: Vec<&Recent> = recents
                    .iter()
                    .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                    .collect();
                if shown.is_empty() {
                    empty_state(ui, t, recents.is_empty());
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        let card = vec2(232.0, 214.0);
                        let cols = ((ui.available_width() + 16.0) / (card.x + 16.0))
                            .floor()
                            .max(1.0) as usize;
                        for row in shown.chunks(cols) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 16.0;
                                for r in row {
                                    recent_card(ui, t, r, card, actions);
                                }
                            });
                            ui.add_space(16.0);
                        }
                    });
            });
    }
}

fn nav_item(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(7), t.bg_raised);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(7), t.bg_hover);
    }
    p.text(
        Pos2::new(rect.left() + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        icon_font(15.0),
        if active { t.accent } else { t.text_secondary },
    );
    p.text(
        Pos2::new(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        t.text_primary,
    );
    resp
}

fn recent_card(ui: &mut Ui, t: &Tokens, r: &Recent, size: egui::Vec2, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let border = if resp.hovered() {
        t.accent
    } else {
        t.border_subtle
    };
    p.rect(
        rect,
        CornerRadius::same(10),
        t.bg_raised,
        Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );
    let thumb = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(size.x - 2.0, 150.0));
    p.rect_filled(
        thumb,
        CornerRadius {
            nw: 9,
            ne: 9,
            sw: 0,
            se: 0,
        },
        t.bg_canvas,
    );
    match &r.thumbnail {
        Some(tex) => {
            // Fit the thumbnail inside the preview area.
            let ts = tex.size_vec2();
            let s = (thumb.width() / ts.x).min(thumb.height() / ts.y).min(1.0);
            let img = Rect::from_center_size(thumb.center(), ts * s);
            p.image(
                tex.id(),
                img,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            p.text(
                thumb.center(),
                Align2::CENTER_CENTER,
                format_icon(&r.ext),
                icon_font(30.0),
                t.text_muted,
            );
        }
    }
    p.text(
        Pos2::new(rect.left() + 12.0, thumb.bottom() + 20.0),
        Align2::LEFT_CENTER,
        &r.name,
        FontId::proportional(13.0),
        t.text_primary,
    );
    p.text(
        Pos2::new(rect.left() + 12.0, thumb.bottom() + 40.0),
        Align2::LEFT_CENTER,
        format!("{} · {}", r.ext.to_uppercase(), r.modified),
        FontId::proportional(11.0),
        t.text_muted,
    );
    let resp = resp.on_hover_text(r.path.display().to_string());
    if resp.clicked() {
        actions.push(Action::OpenRecent(r.path.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button("Open").clicked() {
            actions.push(Action::OpenRecent(r.path.clone()));
            ui.close();
        }
        if ui.button("Remove from recents").clicked() {
            actions.push(Action::RemoveRecent(r.path.clone()));
            ui.close();
        }
    });
}

fn empty_state(ui: &mut Ui, t: &Tokens, nothing_yet: bool) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(icon(icons::LAYOUT_TEMPLATE, 44.0, t.text_muted));
        ui.add_space(10.0);
        ui.label(
            RichText::new(if nothing_yet {
                "No recent files yet"
            } else {
                "No files match your search"
            })
            .size(16.0)
            .strong()
            .color(t.text_primary),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new("Create a design, or open a file from any of these formats:")
                .color(t.text_muted),
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            // Chip widths: measured label + 16 px margin, plus the spacing between chips.
            let font = egui::FontId::monospace(11.0);
            let gap = ui.spacing().item_spacing.x;
            let total: f32 = FORMATS
                .iter()
                .map(|f| {
                    let g = ui.painter().layout_no_wrap(
                        (*f).to_owned(),
                        font.clone(),
                        t.text_secondary,
                    );
                    g.size().x + 16.0 + gap
                })
                .sum::<f32>()
                - gap;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for f in FORMATS {
                egui::Frame::new()
                    .fill(t.bg_raised)
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(f)
                                .size(11.0)
                                .monospace()
                                .color(t.text_secondary),
                        );
                    });
            }
        });
    });
}
