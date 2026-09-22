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
/// The checksum file the release workflow publishes beside the binary.
const SUMS: &str = "SHA256SUMS";

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
    /// The `SHA256SUMS` the release workflow hangs next to the binary.
    ///
    /// `None` for a release published before the updater learned to read it.
    /// Those still install on the shape checks alone, which is what they were
    /// verified by when they were current; refusing them would break updating
    /// *from* an old build, which is the one case an updater exists for.
    pub sums_url: Option<String>,
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

    let sums_url = api
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(SUMS))
        .map(|a| a.browser_download_url.clone());

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
        sums_url,
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

/// The digest a `SHA256SUMS` file records for one file name.
///
/// The format is one line per file, `<hex>  <name>`, which is what
/// `sha256sum` writes and what the release workflow reproduces from
/// PowerShell's `Get-FileHash`. Parsed leniently on whitespace so a file
/// written with one space, or with CRLF line endings, still reads.
fn digest_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let listed = parts.next()?.trim_start_matches('*');
        (listed.eq_ignore_ascii_case(name) && hex.len() == 64).then(|| hex.to_ascii_lowercase())
    })
}

fn sha256_of(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    // Streamed rather than read whole: the binary is several megabytes and
    // there is no reason for a second copy of it in memory.
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Guard against installing something that is not the release it claims to be.
///
/// The shape checks catch a rate-limit page, an error body or a truncated
/// stream. The checksum catches the rest of what a shape check cannot see: it
/// is compared against the `SHA256SUMS` the release workflow publishes beside
/// the binary, which until now the app hung there and never read.
///
/// ponytail: this still trusts HTTPS and GitHub, because both the binary and
/// the digest come from the same place — anyone who can replace one can
/// replace the other. It closes the accidental cases, not a compromised
/// account. Signing the releases and checking the signature here is the real
/// answer, and it costs a certificate.
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

    let Some(url) = &rel.sums_url else {
        return Ok(());
    };
    let sums = ureq::get(url)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(15))
        .call()?
        .into_string()?;
    check_digest(&sums, path)
}

/// The half of the checksum step that does not touch the network, so it can
/// be tested against a real file and a real `SHA256SUMS`.
fn check_digest(sums: &str, path: &Path) -> Result<()> {
    // A checksum file that does not name the asset is a fault in the release,
    // not a reason to install something unverified.
    let want = digest_for(sums, ASSET).ok_or_else(|| anyhow!(i18n::upd_err_no_digest(ASSET)))?;
    let got = sha256_of(path)?;
    if got != want {
        bail!(i18n::upd_err_checksum(&got, &want));
    }
    Ok(())
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".netdoctor-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Tells the new copy that the one it replaces is still shutting down, so it
/// waits for the single-instance name instead of deferring to a process that
/// is about to be gone. See [`crate::single::acquire_within`].
pub const AFTER_UPDATE_FLAG: &str = "--after-update";

/// Start the freshly installed binary. The caller exits straight after, which
/// is what releases the lock on the copy left behind.
pub fn restart() -> Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).arg(AFTER_UPDATE_FLAG).spawn()?;
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

    /// The known answer for "abc", so a broken hash cannot agree with itself.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("netdoctor-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_hash_matches_a_known_answer() {
        let path = temp_file("abc.bin", b"abc");
        assert_eq!(sha256_of(&path).unwrap(), ABC_SHA256);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_larger_than_the_read_buffer_hashes_the_same_as_windows_does() {
        // 200 000 bytes is three passes of the 64 KiB buffer plus a short
        // one, so an off-by-one in the chunking shows up here and nowhere in
        // the three-byte vector above. The expected digest was computed by
        // PowerShell's Get-FileHash over the same bytes — a second
        // implementation, which is the point of a known answer.
        let bytes: Vec<u8> = (0..200_000usize).map(|i| (i % 251) as u8).collect();
        let path = temp_file("chunked.bin", &bytes);
        assert_eq!(
            sha256_of(&path).unwrap(),
            "e24bc62381f1224fbbb74688663f8f9743b9680b193edd666835e97b06e730eb"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sums_file_is_read_the_way_the_workflow_writes_it() {
        // PowerShell's Out-File gives CRLF, and the workflow joins the hash to
        // the name with two spaces.
        let sums = format!("{ABC_SHA256}  netdoctor.exe\r\n");
        assert_eq!(digest_for(&sums, "netdoctor.exe").as_deref(), Some(ABC_SHA256));

        // sha256sum's binary marker, a single space, and an unrelated line
        // before the one that matters.
        let mixed = format!("deadbeef  notes.txt\n{ABC_SHA256} *netdoctor.exe\n");
        assert_eq!(digest_for(&mixed, "netdoctor.exe").as_deref(), Some(ABC_SHA256));

        assert!(digest_for(&sums, "other.exe").is_none(), "only the named file counts");
        assert!(
            digest_for("abc123  netdoctor.exe", "netdoctor.exe").is_none(),
            "a digest that is not 64 hex characters is not a digest"
        );
    }

    #[test]
    fn a_download_that_does_not_match_its_checksum_is_refused() {
        let path = temp_file("staged.bin", b"abc");
        let good = format!("{ABC_SHA256}  netdoctor.exe\n");
        assert!(check_digest(&good, &path).is_ok(), "the real digest of the real file");

        // One byte different, which is the whole point: the size and the
        // shape checks pass and only the digest notices.
        std::fs::write(&path, b"abd").unwrap();
        let err = check_digest(&good, &path).expect_err("a changed file must not install");
        assert!(err.to_string().contains(ABC_SHA256), "the message says what was expected");

        let missing = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef  other.exe";
        assert!(
            check_digest(missing, &path).is_err(),
            "a checksum file that never names the asset verifies nothing"
        );
        let _ = std::fs::remove_file(path);
    }

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
