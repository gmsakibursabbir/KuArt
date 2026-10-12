//! Command palette (Ctrl+K): one search box for every command, tool and layer on the page.
//! Type to filter, ↑/↓ to choose, Enter to run, Esc to close.

use egui::{Align2, Key, RichText, TextEdit, Ui};
use ude_core::{Document, NodeId, NodeKind};
use ude_editor::Align as A;

use crate::layers::{kind_icon, kind_name};
use crate::theme::Tokens;
use crate::widgets::icon_font;
use crate::{Action, BOOLEANS, Edit, icons};

const MAX_RESULTS: usize = 40;

#[derive(Default)]
pub struct Palette {
    open: bool,
    query: String,
    selected: usize,
    focus: bool,
}

struct Entry {
    glyph: &'static str,
    label: String,
    detail: String,
    action: Action,
}

fn commands() -> Vec<Entry> {
    let c = |glyph, label: &str, key: &str, action| Entry {
        glyph,
        label: label.to_owned(),
        detail: key.to_owned(),
        action,
    };
    let mut v = vec![
        c(icons::FILE_PLUS, "New design", "Ctrl+N", Action::New),
        c(icons::FOLDER_OPEN, "Open…", "Ctrl+O", Action::Open),
        c(icons::FILE_INPUT, "Import…", "", Action::Import),
        c(icons::SAVE, "Save", "Ctrl+S", Action::Save),
        c(icons::SAVE_ALL, "Save as…", "Ctrl+Shift+S", Action::SaveAs),
        c(icons::DOWNLOAD, "Export…", "Ctrl+Shift+E", Action::Export),
        c(icons::FILE_PLUS_2, "Add page", "", Action::AddPage),
        c(icons::HOUSE, "Back to files", "", Action::Home),
        c(icons::UNDO_2, "Undo", "Ctrl+Z", Action::Undo),
        c(icons::REDO_2, "Redo", "Ctrl+Shift+Z", Action::Redo),
        c(icons::COPY, "Copy", "Ctrl+C", Action::Edit(Edit::Copy)),
        c(icons::SCISSORS, "Cut", "Ctrl+X", Action::Edit(Edit::Cut)),
        c(
            icons::CLIPBOARD_PASTE,
            "Paste",
            "Ctrl+V",
            Action::Edit(Edit::Paste),
        ),
        c(
            icons::COPY_PLUS,
            "Duplicate",
            "Ctrl+D",
            Action::Edit(Edit::Duplicate),
        ),
        c(icons::TRASH_2, "Delete", "Del", Action::Edit(Edit::Delete)),
        c(
            icons::SQUARE_DASHED_MOUSE_POINTER,
            "Select all",
            "Ctrl+A",
            Action::Edit(Edit::SelectAll),
        ),
        c(icons::GROUP, "Group", "Ctrl+G", Action::Edit(Edit::Group)),
        c(
            icons::UNGROUP,
            "Ungroup",
            "Ctrl+Shift+G",
            Action::Edit(Edit::Ungroup),
        ),
        c(
            icons::COMBINE,
            "Flatten",
            "Ctrl+E",
            Action::Edit(Edit::Flatten),
        ),
        c(
            icons::SCAN_EYE,
            "Use as mask",
            "Ctrl+Alt+M",
            Action::Edit(Edit::Mask),
        ),
        c(
            icons::BRING_TO_FRONT,
            "Bring forward",
            "Ctrl+]",
            Action::Edit(Edit::BringForward),
        ),
        c(
            icons::SEND_TO_BACK,
            "Send backward",
            "Ctrl+[",
            Action::Edit(Edit::SendBackward),
        ),
        c(
            icons::ALIGN_START_VERTICAL,
            "Align left",
            "Alt+A",
            Action::Align(A::Left),
        ),
        c(
            icons::ALIGN_CENTER_VERTICAL,
            "Align horizontal centers",
            "Alt+H",
            Action::Align(A::HCenter),
        ),
        c(
            icons::ALIGN_END_VERTICAL,
            "Align right",
            "Alt+D",
            Action::Align(A::Right),
        ),
        c(
            icons::ALIGN_START_HORIZONTAL,
            "Align top",
            "Alt+W",
            Action::Align(A::Top),
        ),
        c(
            icons::ALIGN_CENTER_HORIZONTAL,
            "Align vertical centers",
            "Alt+V",
            Action::Align(A::VCenter),
        ),
        c(
            icons::ALIGN_END_HORIZONTAL,
            "Align bottom",
            "Alt+S",
            Action::Align(A::Bottom),
        ),
        c(
            icons::ALIGN_HORIZONTAL_SPACE_BETWEEN,
            "Distribute horizontally",
            "Alt+Shift+H",
            Action::Align(A::DistributeH),
        ),
        c(
            icons::ALIGN_VERTICAL_SPACE_BETWEEN,
            "Distribute vertically",
            "Alt+Shift+V",
            Action::Align(A::DistributeV),
        ),
        c(icons::ZOOM_IN, "Zoom in", "Ctrl++", Action::ZoomIn),
        c(icons::ZOOM_OUT, "Zoom out", "Ctrl+-", Action::ZoomOut),
        c(icons::MAXIMIZE, "Zoom to fit", "Shift+1", Action::ZoomToFit),
        c(
            icons::ACTIVITY,
            "Diagnostics",
            "F12",
            Action::ToggleDiagnostics,
        ),
        c(
            icons::ROTATE_CW,
            "Check for updates",
            "",
            Action::CheckUpdates,
        ),
    ];
    for (glyph, label, key, e) in BOOLEANS {
        v.push(c(glyph, label, key, Action::Edit(e)));
    }
    for (tool, glyph) in crate::TOOL_ICONS {
        let key = tool.shortcut().map(String::from).unwrap_or_default();
        v.push(c(
            glyph,
            &format!("{} tool", tool.label()),
            &key,
            Action::Tool(tool),
        ));
    }
    v
}

