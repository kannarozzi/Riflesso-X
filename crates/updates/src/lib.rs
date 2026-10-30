//! Whether a newer Riflesso X is out: the latest release on GitHub, compared
//! with the running version. Also downloads that release's installer and
//! checks it against the published SHA-256.
//!
//! Downloads go through the system's own downloader (`URLDownloadToFile` on
//! Windows, `curl` or `wget` on Linux), so no HTTP library is added to the
//! programs. This is the only time Riflesso X contacts the
//! internet by itself, and users can turn it off.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST_RELEASE: &str = "https://api.github.com/repos/kannarozzi/Riflesso-X/releases/latest";

/// A published release.
#[derive(Debug, Clone)]
pub struct Release {
    /// "1.2.0", without the tag's "v".
    pub version: String,
    /// Release notes, as written on GitHub (Markdown).
    pub notes: String,
    /// The release's page on GitHub.
    pub page: String,
    installer: Option<String>,
    checksums: Option<String>,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    html_url: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// The latest release (GitHub leaves out drafts and pre-releases).
pub fn latest() -> Result<Release> {
    let temp = std::env::temp_dir().join(format!("riflesso-release-{}.json", std::process::id()));
    // A changing query string keeps Windows' download cache out of the way.
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let result = download(&format!("{LATEST_RELEASE}?t={stamp}"), &temp).and_then(|()| {
        let json = std::fs::read(&temp).context("reading the release information")?;
        serde_json::from_slice::<GitHubRelease>(&json).context("understanding the release information")
    });
    let _ = std::fs::remove_file(&temp);
    let release = result?;
    let asset = |suffix: &str| {
        release.assets.iter().find(|a| a.name.ends_with(suffix)).map(|a| a.browser_download_url.clone())
    };
    Ok(Release {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        installer: asset("_x64-setup.exe"),
        checksums: asset("SHA256SUMS.txt"),
        notes: release.body,
        page: release.html_url,
    })
}

/// Whether `candidate` is a later version than `current` ("1.10.0" > "1.9.2").
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split(['.', '-', '+']);
    let mut next = || parts.next()?.parse().ok();
    Some((next()?, next()?, next().unwrap_or(0)))
}

impl Release {
    /// Downloads the installer into `folder` and checks it against the
    /// release's SHA256SUMS.txt. Returns the installer's path.
    pub fn download_installer(&self, folder: &Path) -> Result<PathBuf> {
        let (Some(url), Some(sums_url)) = (&self.installer, &self.checksums) else {
            bail!("this release has no installer to download");
        };
        let name = url.rsplit('/').next().unwrap_or("Riflesso-X-setup.exe").to_owned();
        std::fs::create_dir_all(folder).context("creating the download folder")?;
        let installer = folder.join(&name);
        let sums = folder.join("SHA256SUMS.txt");
        download(sums_url, &sums)?;
        download(url, &installer)?;

        let listed = std::fs::read_to_string(&sums).context("reading the checksums")?;
        let _ = std::fs::remove_file(&sums);
        let expected = listed
            .lines()
            .find_map(|line| line.split_once(char::is_whitespace).filter(|(_, file)| file.trim() == name))
            .map(|(hash, _)| hash.to_ascii_lowercase())
            .context("the installer is missing from the checksums")?;
        let actual = hex::encode(Sha256::digest(std::fs::read(&installer).context("reading the installer")?));
        if actual != expected {
            let _ = std::fs::remove_file(&installer);
            bail!("the download does not match its published checksum");
        }
        Ok(installer)
    }
}

/// Saves `url` to `file` with Windows' downloader (follows GitHub's redirects).
#[cfg(windows)]
fn download(url: &str, file: &Path) -> Result<()> {
    use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    // SAFETY: COM is set up for this thread (already initialised is fine) and
    // the strings outlive the call.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        URLDownloadToFileW(None, &HSTRING::from(url), &HSTRING::from(file.as_os_str()), 0, None)
    }
    .with_context(|| format!("downloading {url}"))
}

/// Saves `url` to `file` with `curl`, or `wget` where curl is missing (some
/// desktop distributions ship only one of the two). Both follow redirects.
#[cfg(not(windows))]
fn download(url: &str, file: &Path) -> Result<()> {
    use std::process::{Command, Stdio};

    let user_agent = concat!("Riflesso-X/", env!("CARGO_PKG_VERSION"));
    let attempts = [
        ("curl", vec!["-fsSL", "--max-time", "120", "-A", user_agent, "-o"]),
        ("wget", vec!["-q", "--timeout=120", "-U", user_agent, "-O"]),
    ];
    for (program, args) in attempts {
        let status = Command::new(program).args(args).arg(file).arg(url).stdin(Stdio::null()).status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => bail!("downloading {url}: {program} failed ({status})"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => bail!("downloading {url}: cannot run {program}: {e}"),
        }
    }
    bail!("downloading {url}: neither curl nor wget is installed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_as_numbers() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.2"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("not a version", "1.0.0"));
    }

    /// Needs the internet: `cargo test -p riflesso-updates -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reads_the_latest_release_from_github() {
        let release = latest().unwrap();
        println!("latest: {} ({}), installer: {:?}", release.version, release.page, release.installer);
        assert!(parse(&release.version).is_some());
        if release.installer.is_some() {
            let folder = std::env::temp_dir().join(format!("riflesso-update-test-{}", std::process::id()));
            let installer = release.download_installer(&folder).unwrap();
            println!("downloaded and verified: {}", installer.display());
            std::fs::remove_dir_all(folder).unwrap();
        }
    }
}
