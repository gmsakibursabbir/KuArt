//! Design tab: properties of the selected layer in Figma-style sections (layout, layer, fill,
//! stroke, text, export). Each drag or edit of a field is one undo step.

use egui::{Color32, DragValue, Grid, Response, RichText, TextEdit, Ui};
use ude_core::styles::{BlendMode, Color, Paint, PaintKind, Paints, Stroke, StrokeAlign};
use ude_core::typography::TextData;
use ude_core::{Affine, Document, NodeId, NodeKind, Prop, Size};
use ude_editor::Selection;

use crate::layers::{kind_icon, kind_name};
use crate::theme::Tokens;
use crate::widgets::{field_label, icon, icon_button_sized, primary_button, section};
use crate::{Action, ExportFormat, ExportOptions, icons};

/// Inspector merge keys live in their own range so they never merge with canvas gestures.
mod effects;
mod gradient;

const KEY_SPACE: u64 = 1 << 63;

const WEIGHTS: [(u16, &str); 9] = [
    (100, "Thin"),
    (200, "Extra Light"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "Semi Bold"),
    (700, "Bold"),
    (800, "Extra Bold"),
    (900, "Black"),
];

const BLENDS: [(BlendMode, &str); 17] = [
    (BlendMode::Normal, "Normal"),
    (BlendMode::PassThrough, "Pass through"),
    (BlendMode::Darken, "Darken"),
    (BlendMode::Multiply, "Multiply"),
    (BlendMode::ColorBurn, "Color burn"),
    (BlendMode::Lighten, "Lighten"),
    (BlendMode::Screen, "Screen"),
    (BlendMode::ColorDodge, "Color dodge"),
    (BlendMode::Overlay, "Overlay"),
    (BlendMode::SoftLight, "Soft light"),
    (BlendMode::HardLight, "Hard light"),
    (BlendMode::Difference, "Difference"),
    (BlendMode::Exclusion, "Exclusion"),
    (BlendMode::Hue, "Hue"),
    (BlendMode::Saturation, "Saturation"),
    (BlendMode::Color, "Color"),
    (BlendMode::Luminosity, "Luminosity"),
];

#[derive(Default)]
pub struct Inspector {
    session: u64,
    /// Selected gradient stop in the Fill section.
    stop: usize,
    pub export: ExportOptions,
}

