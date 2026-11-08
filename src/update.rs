//! The app's own updates. An installed app (Linux: `~/.local/share/h0x-ade`,
//! macOS: `HyperOx.app`, Windows: `%LOCALAPPDATA%\Programs\h0x-ade`) checks
//! the latest GitHub release's `latest.json` a little
//! after it opens and every hour, downloads a newer build for this
//! machine, checks its sha256 and minisign signature and unpacks it next to
//! itself (`<install>.new`).
//! The frame then shows "Restart"; the swap happens on restart or the next
//! quit, or before opening the next window after a crash. The first start
//! after a swap runs `h0x-core update` for the core.
//!
//! A build from source never updates: only `scripts/package` (the release
//! builds) sets `H0X_ADE_RELEASE`. `scripts/install-macos` makes a
//! HyperOx.app too, so the install's shape alone can't tell.
//!
//! The core is checked on the same clock (the core release's `latest.json`): a newer core
//! of the same generation is offered (also applied after an app update), unless
//! a package manager owns it. Within a generation a newer core only adds.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const EVERY: Duration = Duration::from_secs(60 * 60);
/// After a failed check or download (offline, a damaged file).
const RETRY: Duration = Duration::from_secs(30 * 60);

/// Where this app is installed: the folder (Linux) or bundle (macOS) an
/// update replaces. Read once: after a swap this process's own file is gone.
fn root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ROOT.get_or_init(find_root).clone()
}

fn find_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    if cfg!(target_os = "macos") {
        // HyperOx.app/Contents/MacOS/HyperOx
        let bundle = dir.parent()?.parent()?;
        (bundle.extension()? == "app").then(|| bundle.to_path_buf())
    } else {
        // The release folder holds the binary and its icon; cargo's target/ doesn't.
        dir.join("h0x-ade.svg").is_file().then(|| dir.to_path_buf())
    }
}

/// The folder or bundle name inside a release tarball.
const PACKED: &str = if cfg!(target_os = "macos") { "HyperOx.app" } else { "h0x-ade" };

fn beside(root: &Path, suffix: &str) -> PathBuf {
    let mut name = root.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    // macOS: Spotlight skips a folder ending in `.noindex`, so a staged
    // `HyperOx.app.new` never shows up in search under that name.
    if cfg!(target_os = "macos") {
        name.push(".noindex");
    }
    root.with_file_name(name)
}

fn target() -> String {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// Environment overrides (release URLs, core dirs) count only in debug and
/// test builds: a release build reads its releases from GitHub alone.
pub(crate) const OVERRIDES: bool = cfg!(any(debug_assertions, test));

/// A releases base URL from the environment; empty counts as unset, a trailing `/` is dropped.
fn env_url(key: &str) -> Option<String> {
    overridden(OVERRIDES, std::env::var(key).ok())
}

fn overridden(honoured: bool, value: Option<String>) -> Option<String> {
    value.filter(|v| honoured && !v.trim().is_empty()).map(|v| v.trim().trim_end_matches('/').to_owned())
}

/// The only hosts the app downloads from: feeds, releases and release-note media.
const TRUSTED_HOSTS: [&str; 3] = ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// curl may neither start nor be redirected anywhere but https.
pub(crate) const CURL_HTTPS: [&str; 4] = ["--proto", "=https", "--proto-redir", "=https"];

/// The curl flags for `url`: https only, except the unit tests' loopback stub.
fn curl_flags(url: &str) -> [&'static str; 4] {
    if loopback_stub(url) { ["--proto", "=https,http", "--proto-redir", "=https"] } else { CURL_HTTPS }
}

/// Unit tests only: a plain-http release server on `http://127.0.0.1:<port>/…`.
#[cfg(test)]
fn loopback_stub(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.split(['/', '?', '#']).next()).is_some_and(|port| port.parse::<u16>().is_ok())
}

/// Release and debug builds: GitHub over https, nothing else.
#[cfg(not(test))]
fn loopback_stub(_url: &str) -> bool {
    false
}

/// `https://<trusted host>/…`, the host exactly: no userinfo, no port.
pub(crate) fn trusted_url(url: &str) -> bool {
    if loopback_stub(url) {
        return true;
    }
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    TRUSTED_HOSTS.contains(&authority)
}

fn untrusted(url: &str) -> String {
    format!("{url}: not an https GitHub URL")
}

/// GitHub release files redirect to *.githubusercontent.com: curl follows at
/// most 5 redirects and prints the URL it landed on after the body (stdout).
pub(crate) const CURL_LANDED: [&str; 4] = ["--max-redirs", "5", "--write-out", r"\n%{url_effective}"];

/// Splits curl's stdout (run with `CURL_LANDED`) at its last newline into the
/// body and the final URL; a final URL that is not trusted is refused.
pub(crate) fn landed(stdout: &[u8]) -> Result<&[u8], String> {
    let at = stdout.iter().rposition(|&b| b == b'\n').ok_or("curl did not report the final URL")?;
    let url = String::from_utf8_lossy(&stdout[at + 1..]);
    if !trusted_url(&url) {
        return Err(untrusted(&url));
    }
    Ok(&stdout[..at])
}

