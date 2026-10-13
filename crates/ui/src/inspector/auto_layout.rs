//! Auto layout section (frames): direction, gap, padding, alignment, hug/fixed sizing; and
//! for children of an auto layout frame, fixed/hug/fill resizing and "ignore auto layout".

use egui::{RichText, Ui};
use ude_core::layout::{ChildSizing, ContainerSizing, CrossAlign, LayoutDirection, MainAlign};
use ude_core::{Document, Node, NodeId, NodeKind, Prop};

use super::Inspector;
use crate::theme::Tokens;
use crate::widgets::{field_label, icon_button_sized, length, section};
use crate::{Action, icons};

const MAIN: [(MainAlign, &str); 4] = [
    (MainAlign::Start, "Start"),
    (MainAlign::Center, "Center"),
    (MainAlign::End, "End"),
    (MainAlign::SpaceBetween, "Space between"),
];
const CROSS: [(CrossAlign, &str); 4] = [
    (CrossAlign::Start, "Start"),
    (CrossAlign::Center, "Center"),
    (CrossAlign::End, "End"),
    (CrossAlign::Stretch, "Stretch"),
];
const CHILD: [(ChildSizing, &str); 3] = [
    (ChildSizing::Fixed, "Fixed"),
    (ChildSizing::Hug, "Hug"),
    (ChildSizing::Fill, "Fill"),
];

fn combo<T: PartialEq + Copy>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)]) -> bool {
    let before = *value;
    let shown = options
        .iter()
        .find(|(v, _)| *v == *value)
        .map_or("", |o| o.1);
    egui::ComboBox::from_id_salt(id)
        .width(96.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (v, l) in options {
                ui.selectable_value(value, *v, *l);
            }
        });
    *value != before
}

/// Frames (and components) can hold an auto layout.
pub(super) fn can_auto_layout(n: &Node) -> bool {
    matches!(
        n.kind,
        NodeKind::Frame | NodeKind::Component(_) | NodeKind::Instance(_)
    )
}