pub fn to_egui(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

pub fn from_egui(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::from_rgba8(r, g, b, a)
}

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
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

    /// A one-shot edit (button, combo): its own undo step.
    fn once(&mut self) -> u64 {
        self.session += 1;
        KEY_SPACE | self.session
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        selection: &Selection,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        if !selection.is_empty() {
            align_bar(ui, t, selection.ids().len(), actions);
        }
        let id = match selection.ids() {
            [] => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(icon(icons::MOUSE_POINTER_2, 22.0, t.text_muted));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Select a layer to edit it").color(t.text_muted));
                });
                return;
            }
            [id] => *id,
            many => {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("{} layers selected", many.len()))
                        .color(t.text_secondary),
                );
                ui.add_space(8.0);
                self.export_section(ui, t, actions, "selection");
                return;
            }
        };
        let Some(n) = doc.node(id) else { return };
        let name = doc.strings.resolve(n.name);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(icon(kind_icon(&n.kind), 15.0, t.text_secondary));
            ui.label(
                RichText::new(if name.is_empty() {
                    kind_name(&n.kind)
                } else {
                    name
                })
                .strong()
                .color(t.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(kind_name(&n.kind))
                        .size(11.0)
                        .color(t.text_muted),
                );
            });
        });

        // Layout: decompose the local transform into position, rotation and scale (no skew).
        let [a, b, c, d, e, f] = n.transform.0.map(f64::from);
        let sx = a.hypot(b);
        let sy = if sx > 0.0 { (a * d - b * c) / sx } else { d };
        let (mut x, mut y, mut rot) = (e, f, b.atan2(a).to_degrees());
        let (mut w, mut h) = (f64::from(n.size.w), f64::from(n.size.h));
        let mut radius = f64::from(n.corner_radius);
        let transform = |x: f64, y: f64, rot: f64| {
            Affine::translate(x as f32, y as f32)
                * Affine::rotate(rot.to_radians() as f32)
                * Affine::scale(sx as f32, sy as f32)
        };
        let set = |actions: &mut Vec<Action>, key: Option<u64>, value: Prop| {
            if let Some(merge_key) = key {
                actions.push(Action::Set {
                    id,
                    value,
                    merge_key,
                });
            }
        };
        section(
            ui,
            t,
            "Layout",
            |_| {},
            |ui| {
                Grid::new("layout")
                    .num_columns(4)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        field_label(ui, t, "X");
                        let kx = self.edited(&ui.add(DragValue::new(&mut x).max_decimals(2)));
                        field_label(ui, t, "Y");
                        let ky = self.edited(&ui.add(DragValue::new(&mut y).max_decimals(2)));
                        ui.end_row();
                        set(actions, kx.or(ky), Prop::Transform(transform(x, y, rot)));
                        field_label(ui, t, "W");
                        let kw = self.edited(
                            &ui.add(
                                DragValue::new(&mut w)
                                    .range(0.01..=f64::MAX)
                                    .max_decimals(2),
                            ),
                        );
                        field_label(ui, t, "H");
                        let kh = self.edited(
                            &ui.add(
                                DragValue::new(&mut h)
                                    .range(0.01..=f64::MAX)
                                    .max_decimals(2),
                            ),
                        );
                        ui.end_row();
                        set(
                            actions,
                            kw.or(kh),
                            Prop::Size(Size::new(w as f32, h as f32)),
                        );
                        ui.label(icon(icons::ROTATE_CW, 12.0, t.text_muted))
                            .on_hover_text("Rotation");
                        let kr = self.edited(
                            &ui.add(
                                DragValue::new(&mut rot)
                                    .speed(0.5)
                                    .suffix("°")
                                    .max_decimals(1),
                            ),
                        );
                        ui.label(icon(icons::SQUARE_ROUND_CORNER, 12.0, t.text_muted))
                            .on_hover_text("Corner radius");
                        let kc = self.edited(
                            &ui.add(
                                DragValue::new(&mut radius)
                                    .speed(0.25)
                                    .range(0.0..=f64::MAX)
                                    .max_decimals(1),
                            ),
                        );
                        ui.end_row();
                        set(actions, kr, Prop::Transform(transform(x, y, rot)));
                        set(actions, kc, Prop::CornerRadius(radius as f32));
                    });
            },
        );

        // Layer: opacity and blend mode.
        let mut opacity = f64::from(n.opacity) * 100.0;
        section(
            ui,
            t,
            "Layer",
            |_| {},
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(icon(icons::BLEND, 12.0, t.text_muted));
                    let mut blend = n.blend;
                    egui::ComboBox::from_id_salt("blend")
                        .width(120.0)
                        .selected_text(
                            BLENDS
                                .iter()
                                .find(|(b, _)| *b == blend)
                                .map_or("Normal", |(_, l)| *l),
                        )
                        .show_ui(ui, |ui| {
                            for (b, l) in BLENDS {
                                ui.selectable_value(&mut blend, b, l);
                            }
                        });
                    if blend != n.blend {
                        let k = self.once();
                        set(actions, Some(k), Prop::Blend(blend));
                    }
                    ui.label(icon(icons::DROPLET, 12.0, t.text_muted))
                        .on_hover_text("Opacity");
                    let ko = self.edited(
                        &ui.add(
                            DragValue::new(&mut opacity)
                                .speed(0.5)
                                .range(0.0..=100.0)
                                .suffix("%")
                                .max_decimals(0),
                        ),
                    );
                    set(actions, ko, Prop::Opacity((opacity / 100.0) as f32));
                });
            },
        );

        if !matches!(n.kind, NodeKind::Text(_) | NodeKind::Group) {
            self.fill_section(ui, t, id, &n.fills, actions);
            self.stroke_section(ui, t, id, n, actions);
        }
        self.effects_section(ui, t, id, n, actions);
        if let NodeKind::Text(text) = &n.kind {
            self.text_section(ui, doc, id, text, t, actions);
        }
        self.export_section(
            ui,
            t,
            actions,
            if name.is_empty() {
                kind_name(&n.kind)
            } else {
                name
            },
        );
    }

    fn fill_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        id: NodeId,
        fills: &Paints,
        actions: &mut Vec<Action>,
    ) {
        let mut paints: Vec<Paint> = fills.as_slice().to_vec();
        let mut changed: Option<u64> = None;
        let mut add = false;
        section(
            ui,
            t,
            "Fill",
            |ui| {
                if icon_button_sized(ui, t, icons::PLUS, "Add fill", false, 22.0, 13.0).clicked() {
                    add = true;
                }
            },
            |ui| {
                if paints.is_empty() {
                    ui.label(RichText::new("No fill").size(11.5).color(t.text_muted));
                }
                let mut remove = None;
                for (i, p) in paints.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if !matches!(p.kind, PaintKind::Image { .. })
                            && let Some(k) = self.type_menu(ui, t, p, i)
                        {
                            changed = Some(k);
                        }
                        match &mut p.kind {
                            PaintKind::Solid(c) => {
                                let mut c32 = to_egui(*c);
                                let r = ui.color_edit_button_srgba(&mut c32);
                                if r.changed() {
                                    *c = from_egui(c32);
                                    changed = Some(changed.unwrap_or_else(|| self.once_key(&r)));
                                }
                                let mut hex = c.to_hex().trim_start_matches('#').to_owned();
                                let hr = ui.add(
                                    TextEdit::singleline(&mut hex)
                                        .desired_width(72.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                if hr.lost_focus()
                                    && let Some(nc) = parse_hex(&hex)
                                    && nc != *c
                                {
                                    *c = nc;
                                    changed = Some(self.once());
                                }
                                let mut pct = f64::from(p.opacity) * 100.0;
                                let pr = ui.add(
                                    DragValue::new(&mut pct)
                                        .range(0.0..=100.0)
                                        .suffix("%")
                                        .max_decimals(0),
                                );
                                if let Some(k) = self.edited(&pr) {
                                    p.opacity = (pct / 100.0) as f32;
                                    changed = Some(k);
                                }
                                if icon_button_sized(
                                    ui,
                                    t,
                                    if p.visible {
                                        icons::EYE
                                    } else {
                                        icons::EYE_OFF
                                    },
                                    "Show/hide fill",
                                    false,
                                    22.0,
                                    12.0,
                                )
                                .clicked()
                                {
                                    p.visible = !p.visible;
                                    changed = Some(self.once());
                                }
                                if icon_button_sized(
                                    ui,
                                    t,
                                    icons::X,
                                    "Remove fill",
                                    false,
                                    22.0,
                                    12.0,
                                )
                                .clicked()
                                {
                                    remove = Some(i);
                                }
                            }
                            PaintKind::Gradient(g) => {
                                let name = format!("{:?} gradient", g.kind);
                                ui.label(RichText::new(name).color(t.text_secondary));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_button_sized(
                                            ui,
                                            t,
                                            icons::X,
                                            "Remove fill",
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
                            }
                            PaintKind::Image { .. } => {
                                ui.label(icon(icons::IMAGE, 13.0, t.text_secondary));
                                ui.label(RichText::new("Image").color(t.text_secondary));
                            }
                        }
                    });
                    if let PaintKind::Gradient(g) = &mut p.kind
                        && let Some(k) = self.gradient_ui(ui, t, std::sync::Arc::make_mut(g), i)
                    {
                        changed = Some(k);
                    }
                }
                if let Some(i) = remove {
                    paints.remove(i);
                    changed = Some(self.once());
                }
            },
        );
        if add {
            paints.push(Paint::solid(Color::from_rgba8(0xD9, 0xD9, 0xD9, 0xFF)));
            changed = Some(self.once());
        }
        if let Some(merge_key) = changed {
            let value = if paints.is_empty() {
                Paints::default()
            } else {
                Paints::new(paints)
            };
            actions.push(Action::Set {
                id,
                value: Prop::Fills(value),
                merge_key,
            });
        }
    }

    /// Colour pickers report changes every frame while open: one undo step per open picker.
    fn once_key(&mut self, r: &Response) -> u64 {
        if r.clicked() || r.gained_focus() {
            self.session += 1;
        }
        KEY_SPACE | self.session
    }

    fn stroke_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        id: NodeId,
        n: &ude_core::Node,
        actions: &mut Vec<Action>,
    ) {
        let current = n.ext.as_ref().and_then(|e| e.strokes.as_deref()).cloned();
        let mut stroke = current.clone();
        let mut key = None;
        let mut add = false;
        let has_stroke = stroke.is_some();
        section(
            ui,
            t,
            "Stroke",
            |ui| {
                if !has_stroke
                    && icon_button_sized(ui, t, icons::PLUS, "Add stroke", false, 22.0, 13.0)
                        .clicked()
                {
                    add = true;
                }
            },
            |ui| {
                let Some(s) = stroke.as_mut() else {
                    ui.label(RichText::new("No stroke").size(11.5).color(t.text_muted));
                    return;
                };
                ui.horizontal(|ui| {
                    if let Some(Paint {
                        kind: PaintKind::Solid(c),
                        ..
                    }) = s.paints.as_slice().first().cloned()
                    {
                        let mut c32 = to_egui(c);
                        let r = ui.color_edit_button_srgba(&mut c32);
                        if r.changed() {
                            s.paints = Paints::new(vec![Paint::solid(from_egui(c32))]);
                            key = Some(self.once_key(&r));
                        }
                    }
                    let mut width = f64::from(s.width);
                    let wr = ui.add(
                        DragValue::new(&mut width)
                            .speed(0.1)
                            .range(0.0..=1000.0)
                            .max_decimals(1),
                    );
                    if let Some(k) = self.edited(&wr) {
                        s.width = width as f32;
                        key = Some(k);
                    }
                    let mut align = s.align;
                    egui::ComboBox::from_id_salt("stroke_align")
                        .width(80.0)
                        .selected_text(match align {
                            StrokeAlign::Inside => "Inside",
                            StrokeAlign::Center => "Center",
                            StrokeAlign::Outside => "Outside",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut align, StrokeAlign::Inside, "Inside");
                            ui.selectable_value(&mut align, StrokeAlign::Center, "Center");
                            ui.selectable_value(&mut align, StrokeAlign::Outside, "Outside");
                        });
                    if align != s.align {
                        s.align = align;
                        key = Some(self.once());
                    }
                    if icon_button_sized(ui, t, icons::X, "Remove stroke", false, 22.0, 12.0)
                        .clicked()
                    {
                        key = Some(self.once());
                        s.width = -1.0; // marker: removed below
                    }
                });
            },
        );
        if add {
            stroke = Some(Stroke {
                paints: Paints::new(vec![Paint::solid(Color::BLACK)]),
                width: 1.0,
                miter_limit: 4.0,
                ..Default::default()
            });
            key = Some(self.once());
        }
        if let Some(merge_key) = key {
            let mut ext = n.ext.clone().unwrap_or_default();
            ext.strokes = stroke.filter(|s| s.width >= 0.0).map(Box::new);
            actions.push(Action::Set {
                id,
                value: Prop::Ext(Some(ext)),
                merge_key,
            });
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
        let mut content = text.content.to_string();
        let mut family = doc.strings.resolve(style.family).to_owned();
        let (mut size, mut weight) = (f64::from(style.size), style.weight);
        let (mut kc, mut kf, mut ks, mut kw) = (None, None, None, None);
        section(
            ui,
            t,
            "Text",
            |ui| {
                if icon_button_sized(ui, t, icons::COPY, "Copy text", false, 22.0, 12.0).clicked() {
                    ui.ctx().copy_text(text.content.to_string());
                }
            },
            |ui| {
                kc = self.edited(
                    &ui.add(
                        TextEdit::multiline(&mut content)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    ),
                );
                ui.add_space(4.0);
                Grid::new("text_style")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        field_label(ui, t, "Font");
                        kf = self.edited(
                            &ui.add(TextEdit::singleline(&mut family).desired_width(150.0)),
                        );
                        ui.end_row();
                        field_label(ui, t, "Weight");
                        let before = weight;
                        egui::ComboBox::from_id_salt("weight")
                            .width(150.0)
                            .selected_text(
                                WEIGHTS
                                    .iter()
                                    .find(|(w, _)| *w == weight)
                                    .map_or_else(|| weight.to_string(), |(_, l)| (*l).to_owned()),
                            )
                            .show_ui(ui, |ui| {
                                for (w, l) in WEIGHTS {
                                    ui.selectable_value(&mut weight, w, l);
                                }
                            });
                        if weight != before {
                            kw = Some(self.once());
                        }
                        ui.end_row();
                        field_label(ui, t, "Size");
                        ks = self.edited(
                            &ui.add(
                                DragValue::new(&mut size)
                                    .speed(0.5)
                                    .range(1.0..=1000.0)
                                    .max_decimals(1),
                            ),
                        );
                        ui.end_row();
                    });
            },
        );
        if let Some(merge_key) = kc.or(kf).or(ks).or(kw) {
            actions.push(Action::SetText {
                id,
                content: kc.map(|_| content),
                family: kf.map(|_| family),
                size: ks.map(|_| size as f32),
                weight: kw.map(|_| weight),
                merge_key,
            });
        }
    }

    fn export_section(&mut self, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>, name: &str) {
        let o = &mut self.export;
        section(
            ui,
            t,
            "Export",
            |_| {},
            |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("export_scale")
                        .width(56.0)
                        .selected_text(format!("{}x", o.scale))
                        .show_ui(ui, |ui| {
                            for s in [0.5, 1.0, 1.5, 2.0, 3.0, 4.0] {
                                ui.selectable_value(&mut o.scale, s, format!("{s}x"));
                            }
                        });
                    egui::ComboBox::from_id_salt("export_format")
                        .width(72.0)
                        .selected_text(o.format.label())
                        .show_ui(ui, |ui| {
                            for f in ExportFormat::ALL {
                                ui.selectable_value(&mut o.format, f, f.label());
                            }
                        });
                });
                if o.format.is_raster() {
                    ui.add_space(4.0);
                    ui.checkbox(&mut o.compress, "Compress (smaller file)").on_hover_text(
                    "PNG and WebP: reduce to an optimised 256-colour palette, like TinyPNG. \
                     JPEG: uses the quality below.",
                );
                    if o.format == ExportFormat::Jpeg || o.compress {
                        ui.horizontal(|ui| {
                            field_label(ui, t, "Quality");
                            ui.add(egui::Slider::new(&mut o.quality, 10..=100).suffix("%"));
                        });
                    }
                }
                ui.add_space(6.0);
                if primary_button(ui, t, Some(icons::DOWNLOAD), &format!("Export {name}")).clicked()
                {
                    actions.push(Action::ExportWith(*o));
                }
            },
        );
    }
}

