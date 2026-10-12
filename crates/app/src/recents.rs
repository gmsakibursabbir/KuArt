//! Recent files for the home screen: a list in the KuArt data folder (`recent.txt`, newest
//! first) and a thumbnail per file (`thumbs/<hash>.png`, rendered when it is opened or saved).

use std::path::{Path, PathBuf};

use std::time::SystemTime;
use ude_ui::egui;

use ude_core::{Document, NodeId};
use ude_renderer::RenderCaches;
use ude_ui::Recent;

const MAX_RECENTS: usize = 30;
const THUMB_WIDTH: f64 = 320.0;

fn data_dir() -> Option<PathBuf> {
    ude_core::paths::user_data_dir()
}

fn list_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("recent.txt"))
}

fn thumb_path(file: &Path) -> Option<PathBuf> {
    let hash = blake3::hash(file.to_string_lossy().as_bytes()).to_hex();
    data_dir().map(|d| d.join("thumbs").join(format!("{}.png", &hash[..16])))
}

/// Paths in the list, newest first; files that no longer exist are dropped.
fn load_paths() -> Vec<PathBuf> {
    let Some(text) = list_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    text.lines()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .take(MAX_RECENTS)
        .collect()
}

fn save_paths(paths: &[PathBuf]) {
    let Some(p) = list_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let _ = std::fs::write(p, text.join("\n"));
}

/// "Just now", "5 minutes ago", "Yesterday", "12 Oct 2026"-style relative times.
fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => "Edited just now".into(),
        60..=3599 => format!("Edited {} min ago", secs / 60),
        3600..=86_399 => format!("Edited {} h ago", secs / 3600),
        86_400..=172_799 => "Edited yesterday".into(),
        _ => format!("Edited {} days ago", secs / 86_400),
    }
}

/// The recents list for the UI (thumbnails are loaded separately).
pub fn load() -> Vec<Recent> {
    load_paths()
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let ext = path
                .extension()
                .map_or_else(String::new, |e| e.to_string_lossy().to_ascii_lowercase());
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(ago)
                .unwrap_or_default();
            Recent {
                path,
                name,
                ext,
                modified,
                thumbnail: None,
            }
        })
        .collect()
}

/// Moves `path` to the top of the list.
pub fn touch(path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut paths = load_paths();
    paths.retain(|p| p != &path);
    paths.insert(0, path);
    paths.truncate(MAX_RECENTS);
    save_paths(&paths);
}

pub fn remove(path: &Path) {
    let mut paths = load_paths();
    paths.retain(|p| p != path);
    save_paths(&paths);
    if let Some(t) = thumb_path(path) {
        let _ = std::fs::remove_file(t);
    }
}

/// Renders a small preview of `page` for `path`'s card.
pub fn write_thumbnail(path: &Path, doc: &Document, page: NodeId, caches: &mut RenderCaches) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Some(out) = thumb_path(&path) else { return };
    let Some(bounds) = ude_renderer::export_bounds(doc, page) else {
        return;
    };
    let scale = (THUMB_WIDTH / bounds.width().max(1.0))
        .min(240.0 / bounds.height().max(1.0))
        .min(1.0);
    if let Ok(png) =
        ude_renderer::export_raster(doc, page, scale, ude_renderer::RasterFormat::Png, caches)
    {
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(out, png);
    }
}

/// Loads missing thumbnail textures into egui.
pub fn load_textures(ctx: &egui::Context, recents: &mut [Recent]) {
    for r in recents.iter_mut().filter(|r| r.thumbnail.is_none()) {
        let Some(bytes) = thumb_path(&r.path).and_then(|p| std::fs::read(p).ok()) else {
            continue;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        r.thumbnail = Some(ctx.load_texture(
            format!("thumb:{}", r.path.display()),
            color,
            egui::TextureOptions::LINEAR,
        ));
    }
}
