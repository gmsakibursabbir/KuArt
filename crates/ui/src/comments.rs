//! Comments tab: comments on the current page (written here or read from imported files),
//! with replies, resolve and a composer. Clicking a comment centres it on the canvas.

use egui::{Align2, CornerRadius, FontId, RichText, Ui, vec2};
use ude_core::Document;

use crate::theme::Tokens;
use crate::widgets::{icon, icon_button_sized, primary_button};
use crate::{Action, icons};

#[derive(Default)]
pub struct CommentsPanel {
    draft: String,
    reply_to: Option<(usize, String)>,
    show_resolved: bool,
}

fn ago(created: u64) -> String {
    if created == 0 {
        return String::new();
    }
    let secs = ude_core::comments::now().saturating_sub(created);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

fn avatar(ui: &mut Ui, t: &Tokens, name: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
    let initial = name.chars().next().map_or('?', |c| c.to_ascii_uppercase());
    ui.painter().circle_filled(r.center(), 11.0, t.accent_soft);
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        initial,
        FontId::proportional(11.0),
        t.text_primary,
    );
}

impl CommentsPanel {
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        page_index: u32,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(6.0);
        let on_page: Vec<(usize, &ude_core::comments::Comment)> = doc
            .comments
            .iter()
            .enumerate()
            .filter(|(_, c)| c.page == page_index)
            .collect();
        let open = on_page.iter().filter(|(_, c)| !c.resolved).count();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{open} open"))
                    .size(11.5)
                    .color(t.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(
                    &mut self.show_resolved,
                    RichText::new("Show resolved").size(11.5),
                );
            });
        });
        ui.add_space(6.0);

        // Composer: new comments are pinned at the centre of the view.
        ui.add(
            egui::TextEdit::multiline(&mut self.draft)
                .hint_text("Add a comment…")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Pinned at the centre of the view")
                    .size(11.0)
                    .color(t.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, t, Some(icons::PLUS), "Comment").clicked()
                    && !self.draft.trim().is_empty()
                {
                    actions.push(Action::AddComment(
                        std::mem::take(&mut self.draft).trim().to_owned(),
                    ));
                }
            });
        });
        ui.add_space(10.0);

        if on_page.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(icon(icons::INFO, 18.0, t.text_muted));
                ui.label(RichText::new("No comments on this page").color(t.text_muted));
                ui.label(
                    RichText::new(
                        "Comments in PDF, Illustrator and SVG files appear here when imported.",
                    )
                    .size(11.0)
                    .color(t.text_muted),
                );
            });
            return;
        }
        for (number, (i, c)) in on_page.iter().enumerate() {
            if c.resolved && !self.show_resolved {
                continue;
            }
            let i = *i;
            let frame = egui::Frame::new()
                .fill(t.bg_raised)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let author = if c.author.is_empty() {
                            "Imported"
                        } else {
                            c.author.as_str()
                        };
                        avatar(ui, t, author);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(format!("#{}  {author}", number + 1))
                                    .strong()
                                    .size(12.0)
                                    .color(t.text_primary),
                            );
                            let meta = [ago(c.created), c.source.clone()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            if !meta.is_empty() {
                                ui.label(RichText::new(meta).size(10.5).color(t.text_muted));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if icon_button_sized(ui, t, icons::TRASH_2, "Delete", false, 22.0, 12.0)
                                .clicked()
                            {
                                actions.push(Action::DeleteComment(i));
                            }
                            let tip = if c.resolved { "Reopen" } else { "Resolve" };
                            if icon_button_sized(ui, t, icons::CHECK, tip, c.resolved, 22.0, 12.0)
                                .clicked()
                            {
                                actions.push(Action::ResolveComment(i, !c.resolved));
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new(&c.text).color(if c.resolved {
                        t.text_muted
                    } else {
                        t.text_primary
                    }));
                    for r in &c.replies {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            avatar(ui, t, if r.author.is_empty() { "?" } else { &r.author });
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!("{}  {}", r.author, ago(r.created)))
                                        .size(11.0)
                                        .color(t.text_secondary),
                                );
                                ui.label(RichText::new(&r.text).color(t.text_primary));
                            });
                        });
                    }
                    ui.add_space(6.0);
                    let replying = matches!(&self.reply_to, Some((target, _)) if *target == i);
                    if replying {
                        let mut keep = true;
                        if let Some((_, text)) = &mut self.reply_to {
                            ui.add(
                                egui::TextEdit::singleline(text)
                                    .hint_text("Reply…")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Reply").clicked() && !text.trim().is_empty() {
                                    actions.push(Action::ReplyComment(i, text.trim().to_owned()));
                                    keep = false;
                                } else if ui.button("Cancel").clicked() {
                                    keep = false;
                                }
                            });
                        }
                        if !keep {
                            self.reply_to = None;
                        }
                    } else if ui.small_button("Reply").clicked() {
                        self.reply_to = Some((i, String::new()));
                    }
                });
            if frame.response.interact(egui::Sense::click()).clicked() {
                actions.push(Action::FocusComment(i));
            }
            ui.add_space(8.0);
        }
    }
}