/// Align (to selection, or to the parent frame for one layer) and distribute buttons.
fn align_bar(ui: &mut Ui, t: &Tokens, selected: usize, actions: &mut Vec<Action>) {
    let can_distribute = selected >= 3;
    use ude_editor::Align as A;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (how, glyph, tip) in [
            (A::Left, icons::ALIGN_START_VERTICAL, "Align left  Alt+A"),
            (
                A::HCenter,
                icons::ALIGN_CENTER_VERTICAL,
                "Align horizontal centers  Alt+H",
            ),
            (A::Right, icons::ALIGN_END_VERTICAL, "Align right  Alt+D"),
            (A::Top, icons::ALIGN_START_HORIZONTAL, "Align top  Alt+W"),
            (
                A::VCenter,
                icons::ALIGN_CENTER_HORIZONTAL,
                "Align vertical centers  Alt+V",
            ),
            (
                A::Bottom,
                icons::ALIGN_END_HORIZONTAL,
                "Align bottom  Alt+S",
            ),
        ] {
            if icon_button_sized(ui, t, glyph, tip, false, 28.0, 15.0).clicked() {
                actions.push(Action::Align(how));
            }
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(can_distribute, |ui| {
            for (how, glyph, tip) in [
                (
                    A::DistributeH,
                    icons::ALIGN_HORIZONTAL_SPACE_BETWEEN,
                    "Distribute horizontally  Alt+Shift+H",
                ),
                (
                    A::DistributeV,
                    icons::ALIGN_VERTICAL_SPACE_BETWEEN,
                    "Distribute vertically  Alt+Shift+V",
                ),
            ] {
                if icon_button_sized(ui, t, glyph, tip, false, 28.0, 15.0).clicked() {
                    actions.push(Action::Align(how));
                }
            }
        });
    });
    if selected >= 2 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (glyph, label, key, e) in crate::BOOLEANS {
                let tip = format!("{label}  {key}");
                if icon_button_sized(ui, t, glyph, &tip, false, 28.0, 15.0).clicked() {
                    actions.push(Action::Edit(e));
                }
            }
        });
    }
    let w = ui.available_width();
    let (line, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, t.border_subtle);
}