/// Downloads a trusted `url` into `file`, giving up after `max_time` seconds.
/// A download that was redirected off the trusted hosts, or that `<url>.minisig`
/// does not sign with the release key, is deleted. Ok: the bytes that were
/// checked; use these, not the file (it can be swapped after the check, see `seal`).
pub(crate) fn download(url: &str, file: &Path, max_time: &str) -> Result<Vec<u8>, String> {
    if !trusted_url(url) {
        return Err(untrusted(url));
    }
    let mut curl = crate::spawn::command(system("curl"));
    curl.args(["-fsSL", "--max-time", max_time]).args(curl_flags(url)).args(CURL_LANDED).arg("-o").arg(file).arg(url);
    let out = curl.stdin(Stdio::null()).output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("{url}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    landed(&out.stdout)
        .and_then(|_| {
            let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
            signed(url, &bytes)?;
            Ok(bytes)
        })
        .inspect_err(|_| {
            let _ = std::fs::remove_file(file);
        })
}

/// Writes checked `bytes` to a new file in a new folder under `parent` (the
/// app home: private to this user), so what runs is exactly what was checked,
/// not a path someone could have swapped. Returns the folder (remove it when
/// done) and the file.
pub(crate) fn seal(parent: &Path, bytes: &[u8], name: &str) -> Result<(PathBuf, PathBuf), String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = parent.join(format!("sealed-{}-{nanos}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // Fails if the name exists, and never follows a planted link.
    builder.create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(name);
    let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&file).and_then(|mut f| std::io::Write::write_all(&mut f, bytes));
    match written {
        Ok(()) => Ok((dir, file)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(format!("{}: {e}", file.display()))
        }
    }
}

/// The public half of the key CI signs every app and core release file with (key id 29957E064EAB76C4).
const PUBLIC_KEY: &str = "RWTEdqtOBn6VKQ/x9x3qWk/gIWUb/I1OlZXsTlHyrkp9Yq30qBki+tdt";

/// Ok if `<url>.minisig` is the release key's signature of `bytes`. A test site
/// (the unit tests' loopback stub, or a releases URL override in debug and
/// test builds) is not signed.
fn signed(url: &str, bytes: &[u8]) -> Result<(), String> {
    let overrides = [env_url("H0X_ADE_RELEASES_URL"), env_url("H0X_CORE_RELEASES_URL")];
    if loopback_stub(url) || under_override(url, overrides) {
        return Ok(());
    }
    // A signature that does not download is refused below as missing.
    let sig = fetch(&format!("{url}.minisig")).ok();
    let name = url.rsplit('/').next().unwrap_or(url);
    verify(bytes, sig.as_deref(), PUBLIC_KEY, name).map_err(|why| format!("{url}: not signed by the HyperOx release key: {why}"))
}

/// `url` is on one of the overridden releases sites.
fn under_override(url: &str, overrides: [Option<String>; 2]) -> bool {
    overrides.into_iter().flatten().any(|base| url.starts_with(&format!("{base}/")))
}

/// Checks `bytes` against the minisign signature `sig` by `key` (prehashed signatures only),
/// made for the release file `name` (its trusted comment's `file:`, so one signed file
/// cannot stand in for another).
fn verify(bytes: &[u8], sig: Option<&[u8]>, key: &str, name: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key).map_err(|e| e.to_string())?;
    let text = sig.ok_or("it has no signature")?;
    let signature = std::str::from_utf8(text)
        .map_err(|e| e.to_string())
        .and_then(|text| minisign_verify::Signature::decode(text).map_err(|e| e.to_string()))
        .map_err(|e| format!("its signature is unreadable: {e}"))?;
    key.verify(bytes, &signature, false).map_err(|e| format!("its signature does not match: {e}"))?;
    let file_field = format!("file:{name}");
    if signature.trusted_comment().split('\t').any(|f| f == file_field) {
        Ok(())
    } else {
        Err(format!("its signature is for another file, not {name}"))
    }
}

fn releases() -> String {
    env_url("H0X_ADE_RELEASES_URL").unwrap_or_else(|| "https://github.com/vjk7989/h0x-ADE-app/releases".into())
}

pub(crate) fn core_releases() -> String {
    env_url("H0X_CORE_RELEASES_URL").unwrap_or_else(|| "https://github.com/vjk7989/h0x-ADE-core/releases".into())
}

/// The version downloaded and waiting for a restart.
static READY: Mutex<Option<String>> = Mutex::new(None);

pub fn ready() -> Option<String> {
    READY.lock().unwrap().clone()
}

/// The core the app runs, from its `server.status`, and a newer release waiting.
#[derive(Clone, Default, PartialEq, serde::Serialize)]
pub struct CoreInfo {
    pub version: Option<String>,
    /// A package manager that owns it (homebrew, nix, mise): never updated by the app.
    pub managed: Option<String>,
    pub ready: Option<String>,
    pub updating: bool,
    pub error: Option<String>,
}

static CORE: Mutex<CoreInfo> = Mutex::new(CoreInfo { version: None, managed: None, ready: None, updating: false, error: None });
/// When the last check succeeded (unix seconds); a failed check clears it.
static CHECKED: Mutex<Option<u64>> = Mutex::new(None);
/// "Check now" wakes the checker early.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
/// The checker runs (a release, or pointed at a test server), and is looking right now.
static RUNNING: AtomicBool = AtomicBool::new(false);
static CHECKING: AtomicBool = AtomicBool::new(false);
/// "Check for updates automatically" (settings.json `check_updates`): when off,
/// only "Check now" looks.
static AUTO: AtomicBool = AtomicBool::new(true);

pub fn set_auto(on: bool) {
    AUTO.store(on, Ordering::Relaxed);
}

/// Whether the background checker looks this round: on its schedule only when
/// automatic checks are on; always when the person asked.
fn due(auto: bool, asked: bool) -> bool {
    auto || asked
}

pub fn core() -> CoreInfo {
    CORE.lock().unwrap().clone()
}

/// What the connected core says about itself (on connect and every reconnect).
pub fn set_core(version: Option<String>, managed: Option<String>) {
    let mut c = CORE.lock().unwrap();
    if let (Some(have), Some(ready)) = (&version, &c.ready)
        && !newer(ready, have)
    {
        c.ready = None;
    }
    c.version = version;
    c.managed = managed;
}

fn checks_app() -> Option<PathBuf> {
    root().filter(|_| checks_core())
}

/// A release build checks the core; a source build only when pointed at a test release server.
fn checks_core() -> bool {
    option_env!("H0X_ADE_RELEASE").is_some() || env_url("H0X_ADE_RELEASES_URL").is_some()
}

/// Everything `status` reads that can change while the app runs: the poll
/// rebuilds the status only when this moves.
pub type StatusKey = (Option<String>, CoreInfo, Option<u64>, bool, bool, u64);

pub fn status_key() -> StatusKey {
    (
        ready(),
        core(),
        *CHECKED.lock().unwrap(),
        RUNNING.load(Ordering::Relaxed),
        CHECKING.load(Ordering::Relaxed),
        crate::update_notes::generation(),
    )
}

/// For the settings screen: both programs, what's waiting, when we last looked.
pub fn status() -> serde_json::Value {
    serde_json::json!({
        "app": { "version": env!("CARGO_PKG_VERSION"), "ready": ready(), "updates": checks_app().is_some() },
        "notes": crate::update_notes::notes(ready().as_deref()),
        "core": core(),
        "checked": *CHECKED.lock().unwrap(),
        "checks": RUNNING.load(Ordering::Relaxed),
        "checking": CHECKING.load(Ordering::Relaxed),
    })
}

pub fn check_now() {
    if RUNNING.load(Ordering::Relaxed) {
        CHECKING.store(true, Ordering::Relaxed);
    }
    *WAKE.0.lock().unwrap() = true;
    WAKE.1.notify_all();
}

/// Waits; true when "Check now" cut it short.
fn sleep(d: Duration) -> bool {
    let woken = WAKE.0.lock().unwrap();
    let (mut woken, _) = WAKE.1.wait_timeout_while(woken, d, |w| !*w).unwrap();
    std::mem::take(&mut *woken)
}

