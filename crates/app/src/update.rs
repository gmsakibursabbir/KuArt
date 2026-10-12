//! Updates from GitHub Releases, only when the user asks (Help → Check for updates); KuArt
//! never goes online by itself. Downloads are accepted only from this repository's release
//! assets and must match the release's SHA256SUMS before anything is installed.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

const LATEST: &str = "https://api.github.com/repos/gmsakibursabbir/KuArt/releases/latest";
const DOWNLOADS: &str = "https://github.com/gmsakibursabbir/KuArt/releases/download/";
const MAX_BYTES: u64 = 400 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// Release notes page.
    pub url: String,
    /// The package this installation can update itself with, and the checksum list.
    asset: Option<(String, String)>,
    sums: Option<String>,
}

impl Release {
    /// False for packages a system package manager owns (deb, rpm, Arch): open the page instead.
    pub fn installable(&self) -> bool {
        self.asset.is_some() && self.sums.is_some()
    }
}

/// What a finished install needs the app to do.
pub enum Installed {
    /// The verified installer: run it and quit so it can replace the files.
    Run(PathBuf),
    /// The AppImage was replaced: the new version starts next time.
    Restart,
}

/// The newest release, if it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    let body = ureq::get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("cannot reach GitHub: {e}"))?
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim_start_matches('v').to_owned();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets: Vec<(String, String)> = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_owned();
            let url = a["browser_download_url"].as_str()?.to_owned();
            // Trust boundary: only this repository's release downloads.
            url.starts_with(DOWNLOADS).then_some((name, url))
        })
        .collect();
    let find = |suffix: &str| assets.iter().find(|(n, _)| n.ends_with(suffix)).cloned();
    let asset = if cfg!(windows) {
        find("-x64-setup.exe")
    } else if std::env::var_os("APPIMAGE").is_some() {
        find(".AppImage")
    } else {
        None
    };
    Ok(Some(Release {
        url: json["html_url"]
            .as_str()
            .filter(|u| u.starts_with("https://github.com/gmsakibursabbir/KuArt/"))
            .unwrap_or("https://github.com/gmsakibursabbir/KuArt/releases/latest")
            .to_owned(),
        version,
        asset,
        sums: find("SHA256SUMS").map(|(_, u)| u),
    }))
}

/// `a` > `b` for dotted versions ("0.10.0" > "0.9.1"); unparsable parts count as 0.
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    !a.is_empty() && parse(a) > parse(b)
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("download failed: {e}"))
}

/// Downloads, verifies and installs the update.
pub fn install(r: &Release) -> Result<Installed, String> {
    let (Some((name, url)), Some(sums)) = (&r.asset, &r.sums) else {
        return Err("no package for this installation; download it from the release page".into());
    };
    let sums = String::from_utf8(get(sums, 1024 * 1024)?).map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (hash, file) = l.split_once(char::is_whitespace)?;
            (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| format!("{name} is not listed in SHA256SUMS"))?;
    let bytes = get(url, MAX_BYTES)?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(format!("{name} failed its checksum; not installed"));
    }
    if cfg!(windows) {
        let dir = std::env::temp_dir().join("KuArt-update");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(name);
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Installed::Run(path))
    } else {
        let target = PathBuf::from(std::env::var_os("APPIMAGE").ok_or("not an AppImage")?);
        let tmp = target.with_extension("new");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        // Replacing a running AppImage is safe: the running copy stays mounted.
        std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        Ok(Installed::Restart)
    }
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn version_order() {
        assert!(newer("0.3.0", "0.2.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.99.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
        assert!(!newer("", "0.2.0"));
    }
}
