//! Autosave and crash recovery. While a document has unsaved changes, it is written to
//! `<data>/autosave/<id>.krt` once the user pauses (2 s without input, at most once a
//! minute), so saving never interrupts a gesture. A clean save, closing a tab or quitting
//! removes the file; files left over at startup mean KuArt did not exit cleanly.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Minimum time between autosaves of the active document.
pub const INTERVAL: Duration = Duration::from_secs(60);
/// Input pause before an autosave runs.
pub const IDLE: Duration = Duration::from_secs(2);

fn dir() -> Option<PathBuf> {
    ude_core::paths::user_data_dir().map(|d| d.join("autosave"))
}

/// A new id for an open document (unique per process and time).
pub fn new_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    (t << 16) ^ NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Writes the document and a one-line note with its title and original path.
pub fn write(
    id: u64,
    doc: &ude_core::Document,
    title: &str,
    original: Option<&std::path::Path>,
) -> Result<(), String> {
    let dir = dir().ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = ude_format_native::save(doc).map_err(|e| e.to_string())?;
    // Write then rename, so a crash mid-write never leaves a broken recovery file.
    let tmp = dir.join(format!("{id}.krt.tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(format!("{id}.krt"))).map_err(|e| e.to_string())?;
    let note = format!(
        "{title}\n{}",
        original
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    std::fs::write(dir.join(format!("{id}.txt")), note).map_err(|e| e.to_string())
}

pub fn remove(id: u64) {
    if let Some(dir) = dir() {
        let _ = std::fs::remove_file(dir.join(format!("{id}.krt")));
        let _ = std::fs::remove_file(dir.join(format!("{id}.txt")));
    }
}

/// A document left behind by a previous run.
pub struct Leftover {
    pub id: u64,
    pub file: PathBuf,
    pub title: String,
    pub original: Option<PathBuf>,
}

pub fn leftovers() -> Vec<Leftover> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Leftover> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "krt"))
        .filter_map(|file| {
            let id: u64 = file.file_stem()?.to_str()?.parse().ok()?;
            let note = std::fs::read_to_string(dir.join(format!("{id}.txt"))).unwrap_or_default();
            let mut lines = note.lines();
            let title = lines.next().unwrap_or("Recovered").to_owned();
            let original = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
            Some(Leftover {
                id,
                file,
                title,
                original,
            })
        })
        .collect();
    out.sort_by_key(|l| l.id);
    out
}

/// When the next autosave check should run, if one is due.
pub fn due(dirty: bool, last_save: Instant, last_input: Instant) -> Option<Instant> {
    if !dirty {
        return None;
    }
    Some((last_save + INTERVAL).max(last_input + IDLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_waits_for_a_pause_and_the_interval() {
        let t0 = Instant::now();
        assert_eq!(due(false, t0, t0), None, "nothing to save");
        // Edited just now, last save long ago: wait for the 2 s pause.
        let long_ago = t0 - INTERVAL * 2;
        assert_eq!(due(true, long_ago, t0), Some(t0 + IDLE));
        // Saved recently: wait for the interval even when idle.
        assert_eq!(due(true, t0, t0 - IDLE * 10), Some(t0 + INTERVAL));
    }
}
