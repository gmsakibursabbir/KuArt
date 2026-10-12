//! Editing text directly on the canvas: caret, selection, typing (including IME), deletion,
//! word/line navigation, clipboard and mouse selection. Each editing session is one undo step.

use ude_core::NodeKind;
use ude_core::typography::TextData;
use ude_renderer::vello::kurbo::{Affine, Point};
use ude_renderer::{CaretMove, TextSelection};
use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use crate::App;

pub(crate) struct Editing {
    pub id: ude_core::NodeId,
    pub sel: TextSelection,
    /// Merge key: the whole session is one undo step.
    pub key: u64,
    pub dragging: bool,
}

impl App {
    fn edited_text(&self, id: ude_core::NodeId) -> Option<(TextData, (f32, f32))> {
        let n = self.doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else {
            return None;
        };
        Some(((**t).clone(), (n.size.w, n.size.h)))
    }

    /// Device point → the text layer's box coordinates.
    fn text_local(&self, id: ude_core::NodeId, device: Point) -> (f32, f32) {
        let world = ude_renderer::convert::affine(&self.doc.world_transform(id));
        let view = self.camera.transform(self.scale_factor());
        let p = (view * world).inverse() * device;
        (p.x as f32, p.y as f32)
    }

    pub(crate) fn begin_text_edit(&mut self, id: ude_core::NodeId, select_all: bool) {
        let Some((text, size)) = self.edited_text(id) else {
            return;
        };
        let len = text.content.len();
        let sel = if select_all {
            self.caches
                .text
                .select_range(&text, &self.doc.strings, size, 0, len)
        } else {
            let at = self.text_local(id, self.cursor);
            self.caches
                .text
                .select_at(&text, &self.doc.strings, size, at)
        };
        let key = self.next_merge_key();
        self.selection.set([id]);
        self.editing = Some(crate::text_edit::Editing {
            id,
            sel,
            key,
            dragging: false,
        });
        if let Some(w) = &self.window {
            w.set_ime_allowed(true);
        }
        self.redraw();
    }

    pub(crate) fn end_text_edit(&mut self) {
        if self.editing.take().is_some() {
            if let Some(w) = &self.window {
                w.set_ime_allowed(false);
            }
            self.redraw();
        }
    }

    /// Replaces the selection with `s` (one undo step per session).
    pub(crate) fn text_insert(&mut self, s: &str) {
        let Some(e) = &self.editing else { return };
        let (id, range, key) = (e.id, e.sel.range(), e.key);
        let Some((text, _)) = self.edited_text(id) else {
            return self.end_text_edit();
        };
        let mut content = text.content.to_string();
        let range = range.start.min(content.len())..range.end.min(content.len());
        content.replace_range(range.clone(), s);
        let caret = range.start + s.len();
        self.set_text(id, Some(content), None, None, None, key);
        self.reselect(caret, caret);
    }

    fn reselect(&mut self, anchor: usize, focus: usize) {
        let Some(e) = &self.editing else { return };
        let id = e.id;
        let Some((text, size)) = self.edited_text(id) else {
            return;
        };
        let sel = self
            .caches
            .text
            .select_range(&text, &self.doc.strings, size, anchor, focus);
        if let Some(e) = &mut self.editing {
            e.sel = sel;
        }
        self.redraw();
    }

    fn move_caret(&mut self, how: CaretMove, extend: bool) {
        let Some(e) = &self.editing else { return };
        let (id, sel) = (e.id, e.sel);
        let Some((text, size)) = self.edited_text(id) else {
            return;
        };
        let sel = self
            .caches
            .text
            .move_caret(&text, &self.doc.strings, size, sel, how, extend);
        if let Some(e) = &mut self.editing {
            e.sel = sel;
        }
        self.redraw();
    }

    /// Deletes the selection, or one character before/after the caret.
    fn delete(&mut self, forward: bool) {
        let Some(e) = &self.editing else { return };
        if e.sel.is_caret() {
            self.move_caret(
                if forward {
                    CaretMove::Right
                } else {
                    CaretMove::Left
                },
                true,
            );
        }
        let empty = self.editing.as_ref().is_some_and(|e| e.sel.is_caret());
        if !empty {
            self.text_insert("");
        }
    }

    fn selected_text(&self) -> Option<String> {
        let e = self.editing.as_ref()?;
        let (text, _) = self.edited_text(e.id)?;
        text.content.get(e.sel.range()).map(str::to_owned)
    }

