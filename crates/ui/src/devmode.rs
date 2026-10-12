//! Dev tab: inspect values and code for the selection (ude-codegen), with one-click copy.
//! Generated code is cached per (document revision, layer, target).

use egui::{RichText, Ui};
use ude_codegen::Target;
use ude_core::styles::PaintKind;
use ude_core::{Document, NodeId, NodeKind};
use ude_editor::Selection;

use crate::icons;
use crate::inspector::to_egui;
use crate::theme::Tokens;
use crate::widgets::{field_label, icon, icon_button_sized, section, segmented};

const TARGETS: [(Target, &str); 5] = [
    (Target::Css, "CSS"),
    (Target::Tailwind, "Tailwind"),
    (Target::React, "React"),
    (Target::Vue, "Vue"),
    (Target::Html, "HTML"),
];

/// Code for (revision, layer, target): the code and its notes.
type Cached = ((u64, NodeId, usize), String, Vec<String>);

#[derive(Default)]
pub struct DevMode {
    target: usize,
    cache: Option<Cached>,
    copied: Option<(String, f64)>,
}

impl DevMode {
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        selection: &Selection,
        page: NodeId,
        t: &Tokens,
    ) {
        let id = match selection.ids() {
            [id] => *id,
            [] => page,
            _ => {
                ui.add_space(12.0);
                ui.label(RichText::new("Select one layer to inspect it").color(t.text_muted));
                return;
            }
        };
        let Some(n) = doc.node(id) else { return };
        let now = ui.input(|i| i.time);

        // Properties with copy buttons.
        let mut props: Vec<(&str, String)> = Vec::new();
        if !matches!(n.kind, NodeKind::Page) {
            let tr = n.transform.translation();
            props.push(("Width", format!("{}px", trim(n.size.w))));
            props.push(("Height", format!("{}px", trim(n.size.h))));
            props.push(("Left", format!("{}px", trim(tr.x))));
            props.push(("Top", format!("{}px", trim(tr.y))));
            if n.corner_radius > 0.0 {
                props.push(("Radius", format!("{}px", trim(n.corner_radius))));
            }
            if n.opacity < 1.0 {
                props.push(("Opacity", format!("{}%", (n.opacity * 100.0).round())));
            }
        }
        let fills: Vec<_> = match &n.kind {
            NodeKind::Text(text) => text
                .styles
                .first()
                .map(|s| s.fills.as_slice().to_vec())
                .unwrap_or_default(),
            _ => n.fills.as_slice().to_vec(),
        };
        if let NodeKind::Text(text) = &n.kind
            && let Some(s) = text.styles.first()
        {
            props.push(("Font", doc.strings.resolve(s.family).to_owned()));
            props.push(("Size", format!("{}px", trim(s.size))));
            props.push(("Weight", s.weight.to_string()));
        }
        ui.add_space(4.0);
        section(
            ui,
            t,
            "Inspect",
            |_| {},
            |ui| {
                egui::Grid::new("dev_props")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (k, v) in &props {
                            field_label(ui, t, k);
                            ui.label(RichText::new(v).monospace().color(t.text_primary));
                            if icon_button_sized(ui, t, icons::COPY, "Copy", false, 20.0, 11.0)
                                .clicked()
                            {
                                ui.ctx().copy_text(v.clone());
                                self.copied = Some((v.clone(), now));
                            }
                            ui.end_row();
                        }
                        for p in &fills {
                            if let PaintKind::Solid(c) = p.kind {
                                let hex = c.to_hex();
                                field_label(ui, t, "Color");
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::vec2(14.0, 14.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(r, 3.0, to_egui(c));
                                    ui.label(RichText::new(&hex).monospace().color(t.text_primary));
                                });
                                if icon_button_sized(ui, t, icons::COPY, "Copy", false, 20.0, 11.0)
                                    .clicked()
                                {
                                    ui.ctx().copy_text(hex.clone());
                                    self.copied = Some((hex, now));
                                }
                                ui.end_row();
                            }
                        }
                    });
            },
        );

        // Code.
        ui.add_space(6.0);
        let labels: Vec<&str> = TARGETS.iter().map(|(_, l)| *l).collect();
        segmented(ui, t, &labels, &mut self.target);
        let key = (doc.revision(), id, self.target);
        if self.cache.as_ref().is_none_or(|(k, _, _)| *k != key) {
            let (code, notes) = match ude_codegen::generate(doc, id, TARGETS[self.target].0) {
                Ok(o) => (o.code, o.notes),
                Err(e) => (format!("/* {e} */"), Vec::new()),
            };
            self.cache = Some((key, code, notes));
        }
        let Some((_, code, notes)) = &self.cache else {
            return;
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(icon(icons::CODE_XML, 13.0, t.dev));
            ui.label(
                RichText::new(TARGETS[self.target].1)
                    .strong()
                    .color(t.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let just_copied = self
                    .copied
                    .as_ref()
                    .is_some_and(|(c, at)| c == code && now - at < 1.5);
                let label = if just_copied { "Copied" } else { "Copy code" };
                let glyph = if just_copied {
                    icons::CHECK
                } else {
                    icons::COPY
                };
                if crate::widgets::secondary_button(ui, t, Some(glyph), label).clicked() {
                    ui.ctx().copy_text(code.clone());
                    self.copied = Some((code.clone(), now));
                }
            });
        });
        for note in notes {
            ui.horizontal(|ui| {
                ui.label(icon(icons::INFO, 11.0, t.text_muted));
                ui.label(RichText::new(note).size(11.0).color(t.text_muted));
            });
        }
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(t.bg_raised)
            .corner_radius(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .max_height(ui.available_height() - 8.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut text = code.as_str();
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .frame(egui::Frame::NONE),
                        );
                    });
            });
    }
}

fn trim(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}
