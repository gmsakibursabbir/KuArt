//! Effects section: drop and inner shadows (offset, blur, spread, colour) and layer and
//! background blur (radius), each with show/hide and remove.

use egui::{RichText, Ui};
use ude_core::styles::{BlendMode, Color, Effect, Shadow};
use ude_core::{NodeId, Point, Prop};

use super::{Inspector, from_egui, to_egui};
use crate::theme::Tokens;
use crate::widgets::{icon_button_sized, section};
use crate::{Action, icons};

fn shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(0, 0, 0, 64),
        offset: Point::new(0.0, 4.0),
        blur: 8.0,
        spread: 0.0,
        blend: BlendMode::Normal,
        visible: true,
    }
}

/// What the add menu offers. Inner shadow and background blur (kept and editable when
/// imported) are not offered until the renderer draws them.
fn presets() -> [Effect; 2] {
    [
        Effect::DropShadow(shadow()),
        Effect::LayerBlur {
            radius: 8.0,
            visible: true,
        },
    ]
}

fn label(e: &Effect) -> &'static str {
    match e {
        Effect::DropShadow(_) => "Drop shadow",
        Effect::InnerShadow(_) => "Inner shadow",
        Effect::LayerBlur { .. } => "Layer blur",
        Effect::BackgroundBlur { .. } => "Background blur",
    }
}

fn visible(e: &mut Effect) -> &mut bool {
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => &mut s.visible,
        Effect::LayerBlur { visible, .. } | Effect::BackgroundBlur { visible, .. } => visible,
    }
}

impl Inspector {
    pub(super) fn effects_section(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        id: NodeId,
        n: &ude_core::Node,
        actions: &mut Vec<Action>,
    ) {
        let mut effects: Vec<Effect> = n
            .ext
            .as_ref()
            .map(|e| e.effects.to_vec())
            .unwrap_or_default();
        let mut key = None;
        let mut add = None;
        section(
            ui,
            t,
            "Effects",
            |ui| {
                ui.menu_button(
                    RichText::new(icons::PLUS).font(crate::widgets::icon_font(13.0)),
                    |ui| {
                        ui.set_width(150.0);
                        for e in presets() {
                            if ui.button(label(&e)).clicked() {
                                add = Some(e);
                                ui.close();
                            }
                        }
                    },
                )
                .response
                .on_hover_text("Add effect");
            },
            |ui| {
                if effects.is_empty() {
                    ui.label(RichText::new("No effects").size(11.5).color(t.text_muted));
                }
                let mut remove = None;
                for (i, e) in effects.iter_mut().enumerate() {
                    ui.push_id(("effect", i), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label(e)).color(t.text_secondary));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icon_button_sized(
                                        ui,
                                        t,
                                        icons::X,
                                        "Remove effect",
                                        false,
                                        22.0,
                                        12.0,
                                    )
                                    .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                    let v = visible(e);
                                    let glyph = if *v { icons::EYE } else { icons::EYE_OFF };
                                    if icon_button_sized(
                                        ui,
                                        t,
                                        glyph,
                                        "Show/hide effect",
                                        false,
                                        22.0,
                                        12.0,
                                    )
                                    .clicked()
                                    {
                                        *v = !*v;
                                        key = Some(self.once());
                                    }
                                },
                            );
                        });
                        match e {
                            Effect::DropShadow(s) | Effect::InnerShadow(s) => {
                                ui.horizontal(|ui| {
                                    let mut c32 = to_egui(s.color);
                                    let r = ui.color_edit_button_srgba(&mut c32);
                                    if r.changed() {
                                        s.color = from_egui(c32);
                                        key = Some(key.unwrap_or_else(|| self.once_key(&r)));
                                    }
                                    for (name, v, min) in [
                                        ("X", &mut s.offset.x, -1000.0),
                                        ("Y", &mut s.offset.y, -1000.0),
                                        ("Blur", &mut s.blur, 0.0),
                                        ("Spread", &mut s.spread, -1000.0),
                                    ] {
                                        let mut f = f64::from(*v);
                                        let r = ui
                                            .add(
                                                crate::widgets::length(&mut f, self.unit)
                                                    .range(min..=1000.0),
                                            )
                                            .on_hover_text(name);
                                        if let Some(k) = self.edited(&r) {
                                            *v = f as f32;
                                            key = Some(k);
                                        }
                                    }
                                });
                            }
                            Effect::LayerBlur { radius, .. }
                            | Effect::BackgroundBlur { radius, .. } => {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Radius").size(11.0).color(t.text_muted),
                                    );
                                    let mut f = f64::from(*radius);
                                    let r = ui.add(
                                        crate::widgets::length(&mut f, self.unit)
                                            .range(0.0..=250.0),
                                    );
                                    if let Some(k) = self.edited(&r) {
                                        *radius = f as f32;
                                        key = Some(k);
                                    }
                                });
                            }
                        }
                    });
                }
                if let Some(i) = remove {
                    effects.remove(i);
                    key = Some(self.once());
                }
            },
        );
        if let Some(e) = add {
            effects.push(e);
            key = Some(self.once());
        }
        if let Some(merge_key) = key {
            let mut ext = n.ext.clone().unwrap_or_default();
            ext.effects = effects.into();
            actions.push(Action::Set {
                id,
                value: Prop::Ext(Some(ext)),
                merge_key,
            });
        }
    }
}
