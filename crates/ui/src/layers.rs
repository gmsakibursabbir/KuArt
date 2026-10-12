//! Pages list and the virtualized layers tree: only visible rows are laid out, so 100k layers
//! cost the same per frame as 40 (docs/memory-strategy.md §2.4). Rows show a type icon,
//! show/hide and lock toggles, and rename on double-click.

use std::collections::HashSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Ui, vec2};
use ude_core::{Document, NodeFlags, NodeId, NodeKind, Prop};
use ude_editor::Selection;

use crate::theme::Tokens;
use crate::widgets::{icon_button_sized, icon_font};
use crate::{Action, icons};

pub const ROW_HEIGHT: f32 = 28.0;
const INDENT: f32 = 14.0;

#[derive(Default)]
pub struct LayersPanel {
    expanded: HashSet<NodeId>,
    /// Flattened visible tree: (node, depth). Rebuilt when the document or expansion changes.
    rows: Vec<(NodeId, u16)>,
    built_for: Option<(u64, usize, Option<NodeId>)>,
    /// Layer or page being renamed, with the edited text.
    renaming: Option<(NodeId, String)>,
}

/// Lucide icon for a layer kind.
pub fn kind_icon(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Frame => icons::FRAME,
        NodeKind::Group => icons::GROUP,
        NodeKind::Rectangle => icons::SQUARE,
        NodeKind::Ellipse(_) => icons::CIRCLE,
        NodeKind::Polygon { .. } => icons::TRIANGLE,
        NodeKind::Star { .. } => icons::STAR,
        NodeKind::Line => icons::SLASH,
        NodeKind::Vector(_) | NodeKind::BooleanOp(_) => icons::PEN_TOOL,
        NodeKind::Text(_) => icons::TYPE,
        NodeKind::Raster { .. } => icons::IMAGE,
        NodeKind::Component(_) | NodeKind::ComponentSet => icons::COMPONENT,
        NodeKind::Instance(_) => icons::DIAMOND,
        NodeKind::Page => icons::FILE,
        _ => icons::LAYERS,
    }
}

pub fn kind_name(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Frame => "Frame",
        NodeKind::Group => "Group",
        NodeKind::Rectangle => "Rectangle",
        NodeKind::Ellipse(_) => "Ellipse",
        NodeKind::Polygon { .. } => "Polygon",
        NodeKind::Star { .. } => "Star",
        NodeKind::Line => "Line",
        NodeKind::Text(_) => "Text",
        NodeKind::Vector(_) | NodeKind::BooleanOp(_) => "Vector",
        NodeKind::Raster { .. } => "Image",
        NodeKind::Instance(_) => "Instance",
        NodeKind::Component(_) | NodeKind::ComponentSet => "Component",
        NodeKind::Page => "Page",
        _ => "Layer",
    }
}

impl LayersPanel {
    pub fn rows(&self) -> &[(NodeId, u16)] {
        &self.rows
    }

