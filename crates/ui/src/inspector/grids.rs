//! Layout grid section (frames): columns, rows or a square grid, each with count, gutter,
//! margin (or cell size), show/hide via Shift+G on the canvas, and remove.

use egui::{DragValue, RichText, Ui};
use ude_core::layout::{GridKind, LayoutGrid};
use ude_core::{Node, NodeId, Prop};

use super::Inspector;
use crate::theme::Tokens;
use crate::widgets::{icon_button_sized, length, section};
use crate::{Action, icons};

const KINDS: [(GridKind, &str); 3] = [
    (GridKind::Columns, "Columns"),
    (GridKind::Rows, "Rows"),
    (GridKind::Grid, "Grid"),
];

impl Inspector {
    pub(super) fn grids_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        id: NodeId,
        n: &Node,
        actions: &mut Vec<Action>,
    ) {
        let mut grids: Vec<LayoutGrid> =
            n.ext.as_ref().map(|e| e.grids.to_vec()).unwrap_or_default();
        let mut key = None;
        let mut add = false;
        section(
            ui,
            t,
            "Layout grid",
            |ui| {
                if icon_button_sized(
                    ui,
                    t,
                    icons::PLUS,
                    "Add layout grid (Shift+G shows or hides)",
                    false,
                    22.0,
                    13.0,
                )
                .clicked()
                {
                    add = true;
                }
            },
            |ui| {
                if grids.is_empty() {
                    ui.label(
                        RichText::new("No layout grid")
                            .size(11.5)
                            .color(t.text_muted),
                    );
                }
                let mut remove = None;
                let u = self.unit;
                for (i, g) in grids.iter_mut().enumerate() {
                    ui.push_id(("grid", i), |ui| {
                        ui.horizontal(|ui| {
                            let before = g.kind;
                            egui::ComboBox::from_id_salt("kind")
                                .width(80.0)
                                .selected_text(
                                    KINDS.iter().find(|k| k.0 == g.kind).map_or("", |k| k.1),
                                )
                                .show_ui(ui, |ui| {
                                    for (k, l) in KINDS {
                                        ui.selectable_value(&mut g.kind, k, l);
                                    }
                                });
                            if g.kind != before {
                                if g.kind == GridKind::Grid && g.size.is_none() {
                                    g.size = Some(10.0);
                                }
                                key = Some(self.once());
                            }
                            if g.kind != GridKind::Grid {
                                let mut c = u32::from(g.count);
                                let r = ui
                                    .add(DragValue::new(&mut c).range(1..=48))
                                    .on_hover_text("Count");
                                if let Some(k) = self.edited(&r) {
                                    g.count = c as u16;
                                    key = Some(k);
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icon_button_sized(
                                        ui,
                                        t,
                                        icons::X,
                                        "Remove layout grid",
                                        false,
                                        22.0,
                                        12.0,
                                    )
                                    .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            if g.kind == GridKind::Grid {
                                let mut s = f64::from(g.size.unwrap_or(10.0));
                                let r = ui
                                    .add(length(&mut s, u).range(1.0..=1000.0))
                                    .on_hover_text("Cell size");
                                if let Some(k) = self.edited(&r) {
                                    g.size = Some(s as f32);
                                    key = Some(k);
                                }
                                return;
                            }
                            for (label, v) in [("Gutter", &mut g.gutter), ("Margin", &mut g.margin)]
                            {
                                let mut f = f64::from(*v);
                                let r = ui
                                    .add(length(&mut f, u).range(0.0..=10000.0))
                                    .on_hover_text(label);
                                if let Some(k) = self.edited(&r) {
                                    *v = f as f32;
                                    key = Some(k);
                                }
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    grids.remove(i);
                    key = Some(self.once());
                }
            },
        );
        if add {
            grids.push(LayoutGrid {
                kind: GridKind::Columns,
                count: 12,
                gutter: 20.0,
                margin: 40.0,
                size: None,
            });
            key = Some(self.once());
        }
        if let Some(merge_key) = key {
            let mut ext = n.ext.clone().unwrap_or_default();
            ext.grids = grids.into();
            actions.push(Action::Set {
                id,
                value: Prop::Ext(Some(ext)),
                merge_key,
            });
        }
    }
}
