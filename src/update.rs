//! Self-update against the project's GitHub releases.
//!
//! Windows refuses to delete or overwrite a running image, but it is happy to
//! rename one. So the swap is: move the running binary aside, drop the new one
//! into its place, relaunch. The renamed copy is still locked until this
//! process exits, so it is deleted on the next start rather than here.
//!
//! Everything in this module blocks, and none of it touches the UI. The caller
//! runs it on a worker thread and reports progress down the job channel.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use crate::i18n;

/// Where releases come from. A fork that wants its own update feed changes
/// this one line.
pub const REPO: &str = "pponikiewski/netdoctor";

/// The asset name the release workflow uploads. Anything else attached to a
/// release is not something this build knows how to install.
const ASSET: &str = "netdoctor.exe";

/// Suffix for the outgoing binary, left behind until the next start.
const OLD_SUFFIX: &str = "old";
/// Suffix for the download, before it is put in place.
const NEW_SUFFIX: &str = "new";

/// A downloaded file smaller than this is a rate-limit page or a truncated
/// stream, not a build. The real binary is several megabytes.
const MIN_PLAUSIBLE_BYTES: u64 = 1 << 20;

const UA: &str = concat!("netdoctor/", env!("CARGO_PKG_VERSION"));

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// A release newer than this build, with everything needed to install it.
#[derive(Debug, Clone)]
pub struct Release {
    /// Normalised: the tag with any leading `v` removed.
    pub version: String,
    /// The release body, shown as "what's new". May be empty.
    pub notes: String,
    pub page: String,
    pub asset_url: String,
    /// Bytes, as GitHub reports them. `0` when the API omitted it, which only
    /// costs the progress bar its denominator.
    pub size: u64,
}

/// How far along an update is. One value rather than a handful of booleans,
/// because "downloading and also up to date" is not a state that should be
/// representable.
#[derive(Debug, Clone, Default)]
pub enum State {
    #[default]
    Idle,
    Checking,
    /// Checked, and this build is current.
    Current,
    Available(Box<Release>),
    /// Fraction is `None` until the first byte arrives with a known total.
    Downloading(Box<Release>, Option<f32>),
    /// Downloaded and put in place. Only a restart is left.
    Installed(Box<Release>),
    Failed(String),
}

// ---------------------------------------------------------------------------
// checking
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// `Ok(None)` means this build is already current — not that the check failed.
pub fn check() -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = ureq::get(&url)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(15))
        .call()?
        .into_string()?;

    // Parsed by hand rather than through ureq's `into_json`, which is behind a
    // default feature this crate does not ask for by name.
    let api: ApiRelease = serde_json::from_str(&body)?;

    // `releases/latest` already excludes both, but a repo can be configured to
    // surface a prerelease and that is not something to push at everyone.
    if api.draft || api.prerelease {
        return Ok(None);
    }

    let version = normalise(&api.tag_name);
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }

    let asset = api
        .assets
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET))
        .ok_or_else(|| anyhow!(i18n::upd_err_no_asset(&version, ASSET)))?;

    Ok(Some(Release {
        version,
        notes: api.body.trim().to_string(),
        page: api.html_url,
        asset_url: asset.browser_download_url,
        size: asset.size,
    }))
}

fn normalise(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Numeric compare on the first three components, so `1.10.0` beats `1.9.0`
/// where a string compare would not. A component that is not a number counts
/// as zero, which is what a `1.2.0-rc1` build should compare as: not newer
/// than `1.2.0`.
fn parts(v: &str) -> (u32, u32, u32) {
    let mut it = v.split(['.', '-', '+']).map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parts(candidate) > parts(current)
}

// ---------------------------------------------------------------------------
// installing
// ---------------------------------------------------------------------------

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    exe.with_file_name(name)
}

/// Download the release next to the running binary and put it in place.
///
/// `on_progress` is called with the fraction downloaded whenever it moves, and
/// with `None` when the server did not say how big the file is.
pub fn install(rel: &Release, mut on_progress: impl FnMut(Option<f32>)) -> Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| anyhow!(i18n::upd_err_no_dir()))?;

    // Checked before the download rather than after it: a copy installed under
    // Program Files cannot replace itself without elevation, and finding that
    // out after pulling several megabytes is a worse way to learn it.
    if !writable(dir) {
        bail!(i18n::upd_err_read_only(&dir.to_string_lossy()));
    }

    let staged = sibling(&exe, NEW_SUFFIX);
    let _ = std::fs::remove_file(&staged);
    download(rel, &staged, &mut on_progress)?;

    if let Err(e) = verify(&staged, rel) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    let old = sibling(&exe, OLD_SUFFIX);
    // A leftover from a previous update that the next start never got to.
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old)?;
    if let Err(e) = std::fs::rename(&staged, &exe) {
        // Put the running binary back. Without this the install directory is
        // left with no executable at the expected path.
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&staged);
        return Err(e.into());
    }
    Ok(())
}

