//! Drawing tools and text edits, kept out of main.rs so the event loop stays a dispatcher.

use ude_core::typography::{TextData, TextRun, TextSizing};
use ude_core::{NodeId, NodeKind, Op, Prop, Size, Transaction};
use ude_editor::{Create, Tool};

use crate::{App, Drag};

impl App {
    pub(crate) fn begin_create(&mut self) {
        if self.tool == Tool::Move || self.index.is_none() {
            return;
        }
        let key = self.next_merge_key();
        let start = self.doc_point(self.cursor);
        let Some(index) = self.index.as_ref() else {
            return;
        };
        match Create::begin(
            &mut self.doc,
            &mut self.history,
            index,
            self.tool,
            start,
            key,
        ) {
            Ok(c) => {
                if self.tool == Tool::Text {
                    // Text is sized by its content, not by the drag.
                    self.autosize_text(c.id, key);
                }
                self.selection.set([c.id]);
                self.drag = Some(Drag::Create(c));
            }
            Err(e) => eprintln!("create failed: {e}"),
        }
        self.redraw();
    }

    pub(crate) fn create_drag_to(&mut self) {
        let pointer = self.doc_point(self.cursor);
        let mods = self.gesture_modifiers();
        if let Some(Drag::Create(c)) = &self.drag
            && self.tool != Tool::Text
            && let Err(e) = c.update(&mut self.doc, &mut self.history, pointer, mods)
        {
            eprintln!("create failed: {e}");
        }
        self.redraw();
    }

    /// Ends creation: default size for a click, then back to the Move tool.
    pub(crate) fn finish_create(&mut self) {
        let pointer = self.doc_point(self.cursor);
        let new_text = match &self.drag {
            Some(Drag::Create(c)) if self.tool == Tool::Text => Some(c.id),
            _ => None,
        };
        if let Some(Drag::Create(c)) = &self.drag
            && let Err(e) = c.finish(&mut self.doc, &mut self.history, pointer)
        {
            eprintln!("create failed: {e}");
        }
        self.tool = Tool::Move;
        // A new text layer is typed into straight away.
        if let Some(id) = new_text {
            self.drag = None;
            self.begin_text_edit(id, true);
        }
    }

    /// Applies a text edit from the inspector as one undoable step per edit session.
    pub(crate) fn set_text(
        &mut self,
        id: NodeId,
        content: Option<String>,
        family: Option<String>,
        size: Option<f32>,
        weight: Option<u16>,
        merge_key: u64,
    ) {
        let Some(NodeKind::Text(old)) = self.doc.node(id).map(|n| &n.kind) else {
            return;
        };
        let mut text: TextData = (**old).clone();
        if let Some(content) = content {
            // Rich runs cannot be mapped through a free-form edit: keep the first style for all.
            let len = u32::try_from(content.len()).unwrap_or(u32::MAX);
            text.content = content.into();
            text.runs = Box::new([TextRun {
                range: 0..len,
                style: 0,
            }]);
        }
        if family.is_some() || size.is_some() || weight.is_some() {
            let family = family.map(|f| self.doc.strings.intern(&f));
            let mut styles = text.styles.to_vec();
            if let Some(s) = styles.first_mut() {
                if let Some(f) = family {
                    s.family = f;
                }
                if let Some(v) = size {
                    s.size = v;
                }
                if let Some(w) = weight {
                    s.weight = w;
                }
            }
            text.styles = styles.into();
        }
        let mut tx = Transaction::new("Edit text").with_merge_key(merge_key);
        match tx.apply(
            &mut self.doc,
            Op::Set {
                id,
                value: Prop::Kind(NodeKind::Text(Box::new(text))),
            },
        ) {
            Ok(()) => self.history.commit(&mut self.doc, tx),
            Err(e) => return eprintln!("text edit failed: {e}"),
        }
        self.autosize_text(id, merge_key);
    }

    /// Fits an auto-sized text box to its content (same undo step as the edit that caused it).
    pub(crate) fn autosize_text(&mut self, id: NodeId, merge_key: u64) {
        let Some(n) = self.doc.node(id) else { return };
        let NodeKind::Text(text) = &n.kind else {
            return;
        };
        let size = match text.sizing {
            TextSizing::AutoWidth => {
                let (w, h) = self
                    .caches
                    .text
                    .measure(text, &self.doc.strings, f32::INFINITY);
                Size::new(w.ceil(), h.ceil())
            }
            TextSizing::AutoHeight => {
                let (_, h) = self.caches.text.measure(text, &self.doc.strings, n.size.w);
                Size::new(n.size.w, h.ceil())
            }
            TextSizing::Fixed | TextSizing::Truncate => return,
        };
        if size == n.size {
            return;
        }
        let mut tx = Transaction::new("Fit text").with_merge_key(merge_key);
        if tx
            .apply(
                &mut self.doc,
                Op::Set {
                    id,
                    value: Prop::Size(size),
                },
            )
            .is_ok()
        {
            self.history.commit(&mut self.doc, tx);
        }
    }
}

/// Single-key tool shortcuts (`Tool::shortcut`).
pub(crate) fn tool_for_key(code: winit::keyboard::KeyCode) -> Option<Tool> {
    use winit::keyboard::KeyCode as K;
    let c = match code {
        K::KeyV => 'V',
        K::KeyF => 'F',
        K::KeyR => 'R',
        K::KeyO => 'O',
        K::KeyP => 'P',
        K::KeyS => 'S',
        K::KeyL => 'L',
        K::KeyT => 'T',
        _ => return None,
    };
    Tool::ALL.into_iter().find(|t| t.shortcut() == Some(c))
}