/// Starts checking in the background: the app if it's an installed release, the core too.
pub fn start() {
    let app = checks_app();
    let core = checks_core();
    if app.is_none() && !core {
        tracing::info!("a build from source: no updates");
        return;
    }
    RUNNING.store(true, Ordering::Relaxed);
    CHECKING.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        let mut asked = sleep(Duration::from_secs(10));
        loop {
            if !due(AUTO.load(Ordering::Relaxed), asked) {
                CHECKING.store(false, Ordering::Relaxed);
                asked = sleep(EVERY);
                continue;
            }
            CHECKING.store(true, Ordering::Relaxed);
            let mut wait = EVERY;
            let mut succeeded = true;
            if let Some(root) = &app {
                match check(root) {
                    Ok(found) => {
                        if found.is_some() {
                            *READY.lock().unwrap() = found;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("update check: {e}");
                        wait = RETRY;
                        succeeded = false;
                    }
                }
            }
            if core && let Err(e) = check_core() {
                tracing::warn!("core update check: {e}");
                wait = RETRY;
                succeeded = false;
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            *CHECKED.lock().unwrap() = checked_at(succeeded, now);
            CHECKING.store(false, Ordering::Relaxed);
            asked = sleep(wait);
        }
    });
}

fn checked_at(succeeded: bool, now: u64) -> Option<u64> {
    succeeded.then_some(now)
}

/// Looks for a newer core of this app's generation.
fn check_core() -> Result<(), String> {
    let (have, managed) = {
        let c = CORE.lock().unwrap();
        (c.version.clone(), c.managed.clone())
    };
    let Some(have) = have else { return Ok(()) };
    if managed.is_some() {
        return Ok(());
    }
    let latest = latest_core()?;
    let same = latest.generation == Some(crate::core_link::GENERATION);
    CORE.lock().unwrap().ready = (same && newer(&latest.version, &have)).then_some(latest.version);
    Ok(())
}

#[derive(serde::Deserialize)]
struct CoreRelease {
    version: String,
    generation: Option<u64>,
}

fn latest_core() -> Result<CoreRelease, String> {
    let text = curl_text(&format!("{}/latest/download/latest.json", core_releases()))?;
    serde_json::from_str(&text).map_err(|e| format!("core latest.json: {e}"))
}

fn after_update_target<'a>(have: &str, latest: Option<&'a CoreRelease>, generation: u64, core_min: &'a str) -> &'a str {
    match latest {
        Some(latest) if latest.generation == Some(generation) && newer(&latest.version, have) => &latest.version,
        _ => core_min,
    }
}

/// `h0x-core update` (terminals keep running; the app reconnects when the core restarts).
pub fn update_core() {
    {
        let mut c = CORE.lock().unwrap();
        if c.updating {
            return;
        }
        c.updating = true;
        c.error = None;
    }
    std::thread::spawn(|| {
        let result = crate::core_bin::update();
        let mut c = CORE.lock().unwrap();
        c.updating = false;
        match result {
            Ok(now) => {
                c.version = Some(now.trim_start_matches("h0x-core ").to_owned());
                c.ready = None;
            }
            Err(e) => {
                c.error = Some(e);
            }
        }
    });
}

/// Downloads the latest release if it's newer. Ok(Some): a version is ready.
fn check(root: &Path) -> Result<Option<String>, String> {
    check_from(root, &releases())
}