impl Inspector {
    pub(super) fn auto_layout_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        doc: &Document,
        id: NodeId,
        n: &Node,
        actions: &mut Vec<Action>,
    ) {
        let current = n
            .ext
            .as_ref()
            .and_then(|e| e.auto_layout.as_deref())
            .copied();
        let mut al = current;
        let mut key = None;
        let mut toggle = false;
        section(
            ui,
            t,
            "Auto layout",
            |ui| {
                let (glyph, tip) = if current.is_some() {
                    (icons::MINUS, "Remove auto layout")
                } else {
                    (icons::PLUS, "Add auto layout  Shift+A")
                };
                if icon_button_sized(ui, t, glyph, tip, false, 22.0, 13.0).clicked() {
                    toggle = true;
                }
            },
            |ui| {
                let Some(a) = al.as_mut() else {
                    ui.label(
                        RichText::new("Children keep their positions")
                            .size(11.5)
                            .color(t.text_muted),
                    );
                    return;
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (d, glyph, tip) in [
                        (
                            LayoutDirection::Horizontal,
                            icons::ARROW_RIGHT,
                            "Horizontal",
                        ),
                        (LayoutDirection::Vertical, icons::ARROW_DOWN, "Vertical"),
                        (LayoutDirection::Wrap, icons::WRAP_TEXT, "Wrap"),
                    ] {
                        if icon_button_sized(ui, t, glyph, tip, a.direction == d, 28.0, 14.0)
                            .clicked()
                            && a.direction != d
                        {
                            a.direction = d;
                            key = Some(self.once());
                        }
                    }
                });
                let u = self.unit;
                egui::Grid::new("auto_layout")
                    .num_columns(4)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        field_label(ui, t, "Gap");
                        let mut gap = f64::from(a.gap);
                        if let Some(k) =
                            self.edited(&ui.add(length(&mut gap, u).range(0.0..=10000.0)))
                        {
                            a.gap = gap as f32;
                            if a.direction != LayoutDirection::Wrap {
                                a.cross_gap = a.gap;
                            }
                            key = Some(k);
                        }
                        if a.direction == LayoutDirection::Wrap {
                            field_label(ui, t, "Rows");
                            let mut g = f64::from(a.cross_gap);
                            if let Some(k) =
                                self.edited(&ui.add(length(&mut g, u).range(0.0..=10000.0)))
                            {
                                a.cross_gap = g as f32;
                                key = Some(k);
                            }
                        }
                        ui.end_row();
                        // Padding: horizontal (left = right) and vertical (top = bottom).
                        let [pt, _, _, pl] = a.padding;
                        field_label(ui, t, "Pad H");
                        let mut ph = f64::from(pl);
                        if let Some(k) =
                            self.edited(&ui.add(length(&mut ph, u).range(0.0..=10000.0)))
                        {
                            a.padding[1] = ph as f32;
                            a.padding[3] = ph as f32;
                            key = Some(k);
                        }
                        field_label(ui, t, "Pad V");
                        let mut pv = f64::from(pt);
                        if let Some(k) =
                            self.edited(&ui.add(length(&mut pv, u).range(0.0..=10000.0)))
                        {
                            a.padding[0] = pv as f32;
                            a.padding[2] = pv as f32;
                            key = Some(k);
                        }
                        ui.end_row();
                    });
                egui::Grid::new("auto_layout_align")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        field_label(ui, t, "Align");
                        if combo(ui, "al_main", &mut a.main_align, &MAIN) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                        field_label(ui, t, "Cross");
                        if combo(ui, "al_cross", &mut a.cross_align, &CROSS) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                        // Width/height map to primary/counter sizing by direction.
                        let horizontal = a.direction != LayoutDirection::Vertical;
                        let sizing = [
                            (ContainerSizing::Fixed, "Fixed"),
                            (ContainerSizing::Hug, "Hug"),
                        ];
                        let (w, h) = if horizontal {
                            (&mut a.primary_sizing, &mut a.counter_sizing)
                        } else {
                            (&mut a.counter_sizing, &mut a.primary_sizing)
                        };
                        field_label(ui, t, "Width");
                        if combo(ui, "al_w", w, &sizing) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                        field_label(ui, t, "Height");
                        if combo(ui, "al_h", h, &sizing) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                    });
            },
        );
        if toggle {
            al = match current {
                Some(_) => None,
                None => Some(ude_editor::layout::default_auto_layout(doc, id)),
            };
            key = Some(self.once());
        }
        if let Some(merge_key) = key {
            let mut ext = n.ext.clone().unwrap_or_default();
            ext.auto_layout = al.map(Box::new);
            actions.push(Action::Set {
                id,
                value: Prop::Ext(Some(ext)),
                merge_key,
            });
        }
    }

    /// Resizing of a layer inside an auto layout frame.
    pub(super) fn layout_child_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        id: NodeId,
        n: &Node,
        actions: &mut Vec<Action>,
    ) {
        let mut child = n
            .ext
            .as_ref()
            .map(|e| e.layout_child.clone())
            .unwrap_or_default();
        let mut key = None;
        section(
            ui,
            t,
            "Resizing",
            |_| {},
            |ui| {
                egui::Grid::new("layout_child")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        field_label(ui, t, "Width");
                        if combo(ui, "lc_w", &mut child.h_sizing, &CHILD) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                        field_label(ui, t, "Height");
                        if combo(ui, "lc_h", &mut child.v_sizing, &CHILD) {
                            key = Some(self.once());
                        }
                        ui.end_row();
                    });
                if ui
                    .checkbox(&mut child.absolute, "Ignore auto layout")
                    .changed()
                {
                    key = Some(self.once());
                }
            },
        );
        if let Some(merge_key) = key {
            let mut ext = n.ext.clone().unwrap_or_default();
            ext.layout_child = child;
            actions.push(Action::Set {
                id,
                value: Prop::Ext(Some(ext)),
                merge_key,
            });
        }
    }
}

/// Whether `id`'s parent lays it out.
pub(super) fn in_auto_layout(doc: &Document, n: &Node) -> bool {
    n.parent
        .and_then(|p| doc.node(p))
        .is_some_and(|p| p.ext.as_ref().is_some_and(|e| e.auto_layout.is_some()))
}
