//! Per-user data locations (std only).

use std::path::PathBuf;

/// Folder for user-added fonts (e.g. downloaded from Google Fonts), loaded on startup:
/// `%APPDATA%\KuArt\fonts` on Windows, `$XDG_DATA_HOME/kuart/fonts` (or
/// `~/.local/share/kuart/fonts`) elsewhere.
pub fn user_fonts_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("KuArt").join("fonts"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .map(|d| d.join("kuart").join("fonts"))
    }
}