fn check_from(root: &Path, releases: &str) -> Result<Option<String>, String> {
    let path = "latest/download/latest.json";
    let text = curl_text(&format!("{releases}/{path}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let version = manifest["version"].as_str().ok_or(format!("{path}: no version"))?;
    if !newer(version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    // Notes are optional: a missing feed must never hold up the update.
    if !crate::update_notes::has(version)
        && let Ok(text) = curl_text(&format!("{releases}/download/v{version}/changelog.md"))
    {
        crate::update_notes::cache(&text);
    }
    let staged = beside(root, ".new");
    if staged_version(root).as_deref() == Some(version) {
        return Ok(Some(version.to_owned()));
    }
    let entry = &manifest["targets"][target()];
    let (Some(url), Some(sha)) = (entry["url"].as_str(), entry["sha256"].as_str()) else {
        return Ok(None);
    };
    tracing::info!("downloading HyperOx {version}");
    let work = beside(root, ".download");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = unpack(url, sha, &work).and_then(|unpacked| {
        if version_of(&unpacked).as_deref() != Some(version) {
            return Err(format!("the download doesn't run as {version}"));
        }
        let _ = std::fs::remove_file(beside(root, ".new.json"));
        let _ = std::fs::remove_dir_all(&staged);
        std::fs::rename(&unpacked, &staged).map_err(|e| format!("{}: {e}", staged.display()))?;
        // Outside the bundle: writing a receipt must not change its signature.
        std::fs::write(beside(root, ".new.json"), serde_json::json!({ "version": version }).to_string()).map_err(|e| e.to_string())
    });
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| Some(version.to_owned()))
}

/// Downloads, checks and unpacks a release into `work`; returns the app inside.
fn unpack(url: &str, sha: &str, work: &Path) -> Result<PathBuf, String> {
    let tarball = work.join("app.tar.gz");
    let bytes = download(url, &tarball, "600")?;
    if sha256(&bytes) != sha.to_ascii_lowercase() {
        return Err("the download is damaged (checksum mismatch)".into());
    }
    let home = crate::mods::ensure_app_dir().map_err(|e| format!("{}: {e}", crate::mods::app_dir().display()))?;
    let (sealed_dir, sealed) = seal(&home, &bytes, "app.tar.gz")?;
    let extracted = run(crate::spawn::command(system("tar")).arg("-xzf").arg(&sealed).arg("-C").arg(work));
    let _ = std::fs::remove_dir_all(&sealed_dir);
    extracted?;
    let app = work.join(PACKED);
    if cfg!(target_os = "macos") {
        // Authority is absent at -dv, even for a Developer ID release.
        let signature = Command::new("/usr/bin/codesign").arg("-dvv").arg(&app).output().map_err(|e| format!("codesign: {e}"))?;
        prepare_macos_bundle(&app, &signature)?;
    }
    Ok(app)
}

fn prepare_macos_bundle(app: &Path, signature: &std::process::Output) -> Result<(), String> {
    if release_signature(signature.status.success(), &String::from_utf8_lossy(&signature.stderr))? {
        run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(app))?;
    } else {
        // Only known dev builds may change the plist and lose their signature.
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
        let plist = app.join("Contents/Info.plist");
        run(Command::new("/usr/libexec/PlistBuddy").arg("-c").arg(format!("Add :LSEnvironment:PATH string {path}")).arg(&plist))?;
        run(Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(app))?;
    }
    Ok(())
}

fn release_signature(success: bool, signature: &str) -> Result<bool, String> {
    if success && signature.lines().any(|line| line.starts_with("Authority=Developer ID Application:")) {
        Ok(true)
    } else if (success && signature.lines().any(|line| line == "Signature=adhoc"))
        || (!success && signature.lines().any(|line| line.ends_with(": code object is not signed at all")))
    {
        Ok(false)
    } else {
        Err(format!("cannot determine app signature: {}", signature.trim()))
    }
}

/// curl and tar: on Windows its own (System32), since Git's tar, often first
/// on PATH, takes `C:\...` for a remote host.
pub fn system(name: &str) -> PathBuf {
    match std::env::var_os("SystemRoot").filter(|_| cfg!(windows)) {
        Some(root) => PathBuf::from(root).join("System32").join(format!("{name}.exe")),
        None => PathBuf::from(name),
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in sha2::Sha256::digest(bytes) {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// A release file's text, refused unless it is signed (see `signed`).
fn curl_text(url: &str) -> Result<String, String> {
    let bytes = fetch(url)?;
    signed(url, &bytes)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// A trusted `url`'s body, giving up after 30 seconds.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if !trusted_url(url) {
        return Err(untrusted(url));
    }
    let mut curl = crate::spawn::command(system("curl"));
    curl.args(["-fsSL", "--max-time", "30"]).args(curl_flags(url)).args(CURL_LANDED).arg(url);
    let out = curl.stdin(Stdio::null()).output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("{url}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(landed(&out.stdout)?.to_vec())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{:?}: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn exe_in(app: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        app.join("Contents/MacOS/HyperOx")
    } else {
        app.join(format!("h0x-ade{}", std::env::consts::EXE_SUFFIX))
    }
}

/// What `h0x-ade --version` says in this folder or bundle (`0.2.0`).
fn version_of(app: &Path) -> Option<String> {
    let out = crate::spawn::command(exe_in(app)).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return None;
    }
    text.trim().strip_prefix("h0x-ade ").map(str::to_owned)
}

/// `a` is a later version than `b` (semver; `0.2.0-rc.1` comes before `0.2.0`).
pub(crate) fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, Option<&str>) {
        let (core, pre) = v.trim_start_matches('v').split_once('-').map_or((v.trim_start_matches('v'), None), |(c, p)| (c, Some(p)));
        (core.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
    }
    let ((ca, pa), (cb, pb)) = (parts(a), parts(b));
    match ca.cmp(&cb) {
        std::cmp::Ordering::Equal => match (pa, pb) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
        o => o == std::cmp::Ordering::Greater,
    }
}

/// A receipt is published only after the archive and its executable were verified.
/// Legacy stages without one are left for the normal checker to download again.
fn staged_version(root: &Path) -> Option<String> {
    let staged = beside(root, ".new");
    let receipt = beside(root, ".new.json");
    let version = std::fs::read(&receipt).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["version"].as_str().map(str::to_owned));
    let version = version?;
    if version_of(&staged).as_deref() != Some(&version) {
        tracing::warn!("update: staged executable does not match its verified manifest; discarding it");
        let _ = std::fs::remove_file(receipt);
        let _ = std::fs::remove_dir_all(staged);
        return None;
    }
    Some(version)
}

fn startup_target<'a>(running: &str, staged: Option<&'a str>, relaunched: Option<&str>) -> Option<&'a str> {
    if relaunched.is_some() {
        return None;
    }
    staged.filter(|version| newer(version, running))
}

/// Recover a staged hotfix even when the previous window never quit cleanly.
/// The inherited expected version allows only one automatic relaunch.
pub fn apply_on_startup() -> bool {
    let expected = std::env::var("H0X_ADE_UPDATE_EXPECTED").ok();
    if let Some(expected) = &expected {
        // SAFETY: startup runs before any app threads. Do not leak the guard
        // into the core's environment and every terminal it later starts.
        unsafe { std::env::remove_var("H0X_ADE_UPDATE_EXPECTED") };
        if expected != env!("CARGO_PKG_VERSION") {
            tracing::warn!("update: relaunched as {}, expected {expected}; not retrying", env!("CARGO_PKG_VERSION"));
        }
        return false;
    }
    let Some(root) = checks_app() else { return false };
    let staged = staged_version(&root);
    let Some(version) = startup_target(env!("CARGO_PKG_VERSION"), staged.as_deref(), expected.as_deref()) else { return false };
    if !swap(&root) {
        return false;
    }
    relaunch_expected(Some(version))
}

/// Puts the downloaded version in place of this one. The running app keeps
/// its files open, so it can go on until it quits. True if it swapped.
/// Windows lets a running program's folder be renamed, not deleted: the old
/// one goes on the next start (`after_update`).
pub fn apply() -> bool {
    let Some(root) = root() else { return false };
    let staged = staged_version(&root);
    if ready().as_deref() != staged.as_deref() || !staged.as_deref().is_some_and(|version| newer(version, env!("CARGO_PKG_VERSION"))) {
        return false;
    }
    swap(&root)
}

fn swap(root: &Path) -> bool {
    let staged = beside(root, ".new");
    let old = beside(root, ".old");
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = std::fs::rename(root, &old) {
        tracing::warn!("update: cannot move {} aside: {e}", root.display());
        return false;
    }
    if let Err(e) = std::fs::rename(&staged, root) {
        tracing::warn!("update: cannot put the new version in place: {e}");
        if let Err(rollback) = std::fs::rename(&old, root) {
            tracing::error!("update: cannot restore the previous version: {rollback}");
        }
        return false;
    }
    let _ = std::fs::remove_file(beside(root, ".new.json"));
    *READY.lock().unwrap() = None;
    let _ = std::fs::write(marker(), "");
    true
}

/// Opens the app again once this process has exited (after `apply`).
pub fn relaunch() {
    relaunch_expected(None);
}

/// `text` as a PowerShell string: it ends at any of the five quote characters
/// (' and U+2018..U+201B), each of which a doubled copy escapes.
#[cfg(any(windows, test))]
fn ps_quote(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(any(windows, test))]
fn relaunch_script(pid: u32, exe: &Path) -> String {
    format!("Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Process {}", ps_quote(&exe.to_string_lossy()))
}

