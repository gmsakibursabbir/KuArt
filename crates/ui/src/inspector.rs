//! Inspector for the selected layer: position, size, rotation, radius, opacity.
//! Each drag or text edit of a field is one undo step (shared merge key per session).

use egui::{DragValue, Grid, Response, TextEdit, Ui};
use ude_core::typography::TextData;
use ude_core::{Affine, Document, NodeId, NodeKind, Prop, Size};
use ude_editor::Selection;

use crate::Action;
use crate::theme::Tokens;

/// Inspector merge keys live in their own range so they never merge with canvas gestures.
const KEY_SPACE: u64 = 1 << 63;

#[derive(Default)]
pub struct Inspector {
    session: u64,
}

impl Inspector {
    /// Starts a new undo session when a field starts being dragged or edited;
    /// returns the session key when the value changed this frame.
    fn edited(&mut self, r: &Response) -> Option<u64> {
        if r.drag_started() || r.gained_focus() {
            self.session += 1;
        }
        r.changed().then_some(KEY_SPACE | self.session)
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        selection: &Selection,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        let id = match selection.ids() {
            [] => return drop(ui.colored_label(t.text_muted, "Nothing selected")),
            [id] => *id,
            many => {
                return drop(
                    ui.colored_label(t.text_secondary, format!("{} layers selected", many.len())),
                );
            }
        };
        let Some(n) = doc.node(id) else { return };
        ui.colored_label(t.text_primary, doc.strings.resolve(n.name));
        ui.add_space(8.0);

        // Decompose the local transform into position, rotation and scale (skew is not shown).
        let [a, b, c, d, e, f] = n.transform.0.map(f64::from);
        let sx = a.hypot(b);
        let sy = if sx > 0.0 { (a * d - b * c) / sx } else { d };
        let (mut x, mut y, mut rot) = (e, f, b.atan2(a).to_degrees());
        let (mut w, mut h) = (f64::from(n.size.w), f64::from(n.size.h));
        let mut radius = f64::from(n.corner_radius);
        let mut opacity = f64::from(n.opacity) * 100.0;
        let mut set = |key: Option<u64>, value: Prop| {
            if let Some(merge_key) = key {
                actions.push(Action::Set {
                    id,
                    value,
                    merge_key,
                });
            }
        };
        let transform = |x: f64, y: f64, rot: f64| {
            Affine::translate(x as f32, y as f32)
                * Affine::rotate(rot.to_radians() as f32)
                * Affine::scale(sx as f32, sy as f32)
        };

        Grid::new("inspector")
            .num_columns(4)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.colored_label(t.text_muted, "X");
                let kx = self.edited(&ui.add(DragValue::new(&mut x).max_decimals(2)));
                ui.colored_label(t.text_muted, "Y");
                let ky = self.edited(&ui.add(DragValue::new(&mut y).max_decimals(2)));
                ui.end_row();
                set(kx.or(ky), Prop::Transform(transform(x, y, rot)));

                ui.colored_label(t.text_muted, "W");
                let kw = self.edited(
                    &ui.add(
                        DragValue::new(&mut w)
                            .range(0.01..=f64::MAX)
                            .max_decimals(2),
                    ),
                );
                ui.colored_label(t.text_muted, "H");
                let kh = self.edited(
                    &ui.add(
                        DragValue::new(&mut h)
                            .range(0.01..=f64::MAX)
                            .max_decimals(2),
                    ),
                );
                ui.end_row();
                set(kw.or(kh), Prop::Size(Size::new(w as f32, h as f32)));

                ui.colored_label(t.text_muted, "Rotation");
                let kr = self.edited(
                    &ui.add(
                        DragValue::new(&mut rot)
                            .speed(0.5)
                            .suffix("°")
                            .max_decimals(1),
                    ),
                );
                ui.colored_label(t.text_muted, "Radius");
                let kc = self.edited(
                    &ui.add(
                        DragValue::new(&mut radius)
                            .speed(0.25)
                            .range(0.0..=f64::MAX)
                            .max_decimals(1),
                    ),
                );
                ui.end_row();
                set(kr, Prop::Transform(transform(x, y, rot)));
                set(kc, Prop::CornerRadius(radius as f32));

                ui.colored_label(t.text_muted, "Opacity");
                let ko = self.edited(
                    &ui.add(
                        DragValue::new(&mut opacity)
                            .speed(0.5)
                            .range(0.0..=100.0)
                            .suffix("%")
                            .max_decimals(0),
                    ),
                );
                ui.end_row();
                set(ko, Prop::Opacity((opacity / 100.0) as f32));
            });

        if let NodeKind::Text(text) = &n.kind {
            self.text_section(ui, doc, id, text, t, actions);
        }
    }

    /// Text content and the first run's font. Editing content collapses rich runs to one style.
    fn text_section(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        id: NodeId,
        text: &TextData,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        let Some(style) = text.styles.first() else {
            return;
        };
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("TEXT")
                .small()
                .strong()
                .color(t.text_muted),
        );
        ui.add_space(4.0);
        let mut content = text.content.to_string();
        let kc = self.edited(
            &ui.add(
                TextEdit::multiline(&mut content)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            ),
        );
        let mut family = doc.strings.resolve(style.family).to_owned();
        let (mut size, mut weight) = (f64::from(style.size), f64::from(style.weight));
        let (kf, ks, kw) = Grid::new("text_style")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.colored_label(t.text_muted, "Font");
                let kf = self.edited(&ui.add(TextEdit::singleline(&mut family)));
                ui.end_row();
                ui.colored_label(t.text_muted, "Size");
                let ks = self.edited(
                    &ui.add(
                        DragValue::new(&mut size)
                            .speed(0.5)
                            .range(1.0..=1000.0)
                            .max_decimals(1),
                    ),
                );
                ui.end_row();
                ui.colored_label(t.text_muted, "Weight");
                let kw = self.edited(
                    &ui.add(
                        DragValue::new(&mut weight)
                            .speed(4.0)
                            .range(100.0..=900.0)
                            .max_decimals(0),
                    ),
                );
                ui.end_row();
                (kf, ks, kw)
            })
            .inner;
        if let Some(merge_key) = kc.or(kf).or(ks).or(kw) {
            actions.push(Action::SetText {
                id,
                content: kc.map(|_| content),
                family: kf.map(|_| family),
                size: ks.map(|_| size as f32),
                weight: kw.map(|_| weight.round() as u16),
                merge_key,
            });
        }
    }
}
