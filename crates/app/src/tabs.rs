//! Several open documents as tabs. The active document lives in the `App` fields as before;
//! the others are parked here and swapped in when their tab is chosen.

use std::path::PathBuf;

use ude_core::{Document, History, NodeId};
use ude_editor::Selection;
use ude_spatial::SpatialIndex;

use crate::App;
use crate::camera::Camera;

/// A parked (inactive) document and its editing state.
pub(crate) struct DocTab {
    doc: Document,
    title: String,
    file: Option<PathBuf>,
    page: Option<NodeId>,
    index: Option<SpatialIndex>,
    selection: Selection,
    history: History,
    saved_revision: u64,
    camera: Camera,
    fitted: bool,
    layers: ude_ui::LayersPanel,
    doc_id: u64,
}

impl App {
    /// Takes the active document out of the app fields (leaving an empty document).
    fn park(&mut self) -> DocTab {
        DocTab {
            doc: std::mem::replace(&mut self.doc, crate::files::blank_document()),
            title: std::mem::take(&mut self.title),
            file: self.options.file.take(),
            page: self.page.take(),
            index: self.index.take(),
            selection: std::mem::take(&mut self.selection),
            history: std::mem::take(&mut self.history),
            saved_revision: self.saved_revision,
            camera: std::mem::take(&mut self.camera),
            fitted: self.fitted,
            layers: std::mem::take(&mut self.panels.layers),
            doc_id: self.doc_id,
        }
    }

    fn unpark(&mut self, t: DocTab) {
        self.doc = t.doc;
        self.title = t.title;
        self.options.file = t.file;
        self.page = t.page;
        self.index = t.index;
        self.selection = t.selection;
        self.history = t.history;
        self.saved_revision = t.saved_revision;
        self.camera = t.camera;
        self.fitted = t.fitted;
        self.panels.layers = t.layers;
        self.doc_id = t.doc_id;
        self.after_switch();
    }

    /// Per-document state that is not parked: hover, gestures, decoded images (keyed by the
    /// document's asset ids), the cached frame and copied layers.
    fn after_switch(&mut self) {
        self.hover = None;
        self.drag = None;
        self.end_text_edit();
        self.clipboard = ude_editor::Clipboard::default();
        self.caches.images = ude_renderer::images::ImageCache::default();
        if let Some(b) = &mut self.backend {
            b.invalidate();
        }
        self.update_title();
        self.redraw();
    }

    /// Opens `doc` in a new tab and makes it active.
    pub(crate) fn open_in_new_tab(&mut self, doc: Document, path: Option<PathBuf>) {
        if let Some(a) = self.active_tab {
            let parked = self.park();
            self.tabs[a] = Some(parked);
        }
        self.tabs.push(None);
        self.active_tab = Some(self.tabs.len() - 1);
        self.home = false;
        self.replace_document(doc, path);
    }

    pub(crate) fn switch_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if self.active_tab == Some(i) {
            self.home = false;
            self.redraw();
            return;
        }
        if let Some(a) = self.active_tab {
            let parked = self.park();
            self.tabs[a] = Some(parked);
        }
        if let Some(t) = self.tabs[i].take() {
            self.unpark(t);
        }
        self.active_tab = Some(i);
        self.home = false;
    }

    /// Closes tab `i` (asking to save); returns false when the user cancels.
    pub(crate) fn close_tab(&mut self, i: usize) -> bool {
        if i >= self.tabs.len() {
            return true;
        }
        let was_home = self.home;
        self.switch_tab(i);
        if !self.confirm_discard() {
            self.home = was_home;
            return false;
        }
        crate::autosave::remove(self.doc_id);
        self.tabs.remove(i);
        self.active_tab = None;
        if self.tabs.is_empty() {
            self.replace_document(crate::files::blank_document(), None);
            self.home = true;
            self.recents_dirty = true;
        } else {
            let next = i.min(self.tabs.len() - 1);
            if let Some(t) = self.tabs[next].take() {
                self.unpark(t);
            }
            self.active_tab = Some(next);
            self.home = was_home;
        }
        self.redraw();
        true
    }

    /// Asks about every unsaved tab before quitting; false when the user cancels.
    pub(crate) fn close_all(&mut self) -> bool {
        while !self.tabs.is_empty() {
            if !self.close_tab(self.tabs.len() - 1) {
                return false;
            }
        }
        true
    }

    /// Titles and unsaved flags for the tab strip.
    pub(crate) fn tab_infos(&self) -> Vec<ude_ui::TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| match t {
                Some(t) => ude_ui::TabInfo {
                    title: t.title.clone(),
                    dirty: t.doc.revision() != t.saved_revision,
                },
                None if Some(i) == self.active_tab => ude_ui::TabInfo {
                    title: self.title.clone(),
                    dirty: self.doc.revision() != self.saved_revision,
                },
                None => ude_ui::TabInfo {
                    title: String::new(),
                    dirty: false,
                },
            })
            .collect()
    }
}