#[cfg(windows)]
fn relaunch_expected(expected: Option<&str>) -> bool {
    let Some(root) = root() else { return false };
    // The system's own PowerShell: one earlier on PATH must not run here.
    let Some(system_root) = std::env::var_os("SystemRoot") else { return false };
    let script = relaunch_script(std::process::id(), &exe_in(&root));
    let mut command = crate::spawn::command(PathBuf::from(system_root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
    if let Some(expected) = expected {
        command.env("H0X_ADE_UPDATE_EXPECTED", expected);
    }
    let spawned = command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("update: cannot reopen the app: {e}");
            false
        }
    }
}

/// Opens the app again once this process has exited (after `apply`).
#[cfg(unix)]
fn relaunch_expected(expected: Option<&str>) -> bool {
    let Some(root) = root() else { return false };
    // LaunchServices need not preserve the startup guard or an isolated home.
    let open = if cfg!(target_os = "macos") {
        if expected.is_some() { "exec \"$1/Contents/MacOS/HyperOx\"" } else { "open \"$1\"" }
    } else {
        "exec \"$1/h0x-ade\""
    };
    let script = format!("while kill -0 \"$0\" 2>/dev/null; do sleep 0.1; done; {open}");
    let spawned = {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        if let Some(expected) = expected {
            command.env("H0X_ADE_UPDATE_EXPECTED", expected);
        }
        command
            .arg("-c")
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    };
    match spawned {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("update: cannot reopen the app: {e}");
            false
        }
    }
}

/// Left by `apply`: the next start brings the core up to date too.
fn marker() -> PathBuf {
    crate::mods::app_dir().join(".updated")
}

