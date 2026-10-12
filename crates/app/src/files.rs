//! Open, save and export through native file dialogs.

use std::path::{Path, PathBuf};

use ude_core::{Document, NodeId, NodeKind};

use crate::App;

const OPENABLE: &[&str] = &[
    "krt", "design", "fig", "jam", "deck", "buzz", "site", "make", "sketch", "xd", "ai", "pdf",
    "psd", "psb", "svg", "png", "jpg", "jpeg", "webp",
];

/// Loads a `.krt` document or imports another format.
pub(crate) fn load_path(path: &Path) -> Result<Document, String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            ude_format_raster::import(bytes, &file_name(path)).map_err(|e| err(&e))
        }
        "psd" | "psb" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im = ude_format_psd::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "sketch" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im = ude_format_sketch::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "ai" | "pdf" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let ai = ext == "ai";
            let im = ude_format_pdf::import(&bytes, &file_name(path), ai).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "xd" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im = ude_format_xd::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "fig" | "jam" | "deck" | "buzz" | "site" | "make" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im = ude_format_fig::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "svg" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im =
                ude_format_svg_import::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        _ => ude_format_native::load_from_path(path, &ude_format_native::Limits::default())
            .map_err(|e| err(&e)),
    }
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned())
}

impl App {
    pub(crate) fn open_dialog(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("Designs and images", OPENABLE)
            .add_filter("KuArt document", &["krt", "design"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    /// Opens another app's file (Figma, Sketch, XD, Photoshop, Illustrator, PDF, SVG, images)
    /// as a new KuArt document.
    pub(crate) fn import_dialog(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import")
            .add_filter(
                "Design files",
                &[
                    "fig", "jam", "deck", "buzz", "site", "make", "sketch", "xd", "psd", "psb",
                    "ai", "pdf", "svg",
                ],
            )
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    /// Loads `path`, records it in recents and leaves the home screen.
    pub(crate) fn open_path(&mut self, path: PathBuf) {
        match load_path(&path) {
            Ok(doc) => {
                self.replace_document(doc, Some(path.clone()));
                self.home = false;
                crate::recents::touch(&path);
                if let Some(page) = self.page {
                    crate::recents::write_thumbnail(&path, &self.doc, page, &mut self.caches);
                }
                self.recents_dirty = true;
            }
            Err(e) => {
                self.report(&e);
                if !path.exists() {
                    crate::recents::remove(&path);
                    self.recents_dirty = true;
                }
            }
        }
    }

    /// A new document with one desktop-sized frame.
    pub(crate) fn new_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let mut doc = blank_document();
        let page = doc.children(doc.root())[0];
        let name = doc.strings.intern("Desktop");
        let mut frame = ude_core::Node::new(NodeKind::Frame, name)
            .with_size(1440.0, 1024.0)
            .with_fills(ude_core::Paints::new(vec![ude_core::styles::Paint::solid(
                ude_core::styles::Color::WHITE,
            )]));
        frame.flags.set(ude_core::NodeFlags::CLIPS_CONTENT, true);
        let _ = doc.build_child(page, frame);
        let _ = doc.take_changes();
        self.replace_document(doc, None);
        self.home = false;
    }

    /// Closes the document and shows the home screen.
    pub(crate) fn go_home(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.replace_document(blank_document(), None);
        self.home = true;
        self.recents_dirty = true;
    }

    /// Asks to save unsaved changes; false when the user cancels.
    pub(crate) fn confirm_discard(&mut self) -> bool {
        if self.home || self.doc.revision() == self.saved_revision {
            return true;
        }
        match rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("KuArt")
            .set_description(format!("Save changes to \u{201c}{}\u{201d}?", self.title))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show()
        {
            rfd::MessageDialogResult::Yes => {
                self.save();
                self.doc.revision() == self.saved_revision
            }
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    pub(crate) fn add_page(&mut self) {
        let n = self.doc.children(self.doc.root()).len();
        let name = self.doc.strings.intern(&format!("Page {}", n + 1));
        let id = self.doc.create(ude_core::Node::new(NodeKind::Page, name));
        let root = self.doc.root();
        let mut tx = ude_core::Transaction::new("Add page");
        if tx
            .apply(
                &mut self.doc,
                ude_core::Op::Attach {
                    id,
                    parent: root,
                    index: n as u32,
                },
            )
            .is_ok()
        {
            self.history.commit(&mut self.doc, tx);
            self.select_page(id);
        }
    }

    pub(crate) fn select_page(&mut self, page: NodeId) {
        self.page = Some(page);
        self.index = Some(ude_spatial::SpatialIndex::build(&self.doc, page));
        self.selection.clear();
        self.hover = None;
        self.fitted = false;
        if let Some(b) = &mut self.backend {
            b.invalidate();
        }
        self.redraw();
    }

    pub(crate) fn rename(&mut self, id: NodeId, name: &str) {
        if name.is_empty() {
            return;
        }
        let value = ude_core::Prop::Name(self.doc.strings.intern(name));
        let mut tx = ude_core::Transaction::new("Rename");
        if tx
            .apply(&mut self.doc, ude_core::Op::Set { id, value })
            .is_ok()
        {
            self.history.commit(&mut self.doc, tx);
        }
    }

    /// Saves to the current `.krt` file, or asks where (imported images, demo).
    pub(crate) fn save(&mut self) {
        match self.options.file.clone().filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ude_format_native::is_native_extension(&e.to_ascii_lowercase()))
        }) {
            Some(path) => self.save_to(path),
            None => self.save_as(),
        }
    }

    pub(crate) fn save_as(&mut self) {
        let stem = self
            .title
            .rsplit_once('.')
            .map_or(self.title.as_str(), |(s, _)| s)
            .to_owned();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("KuArt document", &[ude_format_native::EXTENSION])
            .set_file_name(format!("{stem}.{}", ude_format_native::EXTENSION))
            .save_file()
        {
            self.save_to(path);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match ude_format_native::save_to_path(&self.doc, &path) {
            Ok(()) => {
                self.title = file_name(&path);
                crate::recents::touch(&path);
                if let Some(page) = self.page {
                    crate::recents::write_thumbnail(&path, &self.doc, page, &mut self.caches);
                }
                self.recents_dirty = true;
                self.options.file = Some(path);
                self.saved_revision = self.doc.revision();
                self.update_title();
            }
            Err(e) => self.report(&format!("{}: {e}", path.display())),
        }
    }

    /// Exports the first selected layer (or the page) with the Design tab's export options;
    /// the format can still be changed in the dialog by extension.
    pub(crate) fn export_dialog(&mut self) {
        let options = self.panels.export_options();
        self.export_with(options);
    }

    pub(crate) fn export_with(&mut self, o: ude_ui::ExportOptions) {
        let Some(target) = self.selection.ids().first().copied().or(self.page) else {
            return;
        };
        let name = self
            .doc
            .node(target)
            .map(|n| self.doc.strings.resolve(n.name))
            .filter(|s| !s.is_empty())
            .unwrap_or("export")
            .to_owned();
        let mut dialog = rfd::FileDialog::new().set_title("Export");
        // The chosen format first, so it is the dialog's default.
        let mut formats = ude_ui::ExportFormat::ALL.to_vec();
        formats.retain(|f| *f != o.format);
        formats.insert(0, o.format);
        for f in formats {
            dialog = dialog.add_filter(format!("{} file", f.label()), &[f.extension()]);
        }
        let suffix = if o.format.is_raster() && o.scale != 1.0 {
            format!("@{}x", o.scale)
        } else {
            String::new()
        };
        let Some(path) = dialog
            .set_file_name(format!("{name}{suffix}.{}", o.format.extension()))
            .save_file()
        else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let bytes = match ext.as_str() {
            "svg" => {
                let (doc, text) = (&self.doc, &mut self.caches.text);
                ude_format_svg::export_with(doc, target, &mut |t, size| {
                    text.lines(t, &doc.strings, size)
                })
                .map(String::into_bytes)
            }
            "pdf" => {
                ude_renderer::export_pdf(&self.doc, &[target], &mut self.caches).map(|(b, _)| b)
            }
            other => {
                let format = ude_renderer::RasterFormat::from_extension(other)
                    .unwrap_or(ude_renderer::RasterFormat::Png);
                let compression = ude_renderer::Compression {
                    palette: o.compress,
                    quality: o.quality,
                };
                ude_renderer::export_raster_with(
                    &self.doc,
                    target,
                    f64::from(o.scale),
                    format,
                    compression,
                    &mut self.caches,
                )
            }
        };
        if let Err(e) = bytes.and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
            self.report(&format!("{}: {e}", path.display()));
        }
    }

    pub(crate) fn replace_document(&mut self, mut doc: Document, path: Option<PathBuf>) {
        self.page = doc.children(doc.root()).iter().copied().find(|&id| {
            doc.node(id)
                .is_some_and(|n| matches!(n.kind, NodeKind::Page))
        });
        self.index = self.page.map(|p| ude_spatial::SpatialIndex::build(&doc, p));
        let _ = doc.take_changes();
        self.title = path.as_deref().map_or_else(|| "Untitled".into(), file_name);
        self.options.file = path;
        self.saved_revision = doc.revision();
        self.doc = doc;
        self.history = ude_core::History::default();
        self.selection.clear();
        // Copied layers reference the previous document's strings and assets.
        self.clipboard = ude_editor::Clipboard::default();
        self.hover = None;
        self.drag = None;
        self.panels.layers = ude_ui::LayersPanel::default();
        self.caches = ude_renderer::RenderCaches::default();
        if let Some(b) = &mut self.backend {
            b.invalidate();
        }
        self.fitted = false;
        self.update_title();
        self.redraw();
    }

    /// Window title with an unsaved-changes marker.
    pub(crate) fn update_title(&self) {
        if let Some(w) = &self.window {
            let dirty = if self.doc.revision() != self.saved_revision {
                "• "
            } else {
                ""
            };
            w.set_title(&format!("{dirty}{} — KuArt", self.title));
        }
    }

    fn report(&self, message: &str) {
        eprintln!("error: {message}");
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("KuArt")
            .set_description(message)
            .show();
    }
}

/// An empty document with one page.
pub(crate) fn blank_document() -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    let name = doc.strings.intern("Page 1");
    let _ = doc.build_child(root, ude_core::Node::new(NodeKind::Page, name));
    let _ = doc.take_changes();
    doc
}
