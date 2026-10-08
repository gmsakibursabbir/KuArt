//! Virtualized layers panel: only visible rows are laid out, so 100k layers cost
//! the same per frame as 40 (docs/memory-strategy.md §2.4).

use std::collections::HashSet;

use egui::{Pos2, Sense, Shape, Stroke, Ui, vec2};
use ude_core::{Document, NodeId, NodeKind};
use ude_editor::Selection;

use crate::Action;
use crate::theme::Tokens;

pub const ROW_HEIGHT: f32 = 28.0;
const INDENT: f32 = 14.0;

#[derive(Default)]
pub struct LayersPanel {
    expanded: HashSet<NodeId>,
    /// Flattened visible tree: (node, depth). Rebuilt when the document or expansion changes.
    rows: Vec<(NodeId, u16)>,
    built_for: Option<(u64, usize)>,
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
        let key = (doc.revision(), self.expanded.len());
        if self.built_for == Some(key) {
            return;
        }
        self.rows.clear();
        // Stack of (node, depth); children pushed in reverse so they pop in tree order.
        let mut stack: Vec<(NodeId, u16)> =
            doc.children(page).iter().rev().map(|&c| (c, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                stack.extend(doc.children(id).iter().rev().map(|&c| (c, depth + 1)));
            }
        }
        self.built_for = Some(key);
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
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            ROW_HEIGHT,
            self.rows.len(),
            |ui, range| {
                for &(id, depth) in &self.rows[range] {
                    let Some(node) = doc.node(id) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    let selected = selection.contains(id);
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(rect, 0.0, t.accent.gamma_multiply(0.25));
                    } else if resp.hovered() || hover == Some(id) {
                        p.rect_filled(rect, 0.0, t.bg_hover);
                    }
                    let x = rect.left() + 8.0 + f32::from(depth) * INDENT;
                    let has_children = !node.children.is_empty();
                    if has_children {
                        triangle(
                            p,
                            Pos2::new(x + 5.0, rect.center().y),
                            self.expanded.contains(&id),
                            t.text_muted,
                        );
                    }
                    let color = match node.kind {
                        NodeKind::Component(_) | NodeKind::ComponentSet | NodeKind::Instance(_) => {
                            t.accent_component
                        }
                        _ if node.is_visible() => t.text_primary,
                        _ => t.text_muted,
                    };
                    let name = doc.strings.resolve(node.name);
                    let label = if name.is_empty() {
                        kind_name(&node.kind)
                    } else {
                        name
                    };
                    p.text(
                        Pos2::new(x + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(12.0),
                        color,
                    );

                    if resp.hovered() {
                        actions.push(Action::Hover(Some(id)));
                    }
                    if resp.clicked() {
                        let on_triangle = resp
                            .interact_pointer_pos()
                            .is_some_and(|pos| pos.x < x + 14.0);
                        if has_children && on_triangle {
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
    }
}

fn triangle(p: &egui::Painter, c: Pos2, open: bool, color: egui::Color32) {
    let pts = if open {
        vec![
            c + vec2(-4.0, -2.0),
            c + vec2(4.0, -2.0),
            c + vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + vec2(-2.0, -4.0),
            c + vec2(3.0, 0.0),
            c + vec2(-2.0, 4.0),
        ]
    };
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

fn kind_name(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Frame => "Frame",
        NodeKind::Group => "Group",
        NodeKind::Rectangle => "Rectangle",
        NodeKind::Ellipse(_) => "Ellipse",
        NodeKind::Text(_) => "Text",
        NodeKind::Vector(_) => "Vector",
        NodeKind::Raster { .. } => "Image",
        NodeKind::Instance(_) => "Instance",
        NodeKind::Component(_) => "Component",
        _ => "Layer",
    }
}
