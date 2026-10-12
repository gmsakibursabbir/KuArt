//! Fill type switching and gradient editing in the Fill section: a stops bar (drag a stop,
//! click the bar to add one), the selected stop's colour and position, and the angle.

use egui::{Color32, DragValue, Mesh, Pos2, Rect, Response, RichText, Sense, Ui, vec2};
use ude_core::Affine;
use ude_core::styles::{Color, Gradient, GradientKind, GradientStop, Paint, PaintKind};

use super::{Inspector, KEY_SPACE, from_egui, to_egui};
use crate::icons;
use crate::theme::Tokens;
use crate::widgets::{icon_button_sized, icon_font};

/// Fill types offered by the type menu (`None` = solid).
const TYPES: [(Option<GradientKind>, &str, &str); 5] = [
    (None, icons::SQUARE, "Solid"),
    (Some(GradientKind::Linear), icons::BLEND, "Linear"),
    (Some(GradientKind::Radial), icons::CIRCLE, "Radial"),
    (Some(GradientKind::Angular), icons::ROTATE_CW, "Angular"),
    (Some(GradientKind::Diamond), icons::DIAMOND, "Diamond"),
];

fn kind_of(p: &Paint) -> Option<GradientKind> {
    match &p.kind {
        PaintKind::Gradient(g) => Some(g.kind),
        _ => None,
    }
}

/// Converts a paint to another fill type, keeping its colours.
pub(super) fn set_type(p: &mut Paint, to: Option<GradientKind>) {
    match (&mut p.kind, to) {
        (PaintKind::Gradient(g), Some(k)) => std::sync::Arc::make_mut(g).kind = k,
        (PaintKind::Gradient(g), None) => {
            let c = g.stops.first().map_or(Color::BLACK, |s| s.color);
            p.kind = PaintKind::Solid(c);
        }
        (kind, Some(k)) => {
            let c = match kind {
                PaintKind::Solid(c) => *c,
                _ => Color::BLACK,
            };
            *kind = PaintKind::Gradient(std::sync::Arc::new(Gradient {
                kind: k,
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: c,
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color { a: 0.0, ..c },
                    },
                ]
                .into(),
                transform: Affine::IDENTITY,
            }));
        }
        (_, None) => {}
    }
}

/// Colour at `t` between the stops (stops sorted by offset).
fn color_at(stops: &[GradientStop], t: f32) -> Color {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Color::BLACK;
    };
    if t <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.offset {
            let f = if b.offset > a.offset {
                (t - a.offset) / (b.offset - a.offset)
            } else {
                0.0
            };
            let mix = |x: f32, y: f32| x + (y - x) * f;
            return Color {
                r: mix(a.color.r, b.color.r),
                g: mix(a.color.g, b.color.g),
                b: mix(a.color.b, b.color.b),
                a: mix(a.color.a, b.color.a),
            };
        }
    }
    last.color
}

/// Rotation of a gradient about the centre of its box, in degrees.
fn angle(t: &Affine) -> f32 {
    t.0[1].atan2(t.0[0]).to_degrees()
}

fn rotated(degrees: f32) -> Affine {
    let r = Affine::rotate(degrees.to_radians());
    let [a, b, c, d, _, _] = r.0;
    // T(0.5, 0.5) · R · T(-0.5, -0.5)
    Affine([a, b, c, d, 0.5 - 0.5 * a - 0.5 * c, 0.5 - 0.5 * b - 0.5 * d])
}