/// The first start after an app update brings the core to the latest release
/// of this app's generation (`h0x-core update --version`, terminals kept). If the
/// release check fails or offers another generation, keep the minimum pin.
/// A core a package manager owns is left alone (h0x-core says so).
pub fn after_update() {
    if std::fs::remove_file(marker()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let have = crate::core_bin::version(&crate::core_bin::path()).unwrap_or_default();
        CORE.lock().unwrap().updating = true;
        let latest = latest_core().map_err(|e| tracing::warn!("core release check after app update: {e}")).ok();
        let target = after_update_target(have.trim_start_matches("h0x-core "), latest.as_ref(), crate::core_link::GENERATION, crate::core_link::core_min());
        let result = crate::core_bin::update_to(Some(target));
        let mut core = CORE.lock().unwrap();
        core.updating = false;
        match result {
            Ok(now) => {
                if let Some(root) = root() {
                    let _ = std::fs::remove_dir_all(beside(&root, ".old"));
                }
                tracing::info!("h0x-core is {now}");
                core.version = Some(now.trim_start_matches("h0x-core ").to_owned());
                core.error = None;
            }
            Err(e) => {
                tracing::warn!("h0x-core update: {e}");
                core.error = Some(e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_check_clears_success_and_recovery_records_a_new_time() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // An isolated release server drops the first connection, then recovers.
        // No environment changes, app window, core, or installed files are involved.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for recover in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET /latest/download/latest.json HTTP/1.1\r\n"));
                if recover {
                    let body = format!(r#"{{"version":"{}"}}"#, env!("CARGO_PKG_VERSION"));
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            }
        });
        let home = std::env::temp_dir().join(format!("h0x-ade-offline-check-{}", std::process::id()));
        let mut checked = checked_at(true, 100);
        assert_eq!(checked, Some(100));
        let failed = check_from(&home, &url);
        assert!(failed.is_err(), "a dropped connection must fail: {failed:?}");
        checked = checked_at(failed.is_ok(), 200);
        assert_eq!(checked, None);
        let recovered = check_from(&home, &url);
        assert_eq!(recovered, Ok(None));
        checked = checked_at(recovered.is_ok(), 300);
        assert_eq!(checked, Some(300));
        server.join().unwrap();
    }

    #[test]
    fn only_https_github_urls_are_trusted() {
        for url in [
            "https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/latest.json",
            "https://github.com",
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1?sp=r",
        ] {
            assert!(trusted_url(url), "{url}");
        }
        for url in [
            "",
            " ",
            "https://",
            "http://github.com/vjk7989/h0x-ADE-app/releases",
            "https://example.com/releases",
            "https://github.com.evil.com/releases",
            "https://evil.com/github.com/releases",
            "https://user@github.com/releases",
            "https://github.com:443/releases",
            "https://github.com:8443/releases",
            "https://api.github.com/repos",
            "https://GitHub.com/releases",
            "HTTPS://github.com/releases",
            " https://github.com/releases",
            r"https://github.com\@evil.com/",
            "file:///C:/Users/me/app.tar.gz",
            r"C:\Users\me\app.tar.gz",
            "ftp://github.com/releases",
            // The unit tests' loopback stub is exactly `http://127.0.0.1:<port>`.
            "http://127.0.0.1.evil.com/",
            "http://localhost/",
            "http://127.0.0.1@evil.com/",
            "http://127.0.0.1/",
            "http://127.0.0.1:/",
            "http://127.0.0.1:1@evil.com/",
            "http://127.0.0.1:99999/",
            "https://127.0.0.1:1/",
        ] {
            assert!(!trusted_url(url), "{url:?}");
        }
        assert_eq!(CURL_HTTPS, ["--proto", "=https", "--proto-redir", "=https"]);
        assert_eq!(curl_flags("https://github.com/x"), CURL_HTTPS);
        assert_eq!(curl_flags("http://127.0.0.1.evil.com/"), CURL_HTTPS);
        // cfg(test) only: the loopback stub may be plain http, but never redirect off https.
        for url in ["http://127.0.0.1:1", "http://127.0.0.1:8080/latest/download/latest.json"] {
            assert!(trusted_url(url), "{url}");
            assert_eq!(curl_flags(url), ["--proto", "=https,http", "--proto-redir", "=https"]);
        }
    }

    #[test]
    fn untrusted_urls_are_refused_before_curl_runs() {
        let file = std::env::temp_dir().join(format!("h0x-ade-untrusted-{}", std::process::id()));
        for url in ["", "http://github.com/x", "https://github.com@evil.com/x", "https://github.com:1/x"] {
            assert_eq!(curl_text(url), Err(format!("{url}: not an https GitHub URL")));
            assert_eq!(download(url, &file, "1"), Err(format!("{url}: not an https GitHub URL")));
            assert_eq!(unpack(url, "", &file).unwrap_err(), format!("{url}: not an https GitHub URL"));
        }
        assert!(!file.exists());
    }

    /// A throwaway test key pair (not the release key), its signature of
    /// `PAYLOAD`, and the same payload signed by a second throwaway key; made
    /// with rsign2 (`generate -W`, `sign -W -t`), trusted comments the way
    /// `minisign -S` writes them.
    const TEST_KEY: &str = "RWSdBj3+m1kau24wxYnVraQxNX7fJSNwEU8cGRBq0f/kLy3T7o02KkXn";
    const PAYLOAD: &[u8] = b"h0x-core test release\n";
    const NAME: &str = "h0x-ade-linux-x86_64.tar.gz";
    const SIG: &[u8] = b"untrusted comment: signature from rsign secret key
RUSdBj3+m1kau6scpo4XTeSCqu0PwKULSaApRgvM+/R9RCh20ab58L4jXvM9uh70A7tsOeIF1jDHl4Ch7518nSr1zu2TDwzEaA4=
trusted comment: timestamp:1\tfile:h0x-ade-linux-x86_64.tar.gz\thashed
FcW0YJpp95DgHgZOwtuZhHgRcakK8Nl78hPlfsAKdi7XIDIQPKqgggzZRyHRdYw/r0kESrOGH0czLnUH9XJDAQ==
";
    /// The same payload and key, signed as `other.json`.
    const OTHER_FILE_SIG: &[u8] = b"untrusted comment: signature from rsign secret key
RUSdBj3+m1kau3QihUJUf+OsaCwPgkweY7RF8K/MIFS7iw+5ecagISnHW5/uLpyPa6fOEebWMehn+AD8NEwLp53f7iUv0zjXGw8=
trusted comment: timestamp:1\tfile:other.json\thashed
8lddDBJQHhcebqIZK4ImVgWxj59QJalPUfEcK8Us9XmBfDEf5Yse5HH2tXST2QsmgHBg8amXtME4togIKoREBA==
";
    const OTHER_KEY_SIG: &[u8] = b"untrusted comment: signature from rsign secret key
RURaRIrUmDKGf+VfuMRCB6G+7YmOZuDtq2kGvfhr9JFuy2kWFMZ6J+L3UogLdr/J7fvVdNbKYfGvxVj6sSbZQmp4hSeTA28H6g8=
trusted comment: test
ZJhj3MtbsDPyt278bwB0BcVRB5ZoKqA6VCCZ5s51lq49BobYTBUuzlJqofyqGhaBwWB6iLyCd1cVXXQhXnGlBg==
";

    fn refused(bytes: &[u8], sig: Option<&[u8]>, key: &str, why: &str) {
        let err = verify(bytes, sig, key, NAME).unwrap_err();
        assert!(err.starts_with(why), "{why}: {err}");
    }

    #[test]
    fn a_file_signed_by_the_key_is_kept() {
        assert_eq!(verify(PAYLOAD, Some(SIG), TEST_KEY, NAME), Ok(()));
    }

    #[test]
    fn a_signature_made_for_another_file_is_refused() {
        refused(PAYLOAD, Some(OTHER_FILE_SIG), TEST_KEY, "its signature is for another file, not h0x-ade-linux-x86_64.tar.gz");
        // `file:` must name the release file whole, not just begin with its name.
        assert_eq!(
            verify(PAYLOAD, Some(SIG), TEST_KEY, "h0x-ade-linux-x86_64.tar"),
            Err("its signature is for another file, not h0x-ade-linux-x86_64.tar".to_owned())
        );
    }

    #[test]
    fn a_tampered_file_is_refused() {
        refused(b"h0x-core test releasE\n", Some(SIG), TEST_KEY, "its signature does not match");
        refused(b"", Some(SIG), TEST_KEY, "its signature does not match");
    }

    #[test]
    fn a_missing_or_unreadable_signature_is_refused() {
        refused(PAYLOAD, None, TEST_KEY, "it has no signature");
        refused(PAYLOAD, Some(b""), TEST_KEY, "its signature is unreadable");
        refused(PAYLOAD, Some(b"\xff\xfe"), TEST_KEY, "its signature is unreadable");
    }

    #[test]
    fn a_signature_by_another_key_is_refused() {
        refused(PAYLOAD, Some(OTHER_KEY_SIG), TEST_KEY, "its signature does not match");
        // The test signature does not pass for the embedded release key either.
        refused(PAYLOAD, Some(SIG), PUBLIC_KEY, "its signature does not match");
    }

    #[test]
    fn a_release_file_without_a_signature_is_refused_before_curl_runs() {
        // `<url>.minisig` is not a trusted URL here, so no request is made: the signature is missing.
        let url = "https://example.com/h0x-ade-linux-x86_64.tar.gz";
        assert_eq!(signed(url, PAYLOAD), Err(format!("{url}: not signed by the HyperOx release key: it has no signature")));
    }

    #[test]
    fn only_a_test_site_skips_the_signature() {
        // The unit tests' loopback stub (cfg(test) only) is not signed.
        assert_eq!(signed("http://127.0.0.1:1/latest/download/latest.json", b"unsigned"), Ok(()));
        let release = "https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/latest.json";
        assert!(!under_override(release, [None, None]));
        let fork = || Some("https://github.com/someone/fork/releases".to_owned());
        for overrides in [[fork(), None], [None, fork()]] {
            assert!(under_override("https://github.com/someone/fork/releases/latest/download/install.sh", overrides.clone()));
            assert!(!under_override(release, overrides.clone()), "an override covers its own site only");
            assert!(!under_override("https://github.com/someone/fork/releases-evil/x", overrides.clone()));
            assert!(!under_override("https://github.com/someone/fork/releases", overrides));
        }
        // A release build never honours an override, so nothing it downloads skips the signature.
        assert!(!under_override("https://github.com/someone/fork/releases/latest/download/install.sh", [overridden(false, fork()), overridden(false, fork())]));
    }

    #[test]
    fn the_url_curl_landed_on_must_be_trusted_too() {
        assert_eq!(CURL_LANDED, ["--max-redirs", "5", "--write-out", r"\n%{url_effective}"]);
        // A release file redirects to release-assets; the body may hold newlines itself.
        let ok = b"{\"a\":1}\nline two\n\nhttps://release-assets.githubusercontent.com/github-production-release-asset/1?sp=r";
        assert_eq!(landed(ok), Ok(&b"{\"a\":1}\nline two\n"[..]));
        // With `-o FILE` the body is in the file and stdout is only the newline and URL.
        assert_eq!(landed(b"\nhttps://github.com/vjk7989/h0x-ADE-app/releases/download/v0.1.0/a.tar.gz"), Ok(&b""[..]));
        assert_eq!(landed(b"\nhttps://objects.githubusercontent.com/x"), Ok(&b""[..]));
        for (stdout, url) in [
            (&b"body\nhttps://evil.com/a.tar.gz"[..], "https://evil.com/a.tar.gz"),
            (&b"\nhttp://github.com/x"[..], "http://github.com/x"),
            (&b"\nhttps://github.com.evil.com/x"[..], "https://github.com.evil.com/x"),
            (&b"\nhttps://github.com@evil.com/x"[..], "https://github.com@evil.com/x"),
            (&b"\nhttps://github.com:8443/x"[..], "https://github.com:8443/x"),
            (&b"https://github.com/x\n"[..], ""),
            (&b"\n"[..], ""),
            (&b"\n\xff"[..], "\u{fffd}"),
        ] {
            assert_eq!(landed(stdout), Err(format!("{url}: not an https GitHub URL")), "{stdout:?}");
        }
        for stdout in [&b""[..], &b"https://github.com/x"[..], &b"body without a final url"[..]] {
            assert_eq!(landed(stdout), Err("curl did not report the final URL".to_owned()), "{stdout:?}");
        }
    }

    #[test]
    fn env_overrides_count_only_in_debug_and_test_builds() {
        assert!(OVERRIDES, "unit tests are built with cfg(test)");
        let url = || Some("https://github.com/someone/fork/releases/".to_owned());
        assert_eq!(overridden(true, url()).as_deref(), Some("https://github.com/someone/fork/releases"));
        assert_eq!(overridden(false, url()), None, "a release build ignores the override");
        for value in [None, Some(String::new()), Some(" ".to_owned())] {
            assert_eq!(overridden(true, value.clone()), None);
            assert_eq!(overridden(false, value), None);
        }
    }

    #[test]
    fn trusted_hosts_match_exactly_without_lookalikes() {
        for url in [
            "https://github.com/",
            "https://github.com?x=1",
            "https://github.com#x",
            "https://objects.githubusercontent.com?sig=1",
            "https://release-assets.githubusercontent.com#frag",
        ] {
            assert!(trusted_url(url), "{url}");
        }
        for url in [
            "https://github.com./x",
            "https://objects.githubusercontent.com./x",
            "https://OBJECTS.githubusercontent.com/x",
            "https://xn--github-9za.com/x",
            "https://g\u{456}thub.com/x",
            "https://\u{ff47}ithub.com/x",
            "https://github.com%40evil.com/x",
            "https://evil.com%2F%40github.com/x",
            "https://evil.com%2F@github.com/x",
            "https://github.com:/x",
            "https:///github.com/x",
            "https://[::1]/x",
            "https://140.82.112.3/x",
            "https://github.com\t.evil.com/x",
            "https://github.com\n/x",
            "https://github.com\\evil.com/x",
            "https://github.com;evil.com/x",
        ] {
            assert!(!trusted_url(url), "{url:?}");
            assert_eq!(curl_text(url), Err(format!("{url}: not an https GitHub URL")));
        }
    }

    #[test]
    fn startup_applies_only_a_newer_version_and_never_relaunches_twice() {
        assert_eq!(startup_target("0.1.7", Some("0.1.8"), None), Some("0.1.8"));
        for staged in [None, Some("0.1.6"), Some("0.1.7")] {
            assert_eq!(startup_target("0.1.7", staged, None), None);
        }
        for expected in ["0.1.7", "0.1.8", "0.1.9"] {
            assert_eq!(startup_target("0.1.7", Some("0.1.8"), Some(expected)), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn stages_need_a_receipt_and_a_matching_executable() {
        use std::os::unix::fs::PermissionsExt;
        let work = std::env::temp_dir().join(format!("h0x-ade-update-{}", std::process::id()));
        let root = work.join(PACKED);
        let staged = beside(&root, ".new");
        let receipt = beside(&root, ".new.json");
        let exe = exe_in(&staged);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\necho h0x-ade 0.1.8\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(staged.is_dir()); // Legacy stages are left for the checker.
        std::fs::write(&receipt, r#"{"version":"0.1.8"}"#).unwrap();
        assert_eq!(staged_version(&root).as_deref(), Some("0.1.8"));
        std::fs::write(&receipt, r#"{"version":"0.1.9"}"#).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(!staged.exists());
        assert!(!receipt.exists());
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "corrupt").unwrap();
        std::fs::write(&receipt, r#"{"version":"0.1.8"}"#).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(!staged.exists());
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn signature_inspection_fails_closed() {
        let release = include_str!("../scripts/fixtures/codesign/developer-id.txt");
        let adhoc = include_str!("../scripts/fixtures/codesign/adhoc.txt");
        let unsigned = include_str!("../scripts/fixtures/codesign/unsigned.txt");
        assert_eq!(release_signature(true, release), Ok(true));
        assert_eq!(release_signature(true, adhoc), Ok(false));
        assert_eq!(release_signature(false, unsigned), Ok(false));
        for (success, output) in [(false, release), (false, adhoc), (true, unsigned), (true, ""), (false, "codesign: internal error"), (true, "Authority=Apple Development: Someone")] {
            assert!(release_signature(success, output).unwrap_err().starts_with("cannot determine app signature:"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_signature_does_not_modify_the_bundle() {
        use std::os::unix::process::ExitStatusExt;
        let work = std::env::temp_dir().join(format!("h0x-ade-unknown-signature-{}", std::process::id()));
        let plist = work.join("Contents/Info.plist");
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "unchanged plist").unwrap();
        let signature = std::process::Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"codesign: internal error".to_vec(),
        };
        assert_eq!(prepare_macos_bundle(&work, &signature), Err("cannot determine app signature: codesign: internal error".into()));
        assert_eq!(std::fs::read_to_string(plist).unwrap(), "unchanged plist");
        assert_eq!(std::fs::read_dir(work.join("Contents")).unwrap().count(), 1);
        std::fs::remove_dir_all(work).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "downloads the current release; runs real codesign on isolated bundles"]
    fn macos_unpack_preserves_release_signature_and_prepares_dev_builds() {
        let version = env!("CARGO_PKG_VERSION");
        let manifest: serde_json::Value = serde_json::from_str(&curl_text(&format!("{}/download/v{version}/latest.json", releases())).unwrap()).unwrap();
        let work = std::env::temp_dir().join(format!("h0x-ade-signature-test-{}", std::process::id()));
        let signed = work.join("signed");
        let reference = work.join("reference");
        std::fs::create_dir_all(&signed).unwrap();
        std::fs::create_dir_all(&reference).unwrap();
        let entry = &manifest["targets"][target()];
        let app = unpack(entry["url"].as_str().unwrap(), entry["sha256"].as_str().unwrap(), &signed).unwrap();
        let signature = Command::new("/usr/bin/codesign").arg("-dvv").arg(&app).output().unwrap();
        assert_eq!(release_signature(signature.status.success(), &String::from_utf8_lossy(&signature.stderr)), Ok(true));
        run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app)).unwrap();
        run(Command::new("tar").arg("-xzf").arg(signed.join("app.tar.gz")).arg("-C").arg(&reference)).unwrap();
        for file in ["Contents/Info.plist", "Contents/MacOS/HyperOx", "Contents/_CodeSignature/CodeResources"] {
            assert_eq!(std::fs::read(app.join(file)).unwrap(), std::fs::read(reference.join(PACKED).join(file)).unwrap(), "{file} was modified");
        }
        for kind in ["adhoc", "unsigned"] {
            let source = reference.join(PACKED);
            let mut codesign = Command::new("/usr/bin/codesign");
            if kind == "adhoc" {
                codesign.args(["--force", "--sign", "-"]);
            } else {
                codesign.arg("--remove-signature");
            }
            run(codesign.arg(&source)).unwrap();
            let archive = work.join(format!("{kind}.tar.gz"));
            run(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&reference).arg(PACKED)).unwrap();
            let dev = work.join(kind);
            std::fs::create_dir_all(&dev).unwrap();
            // unpack() downloads over https only: these local dev builds take its steps after the download.
            run(Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&dev)).unwrap();
            let app = dev.join(PACKED);
            let signature = Command::new("/usr/bin/codesign").arg("-dvv").arg(&app).output().unwrap();
            prepare_macos_bundle(&app, &signature).unwrap();
            let signature = Command::new("/usr/bin/codesign").arg("-dvv").arg(&app).output().unwrap();
            assert!(signature.status.success());
            let signature = String::from_utf8_lossy(&signature.stderr);
            assert_eq!(release_signature(true, &signature), Ok(false));
            assert!(signature.lines().any(|line| line == "Signature=adhoc"));
            run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app)).unwrap();
            let path = Command::new("/usr/libexec/PlistBuddy").args(["-c", "Print :LSEnvironment:PATH"]).arg(app.join("Contents/Info.plist")).output().unwrap();
            assert!(path.status.success());
            assert!(String::from_utf8_lossy(&path.stdout).contains("/opt/homebrew/bin"));
        }
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn versions_compare_like_semver() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(newer("1.0.0", "1.0.0-rc.1"));
        assert!(!newer("1.0.0-rc.1", "1.0.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn release_urls_treat_empty_as_unset_and_drop_a_trailing_slash() {
        // Only this test sets these variables inside the unit-test binary.
        for (key, read, default) in [
            ("H0X_ADE_RELEASES_URL", releases as fn() -> String, "https://github.com/vjk7989/h0x-ADE-app/releases"),
            ("H0X_CORE_RELEASES_URL", core_releases as fn() -> String, "https://github.com/vjk7989/h0x-ADE-core/releases"),
        ] {
            unsafe { std::env::set_var(key, "https://mirror.example/releases/") };
            assert_eq!(read(), "https://mirror.example/releases");
            for empty in ["", " ", "\t\n"] {
                unsafe { std::env::set_var(key, empty) };
                assert_eq!(read(), default, "{key}={empty:?} counts as unset");
            }
            unsafe { std::env::remove_var(key) };
            assert_eq!(read(), default);
        }
        if option_env!("H0X_ADE_RELEASE").is_none() {
            unsafe { std::env::set_var("H0X_ADE_RELEASES_URL", " ") };
            assert!(!checks_core(), "an empty test server URL does not turn checks on");
            assert!(checks_app().is_none());
            unsafe { std::env::set_var("H0X_ADE_RELEASES_URL", "https://mirror.example/releases") };
            assert!(checks_core());
            unsafe { std::env::remove_var("H0X_ADE_RELEASES_URL") };
            assert!(!checks_core());
        }
    }

    #[test]
    fn after_app_update_targets_latest_only_in_our_generation() {
        let release = |version: &str, generation| CoreRelease { version: version.into(), generation };
        let latest = release("0.1.13", Some(1));
        assert_eq!(after_update_target("0.1.12", Some(&latest), 1, "0.1.11"), "0.1.13");
        for have in ["0.1.13", "0.1.14"] {
            assert_eq!(after_update_target(have, Some(&latest), 1, "0.1.11"), "0.1.11");
        }
        for latest in [None, Some(release("0.2.0", Some(2))), Some(release("0.1.13", None))] {
            assert_eq!(after_update_target("0.1.12", latest.as_ref(), 1, "0.1.11"), "0.1.11");
        }
        // Every h0x-core release has `update --version`: the first one is bumped too.
        assert_eq!(after_update_target("0.1.0", Some(&latest), 1, "0.1.11"), "0.1.13");
        assert_eq!(after_update_target("0.1.0", None, 1, "0.1.11"), "0.1.11");
    }

    #[test]
    fn background_checks_follow_the_setting_but_asking_always_checks() {
        assert!(due(true, false));
        assert!(due(true, true));
        assert!(!due(false, false), "automatic checks off: nothing runs on the schedule");
        assert!(due(false, true), "Check now still works with automatic checks off");
    }

    #[test]
    fn a_sealed_copy_holds_the_checked_bytes_in_a_fresh_folder() {
        let parent = std::env::temp_dir().join(format!("h0x-ade-seal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        let checked = b"the bytes that were verified";
        let (dir, file) = seal(&parent, checked, "install.sh").unwrap();
        assert!(dir.starts_with(&parent) && file == dir.join("install.sh"));
        assert_eq!(std::fs::read(&file).unwrap(), checked);
        // Swapping the downloaded path afterwards changes nothing that runs.
        let downloaded = parent.join("download.sh");
        std::fs::write(&downloaded, checked).unwrap();
        std::fs::write(&downloaded, b"swapped").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), checked);
        let (other, _) = seal(&parent, b"", "empty").unwrap();
        assert_ne!(dir, other, "every copy gets its own folder");
        assert!(seal(&parent.join("missing").join("deeper"), checked, "x").is_err());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn relaunch_quotes_every_powershell_quote_character() {
        assert_eq!(ps_quote("plain"), "'plain'");
        assert_eq!(ps_quote(""), "''");
        for q in ['\'', '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}'] {
            assert_eq!(ps_quote(&format!("a{q}b")), format!("'a{q}{q}b'"), "U+{:04X}", q as u32);
        }
        let exe = Path::new("C:/Users/O\u{2019}Neil/My Apps/it's here/HyperOx.exe");
        assert_eq!(
            relaunch_script(42, exe),
            "Wait-Process -Id 42 -ErrorAction SilentlyContinue; Start-Process 'C:/Users/O\u{2019}\u{2019}Neil/My Apps/it''s here/HyperOx.exe'"
        );
    }

    #[test]
    fn sha256_is_hex() {
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