/// All query words appear in the text (case-insensitive).
fn matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|w| text.contains(w))
}

fn layers(doc: &Document, page: NodeId, query: &str) -> Vec<Entry> {
    doc.descendants(page)
        .skip(1)
        .filter_map(|id| {
            let n = doc.node(id)?;
            let name = doc.strings.resolve(n.name);
            let label = if name.is_empty() {
                kind_name(&n.kind).to_owned()
            } else {
                name.to_owned()
            };
            let text = match &n.kind {
                NodeKind::Text(t) => format!("{label} {}", t.content),
                _ => label.clone(),
            };
            matches(&text, query).then(|| Entry {
                glyph: kind_icon(&n.kind),
                label,
                detail: kind_name(&n.kind).to_owned(),
                action: Action::Select(vec![id]),
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

impl Palette {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn ui(
        &mut self,
        root: &mut Ui,
        doc: &Document,
        page: NodeId,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        if !self.open {
            return;
        }
        let q = self.query.trim().to_owned();
        let mut results: Vec<Entry> = commands()
            .into_iter()
            .filter(|e| matches(&e.label, &q))
            .collect();
        // Layers only once something is typed (every layer would bury the commands).
        if !q.is_empty() {
            results.extend(layers(doc, page, &q));
        }
        results.truncate(MAX_RESULTS);
        let (up, down, enter, esc) = root.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        if down {
            self.selected = (self.selected + 1).min(results.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(results.len().saturating_sub(1));
        let mut run = None;
        if enter && let Some(e) = results.get(self.selected) {
            run = Some(e.action.clone());
        }

        let mut close = esc;
        let screen = root.ctx().content_rect();
        let width = 520.0_f32.min(screen.width() - 32.0);
        let area = egui::Area::new("command_palette".into())
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, [0.0, 72.0])
            .show(root.ctx(), |ui| {
                egui::Frame::new()
                    .fill(t.bg_raised)
                    .stroke(egui::Stroke::new(1.0_f32, t.border_strong))
                    .corner_radius(8.0)
                    .inner_margin(8.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(90),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icons::SEARCH)
                                    .font(icon_font(15.0))
                                    .color(t.text_muted),
                            );
                            let r = ui.add(
                                TextEdit::singleline(&mut self.query)
                                    .hint_text("Search commands and layers")
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY),
                            );
                            if self.focus {
                                r.request_focus();
                                self.focus = false;
                            }
                            if r.changed() {
                                self.selected = 0;
                            }
                        });
                        ui.separator();
                        if results.is_empty() {
                            ui.label(RichText::new("Nothing found").color(t.text_muted));
                        }
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for (i, e) in results.iter().enumerate() {
                                    let selected = i == self.selected;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 28.0),
                                        egui::Sense::click(),
                                    );
                                    if selected || resp.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            4.0,
                                            if selected { t.accent_soft } else { t.bg_hover },
                                        );
                                    }
                                    if selected && (up || down) {
                                        resp.scroll_to_me(None);
                                    }
                                    let p = ui.painter();
                                    let y = rect.center().y;
                                    p.text(
                                        egui::pos2(rect.left() + 14.0, y),
                                        Align2::CENTER_CENTER,
                                        e.glyph,
                                        icon_font(14.0),
                                        t.text_secondary,
                                    );
                                    p.text(
                                        egui::pos2(rect.left() + 32.0, y),
                                        Align2::LEFT_CENTER,
                                        &e.label,
                                        egui::FontId::proportional(13.0),
                                        t.text_primary,
                                    );
                                    p.text(
                                        egui::pos2(rect.right() - 8.0, y),
                                        Align2::RIGHT_CENTER,
                                        &e.detail,
                                        egui::FontId::proportional(11.0),
                                        t.text_muted,
                                    );
                                    if resp.clicked() {
                                        run = Some(e.action.clone());
                                    }
                                }
                            });
                    });
            });
        // Clicking outside closes it.
        if root.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() {
            close = true;
        }
        if let Some(a) = run {
            actions.push(a);
            close = true;
        }
        if close {
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match() {
        assert!(matches("Align horizontal centers", "align cent"));
        assert!(!matches("Align left", "align cent"));
        assert!(commands().iter().any(|e| matches(&e.label, "pen")));
    }
}