impl Inspector {
    /// Small menu button choosing the fill type; returns the undo key when it changed.
    pub(super) fn type_menu(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        p: &mut Paint,
        row: usize,
    ) -> Option<u64> {
        let current = kind_of(p);
        let glyph = TYPES
            .iter()
            .find(|(k, _, _)| *k == current)
            .map_or(icons::SQUARE, |e| e.1);
        let mut picked = None;
        ui.push_id(("fill_type", row), |ui| {
            ui.menu_button(
                RichText::new(glyph)
                    .font(icon_font(13.0))
                    .color(t.text_secondary),
                |ui| {
                    ui.set_width(130.0);
                    for (k, g, label) in TYPES {
                        let text = RichText::new(format!("{g}  {label}"));
                        if ui.selectable_label(current == k, text).clicked() {
                            picked = Some(k);
                            ui.close();
                        }
                    }
                },
            )
            .response
            .on_hover_text("Fill type");
        });
        let k = picked.filter(|k| *k != current)?;
        set_type(p, k);
        Some(self.once())
    }

    /// Gradient editor under a gradient fill row; returns the undo key when it changed.
    pub(super) fn gradient_ui(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        g: &mut Gradient,
        row: usize,
    ) -> Option<u64> {
        let mut key = None;
        // Stops keep their order while one is dragged (so the drag stays on it); sorted after.
        let mut stops: Vec<GradientStop> = g.stops.to_vec();
        let mut sorted = stops.clone();
        sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        let sel = self.stop.min(stops.len().saturating_sub(1));

        // The bar: checkerboard under the gradient so transparency shows.
        let w = ui.available_width();
        let (bar, bar_resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
        let painter = ui.painter_at(bar.expand(8.0));
        let cell = 5.0;
        for iy in 0..4 {
            for ix in 0..((w / cell) as usize + 1) {
                let light = (ix + iy) % 2 == 0;
                let r = Rect::from_min_size(
                    Pos2::new(bar.left() + ix as f32 * cell, bar.top() + iy as f32 * cell),
                    vec2(cell, cell),
                )
                .intersect(bar);
                painter.rect_filled(
                    r,
                    0.0,
                    if light {
                        Color32::from_gray(200)
                    } else {
                        Color32::from_gray(150)
                    },
                );
            }
        }
        let mut mesh = Mesh::default();
        let x_of = |o: f32| bar.left() + o.clamp(0.0, 1.0) * bar.width();
        let mut cols: Vec<(f32, Color32)> = vec![(bar.left(), to_egui(color_at(&sorted, 0.0)))];
        cols.extend(sorted.iter().map(|s| (x_of(s.offset), to_egui(s.color))));
        cols.push((bar.right(), to_egui(color_at(&sorted, 1.0))));
        for pair in cols.windows(2) {
            let ((x0, c0), (x1, c1)) = (pair[0], pair[1]);
            let i = mesh.vertices.len() as u32;
            for (x, y, c) in [
                (x0, bar.top(), c0),
                (x1, bar.top(), c1),
                (x1, bar.bottom(), c1),
                (x0, bar.bottom(), c0),
            ] {
                mesh.colored_vertex(Pos2::new(x, y), c);
            }
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        painter.add(mesh);
        painter.rect_stroke(
            bar,
            3.0,
            egui::Stroke::new(1.0_f32, t.border_strong),
            egui::StrokeKind::Inside,
        );

        // Stop handles under the bar.
        let (lane, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
        let mut selected = sel;
        let mut moved: Option<(usize, f32, Response)> = None;
        let (mut dragging, mut drag_ended) = (false, false);
        for (i, s) in stops.iter().enumerate() {
            let c = Pos2::new(x_of(s.offset), lane.center().y);
            let r = Rect::from_center_size(c, vec2(10.0, 12.0));
            let resp = ui.interact(r, ui.id().with(("stop", row, i)), Sense::click_and_drag());
            if resp.clicked() || resp.drag_started() {
                selected = i;
            }
            dragging |= resp.dragged();
            drag_ended |= resp.drag_stopped();
            if resp.dragged()
                && let Some(p) = resp.interact_pointer_pos()
            {
                moved = Some((
                    i,
                    ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0),
                    resp.clone(),
                ));
            }
            let border = if i == selected {
                t.accent
            } else {
                t.border_strong
            };
            ui.painter()
                .rect_filled(r, 2.0, to_egui(Color { a: 1.0, ..s.color }));
            ui.painter().rect_stroke(
                r,
                2.0,
                egui::Stroke::new(if i == selected { 2.0_f32 } else { 1.0 }, border),
                egui::StrokeKind::Outside,
            );
        }
        if let Some((i, offset, resp)) = moved {
            stops[i].offset = offset;
            if resp.drag_started() {
                self.session += 1;
            }
            key = Some(KEY_SPACE | self.session);
        }
        // Clicking the bar adds a stop with the colour already there.
        if bar_resp.clicked()
            && let Some(p) = bar_resp.interact_pointer_pos()
        {
            let offset = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            stops.push(GradientStop {
                offset,
                color: color_at(&sorted, offset),
            });
            selected = stops.len() - 1;
            key = Some(self.once());
        }

        // Selected stop: colour, position, remove.
        if let Some(s) = stops.get_mut(selected) {
            ui.horizontal(|ui| {
                let mut c32 = to_egui(s.color);
                let r = ui.color_edit_button_srgba(&mut c32);
                if r.changed() {
                    s.color = from_egui(c32);
                    key = Some(key.unwrap_or_else(|| self.once_key(&r)));
                }
                let mut pct = f64::from(s.offset) * 100.0;
                let pr = ui.add(
                    DragValue::new(&mut pct)
                        .range(0.0..=100.0)
                        .suffix("%")
                        .max_decimals(0),
                );
                if let Some(k) = self.edited(&pr) {
                    s.offset = (pct / 100.0) as f32;
                    key = Some(k);
                }
                ui.label(RichText::new("stop").size(11.0).color(t.text_muted));
                if matches!(g.kind, GradientKind::Linear | GradientKind::Angular) {
                    let mut deg = f64::from(angle(&g.transform));
                    let ar = ui
                        .add(
                            DragValue::new(&mut deg)
                                .range(-180.0..=180.0)
                                .suffix("°")
                                .max_decimals(0),
                        )
                        .on_hover_text("Angle");
                    if let Some(k) = self.edited(&ar) {
                        g.transform = rotated(deg as f32);
                        key = Some(k);
                    }
                }
            });
        }
        let removable = stops.len() > 2;
        if removable
            && ui
                .horizontal(|ui| {
                    icon_button_sized(ui, t, icons::TRASH_2, "Remove stop", false, 22.0, 12.0)
                        .clicked()
                })
                .inner
        {
            stops.remove(selected);
            selected = selected.saturating_sub(1);
            key = Some(self.once());
        }

        // Keep stops ordered (the renderer needs it) and the selection on the same stop.
        if drag_ended {
            key = Some(KEY_SPACE | self.session);
        }
        if dragging {
            self.stop = selected;
        } else {
            let chosen = stops.get(selected).copied();
            stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            self.stop = chosen
                .and_then(|c| stops.iter().position(|s| *s == c))
                .unwrap_or(0);
        }
        if key.is_some() {
            g.stops = stops.into();
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_to_gradient_and_back_keeps_the_colour() {
        let red = Color::from_rgba8(255, 0, 0, 255);
        let mut p = Paint::solid(red);
        set_type(&mut p, Some(GradientKind::Linear));
        let PaintKind::Gradient(g) = &p.kind else {
            panic!()
        };
        assert_eq!(g.stops.len(), 2);
        assert_eq!(color_at(&g.stops, 0.5).a, 0.5);
        set_type(&mut p, Some(GradientKind::Radial));
        set_type(&mut p, None);
        assert_eq!(p.kind, PaintKind::Solid(red));
    }

    #[test]
    fn angle_round_trips_about_the_centre() {
        let t = rotated(90.0);
        assert!((angle(&t) - 90.0).abs() < 1e-3);
        // The centre stays fixed.
        let c = t.apply(ude_core::Point::new(0.5, 0.5));
        assert!((c.x - 0.5).abs() < 1e-5 && (c.y - 0.5).abs() < 1e-5);
    }
}
