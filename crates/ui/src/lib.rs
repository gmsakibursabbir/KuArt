//! Editor panels drawn with egui around the canvas: top bar, layers, inspector, status bar.
//! Panels read the document and return [`Action`]s; the app applies them through history.

mod inspector;
mod layers;
pub mod theme;

pub use egui;
pub use layers::{LayersPanel, ROW_HEIGHT};
pub use theme::{ThemeChoice, Tokens};

use egui::{Align, Layout, Panel, Rect, RichText, Ui};
use inspector::Inspector;
use ude_core::{Document, NodeId, Prop};
use ude_editor::{Selection, Tool};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(Vec<NodeId>),
    Toggle(NodeId),
    Hover(Option<NodeId>),
    Set {
        id: NodeId,
        value: Prop,
        merge_key: u64,
    },
    /// Text edits; the app interns the family and re-measures auto-sized boxes.
    SetText {
        id: NodeId,
        content: Option<String>,
        family: Option<String>,
        size: Option<f32>,
        weight: Option<u16>,
        merge_key: u64,
    },
    Tool(Tool),
    Open,
    Save,
    Export,
    Undo,
    Redo,
    ZoomToFit,
}

/// Read-only facts the bars display.
pub struct Status<'a> {
    pub title: &'a str,
    pub zoom: f64,
    pub renderer: &'a str,
    pub can_undo: bool,
    pub can_redo: bool,
    pub tool: Tool,
    /// Font families used by the document but not installed.
    pub missing_fonts: &'a [&'a str],
}

#[derive(Default)]
pub struct Panels {
    pub theme: ThemeChoice,
    pub layers: LayersPanel,
    inspector: Inspector,
}

impl Panels {
    /// Draws all panels into `root` and returns the actions plus the remaining canvas rect (points).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        root: &mut Ui,
        doc: &Document,
        page: NodeId,
        selection: &Selection,
        hover: Option<NodeId>,
        status: &Status<'_>,
        t: &Tokens,
    ) -> (Vec<Action>, Rect) {
        let mut actions = Vec::new();

        Panel::top("top_bar")
            .exact_size(40.0)
            .show_inside(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("◆ KuArt").strong().color(t.accent));
                    ui.separator();
                    ui.colored_label(t.text_secondary, status.title);
                    for (label, hint, action) in [
                        ("Open", "Open a design or image (Ctrl+O)", Action::Open),
                        ("Save", "Save (Ctrl+S)", Action::Save),
                        (
                            "Export",
                            "Export selection or page as PNG/SVG (Ctrl+Shift+E)",
                            Action::Export,
                        ),
                    ] {
                        if ui.button(label).on_hover_text(hint).clicked() {
                            actions.push(action);
                        }
                    }
                    ui.separator();
                    for tool in Tool::ALL {
                        let hint = format!("{} ({})", tool.label(), tool.shortcut());
                        if ui
                            .selectable_label(status.tool == tool, tool.label())
                            .on_hover_text(hint)
                            .clicked()
                        {
                            actions.push(Action::Tool(tool));
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        egui::ComboBox::from_id_salt("theme")
                            .selected_text(self.theme.label())
                            .show_ui(ui, |ui| {
                                for c in ThemeChoice::ALL {
                                    ui.selectable_value(&mut self.theme, c, c.label());
                                }
                            });
                        if ui
                            .add_enabled(status.can_redo, egui::Button::new("Redo"))
                            .clicked()
                        {
                            actions.push(Action::Redo);
                        }
                        if ui
                            .add_enabled(status.can_undo, egui::Button::new("Undo"))
                            .clicked()
                        {
                            actions.push(Action::Undo);
                        }
                    });
                });
            });

        Panel::bottom("status_bar")
            .exact_size(24.0)
            .show_inside(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui
                        .small_button(format!("{:.0}%", status.zoom * 100.0))
                        .on_hover_text("Zoom to fit (Shift+1)")
                        .clicked()
                    {
                        actions.push(Action::ZoomToFit);
                    }
                    ui.colored_label(t.text_muted, format!("{} layers", self.layers.rows().len()));
                    if !selection.is_empty() {
                        ui.colored_label(
                            t.text_muted,
                            format!("· {} selected", selection.ids().len()),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.colored_label(t.text_muted, status.renderer);
                        if !status.missing_fonts.is_empty() {
                            let list = status.missing_fonts.join(", ");
                            ui.colored_label(t.warning, format!("⚠ Not installed: {list}"))
                                .on_hover_text("Drawn with a fallback font. The original font name is kept when saving.");
                        }
                    });
                });
            });

        Panel::left("layers")
            .default_size(240.0)
            .size_range(160.0..=480.0)
            .show_inside(root, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("LAYERS").small().strong().color(t.text_muted));
                ui.add_space(4.0);
                self.layers
                    .ui(ui, doc, page, selection, hover, t, &mut actions);
            });

        Panel::right("inspector")
            .default_size(280.0)
            .size_range(220.0..=480.0)
            .show_inside(root, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("DESIGN").small().strong().color(t.text_muted));
                ui.add_space(4.0);
                self.inspector.ui(ui, doc, selection, t, &mut actions);
            });

        (actions, root.available_rect_before_wrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Node, NodeKind, Symbol};

    fn doc_with(n: usize) -> (Document, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        for _ in 0..n {
            doc.build_child(page, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
                .unwrap();
        }
        (doc, page)
    }

    /// Headless frame: panels lay out only visible rows whatever the layer count.
    fn frame(panels: &mut Panels, doc: &Document, page: NodeId) -> (Vec<Action>, Rect) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        let status = Status {
            title: "t",
            zoom: 1.0,
            renderer: "cpu",
            can_undo: false,
            can_redo: false,
            tool: ude_editor::Tool::Move,
            missing_fonts: &[],
        };
        let _ = ctx.run_ui(input, |ui| {
            out = Some(panels.ui(
                ui,
                doc,
                page,
                &Selection::default(),
                None,
                &status,
                &theme::DARK,
            ));
        });
        out.unwrap()
    }

    #[test]
    fn canvas_rect_excludes_panels() {
        let (doc, page) = doc_with(3);
        let (_, canvas) = frame(&mut Panels::default(), &doc, page);
        assert!(
            canvas.left() >= 160.0 && canvas.right() <= 1280.0 - 220.0,
            "{canvas:?}"
        );
        assert!(
            canvas.top() >= 40.0 && canvas.bottom() <= 800.0 - 24.0,
            "{canvas:?}"
        );
    }

    #[test]
    fn layers_flatten_respects_expansion() {
        let (mut doc, page) = doc_with(2);
        let first = doc.children(page)[0];
        doc.build_child(first, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        let mut l = LayersPanel::default();
        l.rebuild(&doc, page);
        assert_eq!(l.rows().len(), 2, "collapsed by default");
        l.toggle(first);
        l.rebuild(&doc, page);
        assert_eq!(l.rows().len(), 3);
        assert_eq!(l.rows()[1].1, 1, "child is indented");
    }
}