    /// Keyboard input while editing; true when handled.
    pub(crate) fn text_key(&mut self, event: &KeyEvent) -> bool {
        if self.editing.is_none() || event.state != winit::event::ElementState::Pressed {
            return self.editing.is_some();
        }
        let shift = self.modifiers.shift_key();
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.end_text_edit(),
            Key::Named(NamedKey::Enter) => self.text_insert("\n"),
            Key::Named(NamedKey::Backspace) => self.delete(false),
            Key::Named(NamedKey::Delete) => self.delete(true),
            Key::Named(NamedKey::ArrowLeft) => self.move_caret(
                if ctrl {
                    CaretMove::WordLeft
                } else {
                    CaretMove::Left
                },
                shift,
            ),
            Key::Named(NamedKey::ArrowRight) => self.move_caret(
                if ctrl {
                    CaretMove::WordRight
                } else {
                    CaretMove::Right
                },
                shift,
            ),
            Key::Named(NamedKey::ArrowUp) => self.move_caret(CaretMove::Up, shift),
            Key::Named(NamedKey::ArrowDown) => self.move_caret(CaretMove::Down, shift),
            Key::Named(NamedKey::Home) => self.move_caret(CaretMove::LineStart, shift),
            Key::Named(NamedKey::End) => self.move_caret(CaretMove::LineEnd, shift),
            Key::Character(c) if ctrl => match c.to_ascii_lowercase().as_str() {
                "a" => {
                    let len = self
                        .editing
                        .as_ref()
                        .and_then(|e| self.edited_text(e.id))
                        .map_or(0, |(t, _)| t.content.len());
                    self.reselect(0, len);
                }
                "c" | "x" => {
                    if let Some(s) = self.selected_text().filter(|s| !s.is_empty()) {
                        self.egui.copy_text(s);
                        if c.eq_ignore_ascii_case("x") {
                            self.text_insert("");
                        }
                    }
                }
                "v" => {
                    let pasted = self.egui_state.as_mut().and_then(|s| s.clipboard_text());
                    if let Some(s) = pasted {
                        self.text_insert(&s.replace("\r\n", "\n"));
                    }
                }
                "z" | "y" => {
                    // Undo/redo apply to the document; leave editing first.
                    self.end_text_edit();
                    return false;
                }
                _ => {}
            },
            _ => {
                if let Some(t) = event
                    .text
                    .as_ref()
                    .filter(|t| !ctrl && !t.chars().any(char::is_control))
                {
                    let t = t.to_string();
                    self.text_insert(&t);
                }
            }
        }
        true
    }

    /// Mouse press while editing: inside the box moves the caret (Shift extends); outside ends
    /// editing. Returns true when the press was used.
    pub(crate) fn text_press(&mut self) -> bool {
        let Some(e) = &self.editing else { return false };
        let id = e.id;
        let Some((text, size)) = self.edited_text(id) else {
            self.end_text_edit();
            return false;
        };
        let at = self.text_local(id, self.cursor);
        let pad = 4.0;
        let inside = at.0 >= -pad
            && at.1 >= -pad
            && at.0 <= size.0.max(1.0) + pad
            && at.1 <= size.1.max(1.0) + pad;
        if !inside {
            self.end_text_edit();
            return false;
        }
        let sel = if self.modifiers.shift_key() {
            let old = self.editing.as_ref().map(|e| e.sel).unwrap_or_else(|| {
                self.caches
                    .text
                    .select_at(&text, &self.doc.strings, size, at)
            });
            self.caches
                .text
                .extend_to(&text, &self.doc.strings, size, old, at)
        } else {
            self.caches
                .text
                .select_at(&text, &self.doc.strings, size, at)
        };
        if let Some(e) = &mut self.editing {
            e.sel = sel;
            e.dragging = true;
        }
        self.redraw();
        true
    }

    /// Pointer moved while editing: extends a drag selection. Returns true when used.
    pub(crate) fn text_drag(&mut self) -> bool {
        let Some(e) = &self.editing else { return false };
        if !e.dragging {
            return false;
        }
        let (id, sel) = (e.id, e.sel);
        let Some((text, size)) = self.edited_text(id) else {
            return false;
        };
        let at = self.text_local(id, self.cursor);
        let sel = self
            .caches
            .text
            .extend_to(&text, &self.doc.strings, size, sel, at);
        if let Some(e) = &mut self.editing {
            e.sel = sel;
        }
        self.redraw();
        true
    }

    pub(crate) fn text_release(&mut self) {
        if let Some(e) = &mut self.editing {
            e.dragging = false;
        }
    }

    /// Caret segment and selection quads in device space, for the overlay; also places the
    /// IME candidate window at the caret.
    pub(crate) fn text_overlay(
        &mut self,
        view: Affine,
    ) -> (Option<(Point, Point)>, Vec<[Point; 4]>) {
        let Some(e) = &self.editing else {
            return (None, Vec::new());
        };
        let (id, sel) = (e.id, e.sel);
        let Some((text, size)) = self.edited_text(id) else {
            return (None, Vec::new());
        };
        let (caret, rects) =
            self.caches
                .text
                .selection_geometry(&text, &self.doc.strings, size, sel);
        let m = view * ude_renderer::convert::affine(&self.doc.world_transform(id));
        let pt = |x: f32, y: f32| m * Point::new(f64::from(x), f64::from(y));
        let quads = rects
            .iter()
            .map(|r| {
                [
                    pt(r[0], r[1]),
                    pt(r[2], r[1]),
                    pt(r[2], r[3]),
                    pt(r[0], r[3]),
                ]
            })
            .collect();
        let caret = caret.map(|(x, y0, y1)| (pt(x, y0), pt(x, y1)));
        if let (Some((_, bottom)), Some(w)) = (caret, &self.window) {
            w.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(bottom.x, bottom.y),
                winit::dpi::PhysicalSize::new(1.0, 1.0),
            );
        }
        (caret, quads)
    }
}
