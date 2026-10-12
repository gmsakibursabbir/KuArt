//! Open, save and export through native file dialogs.

use std::path::{Path, PathBuf};

use ude_core::{Document, NodeKind};

use crate::App;

const OPENABLE: &[&str] = &[
    "design", "fig", "psd", "psb", "svg", "png", "jpg", "jpeg", "webp",
];

/// Loads a `.design` document or imports an image.
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
        "xd" => {
            let bytes = std::fs::read(path).map_err(|e| err(&e))?;
            let im = ude_format_xd::import(&bytes, &file_name(path)).map_err(|e| err(&e))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "fig" => {
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
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Designs and images", OPENABLE)
            .pick_file()
        else {
            return;
        };
        match load_path(&path) {
            Ok(doc) => self.replace_document(doc, Some(path)),
            Err(e) => self.report(&e),
        }
    }

    /// Saves to the current `.design` file, or asks where (imported images, demo).
    pub(crate) fn save(&mut self) {
        match self
            .options
            .file
            .clone()
            .filter(|p| p.extension().is_some_and(|e| e == "design"))
        {
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
            .add_filter("KuArt design", &["design"])
            .set_file_name(format!("{stem}.design"))
            .save_file()
        {
            self.save_to(path);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match ude_format_native::save_to_path(&self.doc, &path) {
            Ok(()) => {
                self.title = file_name(&path);
                self.options.file = Some(path);
                self.saved_revision = self.doc.revision();
                self.update_title();
            }
            Err(e) => self.report(&format!("{}: {e}", path.display())),
        }
    }

    /// Exports the first selected layer (or the page) as PNG or SVG, chosen by extension.
    pub(crate) fn export_dialog(&mut self) {
        let Some(target) = self.selection.ids().first().copied().or(self.page) else {
            return;
        };
        let name = self
            .doc
            .node(target)
            .map(|n| self.doc.strings.resolve(n.name))
            .filter(|s| !s.is_empty())
            .unwrap_or("export");
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .add_filter("SVG vector", &["svg"])
            .set_file_name(format!("{name}.png"))
            .save_file()
        else {
            return;
        };
        let svg = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        let bytes = if svg {
            let (doc, text) = (&self.doc, &mut self.caches.text);
            ude_format_svg::export_with(doc, target, &mut |t, size| {
                text.lines(t, &doc.strings, size)
            })
            .map(String::into_bytes)
        } else {
            // 2x for crisp results on high-DPI screens.
            ude_renderer::export_png(&self.doc, target, 2.0, &mut self.caches)
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
