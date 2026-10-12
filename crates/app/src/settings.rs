//! Preferences remembered between launches (`settings.txt` in the KuArt data folder,
//! `key=value` lines).

use ude_ui::ThemeChoice;

fn path() -> Option<std::path::PathBuf> {
    ude_core::paths::user_data_dir().map(|d| d.join("settings.txt"))
}

fn get(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path()?).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_owned())
}

fn set(key: &str, value: &str) {
    let Some(p) = path() else { return };
    let mut lines: Vec<String> = std::fs::read_to_string(&p)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{key}={value}"));
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, lines.join("\n"));
}

pub fn load_theme() -> ThemeChoice {
    match get("theme").as_deref() {
        Some("dark") => ThemeChoice::Dark,
        Some("light") => ThemeChoice::Light,
        Some("high-contrast") => ThemeChoice::HighContrast,
        _ => ThemeChoice::System,
    }
}

pub fn save_theme(t: ThemeChoice) {
    set(
        "theme",
        match t {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
            ThemeChoice::HighContrast => "high-contrast",
        },
    );
}
