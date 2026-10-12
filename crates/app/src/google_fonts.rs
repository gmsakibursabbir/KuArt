//! On-demand Google Fonts download. Runs only when the user clicks "Get" for a missing
//! font (KuArt never goes online by itself). Files are saved to the user fonts folder
//! and loaded on every launch.

use std::path::{Path, PathBuf};

const CSS_API: &str = "https://fonts.googleapis.com/css";
/// Only font files from Google's font CDN are accepted.
const FONT_HOST: &str = "https://fonts.gstatic.com/";
const MAX_FONT_BYTES: u64 = 20 * 1024 * 1024;

/// Downloads every weight and italic of `family` into `dir`. Returns the saved files.
pub fn download(family: &str, dir: &Path) -> Result<Vec<PathBuf>, String> {
    // Family names are letters, digits and spaces; anything else could alter the URL.
    if family.is_empty()
        || !family
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ')
    {
        return Err(format!(
            "\u{201c}{family}\u{201d} is not a valid Google Fonts family name"
        ));
    }
    let weights: Vec<String> = (1..=9)
        .flat_map(|w| [format!("{w}00"), format!("{w}00i")])
        .collect();
    let url = format!(
        "{CSS_API}?family={}:{}",
        family.replace(' ', "+"),
        weights.join(",")
    );
    // A non-browser user agent makes the API answer with TTF files (not WOFF2).
    let css = ureq::get(&url)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(400 | 404) => {
                format!("\u{201c}{family}\u{201d} is not on Google Fonts")
            }
            e => format!("cannot reach Google Fonts: {e}"),
        })?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;

    let mut urls: Vec<&str> = css
        .split("url(")
        .skip(1)
        .filter_map(|s| s.split(')').next())
        .filter(|u| u.starts_with(FONT_HOST))
        .collect();
    urls.sort_unstable();
    urls.dedup();
    if urls.is_empty() {
        return Err(format!(
            "Google Fonts returned no files for \u{201c}{family}\u{201d}"
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem: String = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut saved = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let bytes = ureq::get(*url)
            .call()
            .map_err(|e| format!("download failed: {e}"))?
            .body_mut()
            .with_config()
            .limit(MAX_FONT_BYTES)
            .read_to_vec()
            .map_err(|e| format!("download failed: {e}"))?;
        let path = dir.join(format!("{stem}-{i}.ttf"));
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        saved.push(path);
    }
    Ok(saved)
}
