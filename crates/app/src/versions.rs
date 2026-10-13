//! Version history: before a document is saved over, its previous file is copied into the
//! KuArt data folder (the newest `KEEP` per document). File → Version history opens one.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP: usize = 20;

fn dir_for(file: &Path) -> Option<PathBuf> {
    let file = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let hash = blake3::hash(file.to_string_lossy().as_bytes()).to_hex();
    ude_core::paths::user_data_dir().map(|d| d.join("versions").join(&hash[..16]))
}

/// Keeps a copy of `file` as it is now (called just before it is overwritten).
pub fn keep(file: &Path) {
    let (Some(dir), true) = (dir_for(file), file.exists()) else {
        return;
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if std::fs::create_dir_all(&dir).is_err()
        || std::fs::copy(file, dir.join(format!("{secs}.krt"))).is_err()
    {
        return;
    }
    for (old, _) in list(file).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
}

/// Saved versions of `file`, newest first, with when they were saved.
pub fn list(file: &Path) -> Vec<(PathBuf, u64)> {
    let Some(dir) = dir_for(file) else {
        return Vec::new();
    };
    let mut v: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let secs = p.file_stem()?.to_str()?.parse().ok()?;
            Some((p, secs))
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    v
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn age(secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match now.saturating_sub(secs) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::age;

    #[test]
    fn ages_read_naturally() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(age(now), "just now");
        assert_eq!(age(now - 300), "5 min ago");
        assert_eq!(age(now - 3 * 86_400), "3 days ago");
    }
}