fn download(rel: &Release, to: &Path, on_progress: &mut impl FnMut(Option<f32>)) -> Result<()> {
    // Built as an agent rather than a bare request for the read timeout, which
    // is the one that matters here: `timeout` bounds getting a response, but a
    // line that goes dead partway through several megabytes of body would
    // otherwise hang this thread for as long as the app stays open.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build();
    let resp = agent.get(&rel.asset_url).set("User-Agent", UA).call()?;

    // The API's size is the asset's; the redirect we actually land on reports
    // its own. Prefer the latter, and fall back so a missing header only costs
    // the progress bar.
    let total = resp
        .header("Content-Length")
        .and_then(|h| h.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(rel.size);

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(to)?;
    let mut buf = [0u8; 65536];
    let mut done: u64 = 0;
    let mut reported = 0.0f32;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        done += n as u64;

        if total == 0 {
            on_progress(None);
        } else {
            let frac = (done as f32 / total as f32).min(1.0);
            // Repainting on every 64 KiB chunk is a repaint per millisecond on
            // a fast line, for a bar that moves a fraction of a pixel.
            if frac - reported >= 0.01 {
                reported = frac;
                on_progress(Some(frac));
            }
        }
    }
    std::io::Write::flush(&mut file)?;
    Ok(())
}

/// Guard against installing something that is not a Windows executable: a
/// rate-limit page, an error body, or a stream that was cut short.
///
/// ponytail: shape and size only. Signing the releases and checking the
/// signature here is the real answer if the binary ever ships beyond people
/// who know where it came from.
fn verify(path: &Path, rel: &Release) -> Result<()> {
    let len = std::fs::metadata(path)?.len();
    if len < MIN_PLAUSIBLE_BYTES {
        bail!(i18n::upd_err_too_small(len));
    }
    if rel.size > 0 && len != rel.size {
        bail!(i18n::upd_err_size_mismatch(len, rel.size));
    }
    let mut magic = [0u8; 2];
    std::fs::File::open(path)?.read_exact(&mut magic)?;
    if &magic != b"MZ" {
        bail!(i18n::upd_err_not_exe());
    }
    Ok(())
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".netdoctor-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Start the freshly installed binary. The caller exits straight after, which
/// is what releases the lock on the copy left behind.
pub fn restart() -> Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

/// Delete the binary the last update moved aside. Called once at start, when
/// the old process is gone and the file is no longer locked. Failure is not
/// worth reporting: the file is harmless, and the next start tries again.
pub fn clean_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(sibling(&exe, OLD_SUFFIX));
        let _ = std::fs::remove_file(sibling(&exe, NEW_SUFFIX));
    }
}

/// Hand a URL to the shell. egui's own `open_url` is a no-op on this backend.
pub fn open_in_browser(url: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide =
        |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file = wide(url);
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numerically_not_lexically() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(is_newer("1.1.1", "1.1.0"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
    }

    #[test]
    fn a_prerelease_tag_does_not_outrank_the_release() {
        // The suffix parses as nothing, so `1.2.0-rc1` and `1.2.0` compare
        // equal and the running build is left alone.
        assert!(!is_newer("1.2.0-rc1", "1.2.0"));
        assert!(is_newer("1.2.0-rc1", "1.1.0"));
    }

    #[test]
    fn a_leading_v_is_stripped_from_either_case() {
        assert_eq!(normalise("v1.2.3"), "1.2.3");
        assert_eq!(normalise(" V1.2.3 "), "1.2.3");
        assert_eq!(normalise("1.2.3"), "1.2.3");
    }

    #[test]
    fn garbage_in_a_tag_does_not_trigger_an_update() {
        assert!(!is_newer("not-a-version", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn staging_paths_sit_next_to_the_binary() {
        let exe = Path::new(r"C:\apps\netdoctor.exe");
        assert_eq!(sibling(exe, OLD_SUFFIX), Path::new(r"C:\apps\netdoctor.exe.old"));
        assert_eq!(sibling(exe, NEW_SUFFIX), Path::new(r"C:\apps\netdoctor.exe.new"));
    }
}
