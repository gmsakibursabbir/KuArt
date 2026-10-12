//! Per-user data locations (std only).

use std::path::PathBuf;

/// KuArt's per-user data folder: `%APPDATA%\KuArt` on Windows, `$XDG_DATA_HOME/kuart` (or
/// `~/.local/share/kuart`) elsewhere.
pub fn user_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("KuArt"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .map(|d| d.join("kuart"))
    }
}

/// Folder for user-added fonts (e.g. downloaded from Google Fonts), loaded on startup.
pub fn user_fonts_dir() -> Option<PathBuf> {
    user_data_dir().map(|d| d.join("fonts"))
}