    pub fn toggle(&mut self, id: NodeId) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.built_for = None;
    }

    /// Expands every ancestor of `id` so it becomes a visible row.
    pub fn reveal(&mut self, doc: &Document, id: NodeId) {
        let before = self.expanded.len();
        self.expanded.extend(doc.ancestors(id));
        if self.expanded.len() != before {
            self.built_for = None;
        }
    }

    pub fn rebuild(&mut self, doc: &Document, page: NodeId) {
        let key = (doc.revision(), self.expanded.len(), Some(page));
        if self.built_for == Some(key) {
            return;
        }
        self.rows.clear();
        // Top of the stack first, like every design tool: reverse paint order.
        let mut stack: Vec<(NodeId, u16)> = doc.children(page).iter().map(|&c| (c, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                stack.extend(doc.children(id).iter().map(|&c| (c, depth + 1)));
            }
        }
        self.built_for = Some(key);
    }

    /// Pages list (click to switch, double-click to rename, + to add).
    pub fn pages_ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        page: NodeId,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        for &p in doc.children(doc.root()) {
            let Some(n) = doc.node(p) else { continue };
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            let current = p == page;
            if current {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(6), t.bg_raised);
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(6), t.bg_hover);
            }
            let name = doc.strings.resolve(n.name);
            if self.rename_field(ui, rect.shrink2(vec2(8.0, 2.0)), p, actions) {
                continue;
            }
            ui.painter().text(
                Pos2::new(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                if name.is_empty() { "Page" } else { name },
                FontId::proportional(12.5),
                if current {
                    t.text_primary
                } else {
                    t.text_secondary
                },
            );
            if current {
                ui.painter().text(
                    Pos2::new(rect.right() - 12.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    icons::CHECK,
                    icon_font(13.0),
                    t.accent,
                );
            }
            if resp.double_clicked() {
                self.renaming = Some((p, name.to_owned()));
            } else if resp.clicked() && !current {
                actions.push(Action::SelectPage(p));
            }
        }
    }

    /// Inline rename editor; returns true when `id` is being renamed (and drew the field).
    fn rename_field(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        id: NodeId,
        actions: &mut Vec<Action>,
    ) -> bool {
        let Some((rid, text)) = &mut self.renaming else {
            return false;
        };
        if *rid != id {
            return false;
        }
        let resp = ui.put(
            rect,
            egui::TextEdit::singleline(text).font(FontId::proportional(12.5)),
        );
        resp.request_focus();
        let done = resp.lost_focus();
        if done {
            let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if !cancelled {
                actions.push(Action::Rename(id, text.trim().to_owned()));
            }
            self.renaming = None;
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        page: NodeId,
        selection: &Selection,
        hover: Option<NodeId>,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        self.rebuild(doc, page);
        let shift = ui.input(|i| i.modifiers.shift);
        let mut toggle = None;
        let mut rename = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            ROW_HEIGHT,
            self.rows.len(),
            |ui, range| {
                let rows: Vec<(NodeId, u16)> = self.rows[range].to_vec();
                for (id, depth) in rows {
                    let Some(node) = doc.node(id) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    let selected = selection.contains(id);
                    let row_hover = resp.hovered() || hover == Some(id);
                    let p = ui.painter().clone();
                    if selected {
                        p.rect_filled(rect, CornerRadius::same(5), t.accent_soft);
                    } else if row_hover {
                        p.rect_filled(rect, CornerRadius::same(5), t.bg_hover);
                    }
                    let visible = node.flags.contains(NodeFlags::VISIBLE);
                    let locked = node.flags.contains(NodeFlags::LOCKED);
                    let x = rect.left() + 6.0 + f32::from(depth) * INDENT;
                    let has_children = !node.children.is_empty();
                    if has_children {
                        let open = self.expanded.contains(&id);
                        p.text(
                            Pos2::new(x + 6.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            if open {
                                icons::CHEVRON_DOWN
                            } else {
                                icons::CHEVRON_RIGHT
                            },
                            icon_font(12.0),
                            t.text_muted,
                        );
                    }
                    let component = matches!(
                        node.kind,
                        NodeKind::Component(_) | NodeKind::ComponentSet | NodeKind::Instance(_)
                    );
                    let faded = !visible;
                    let base = if component {
                        t.accent_component
                    } else {
                        t.text_primary
                    };
                    let color = if faded { t.text_muted } else { base };
                    p.text(
                        Pos2::new(x + 22.0, rect.center().y),
                        Align2::CENTER_CENTER,
                        kind_icon(&node.kind),
                        icon_font(13.0),
                        if component {
                            t.accent_component
                        } else {
                            t.text_muted
                        },
                    );
                    let name = doc.strings.resolve(node.name);
                    let label = if name.is_empty() {
                        kind_name(&node.kind)
                    } else {
                        name
                    };
                    let text_rect = Rect::from_min_max(
                        Pos2::new(x + 32.0, rect.top() + 3.0),
                        Pos2::new(rect.right() - 54.0, rect.bottom() - 3.0),
                    );
                    if !self.rename_field(ui, text_rect, id, actions) {
                        p.with_clip_rect(text_rect).text(
                            Pos2::new(text_rect.left(), rect.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(12.5),
                            color,
                        );
                    }

                    // Lock and visibility toggles: always shown when active, otherwise on hover.
                    let mut flags = node.flags;
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(
                        Rect::from_min_max(Pos2::new(rect.right() - 52.0, rect.top()), rect.max),
                    ));
                    child.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if locked || row_hover {
                            let glyph = if locked {
                                icons::LOCK
                            } else {
                                icons::LOCK_OPEN
                            };
                            if icon_button_sized(
                                ui,
                                t,
                                glyph,
                                if locked { "Unlock" } else { "Lock" },
                                false,
                                22.0,
                                12.0,
                            )
                            .clicked()
                            {
                                flags.set(NodeFlags::LOCKED, !locked);
                            }
                        } else {
                            ui.add_space(22.0);
                        }
                        if !visible || row_hover {
                            let glyph = if visible { icons::EYE } else { icons::EYE_OFF };
                            if icon_button_sized(
                                ui,
                                t,
                                glyph,
                                if visible { "Hide" } else { "Show" },
                                false,
                                22.0,
                                12.0,
                            )
                            .clicked()
                            {
                                flags.set(NodeFlags::VISIBLE, !visible);
                            }
                        }
                    });
                    if flags != node.flags {
                        actions.push(Action::Set {
                            id,
                            value: Prop::Flags(flags),
                            merge_key: 0,
                        });
                        continue;
                    }

                    if resp.hovered() {
                        actions.push(Action::Hover(Some(id)));
                    }
                    if resp.double_clicked() {
                        rename = Some((id, label.to_owned()));
                    } else if resp.clicked() {
                        let on_chevron = resp
                            .interact_pointer_pos()
                            .is_some_and(|pos| pos.x < x + 14.0);
                        if has_children && on_chevron {
                            toggle = Some(id);
                        } else if shift {
                            actions.push(Action::Toggle(id));
                        } else {
                            actions.push(Action::Select(vec![id]));
                        }
                    }
                }
            },
        );
        if let Some(id) = toggle {
            self.toggle(id);
        }
        if rename.is_some() {
            self.renaming = rename;
        }
    }
}
