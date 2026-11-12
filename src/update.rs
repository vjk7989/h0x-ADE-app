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
//! Each part reads its own signed `latest.json` (app, core and runtime each have a releases
//! repo; the older repo is the fallback when that one fails to fetch or verify). The core is
//! checked on the same clock: a newer core of the same generation is applied once no agent has
//! worked for two minutes (also after an app update), unless a package manager owns it. Within
//! a generation a newer core only adds. A runtime (`h0x` on PATH, not nix) updates itself the
//! same way with `h0x update --yes --json`; none installed, nothing happens.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    if loopback_stub(url) || under_override(url, overrides) || under_override(url, [env_url("H0X_RUNTIME_RELEASES_URL"), None]) {
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

/// Where a part's signed `latest.json` is read, in order: its own releases repo, then the older
/// one. A test site (`env`) is the only source.
fn sources(env: Option<String>, own: &str, older: &str) -> Vec<String> {
    env.map_or_else(|| vec![own.to_owned(), older.to_owned()], |url| vec![url])
}

fn releases() -> Vec<String> {
    sources(env_url("H0X_ADE_RELEASES_URL"), "https://github.com/buckleson/h0x-ADE-Releases/releases", "https://github.com/vjk7989/h0x-ADE-app/releases")
}

fn core_sources() -> Vec<String> {
    sources(env_url("H0X_CORE_RELEASES_URL"), "https://github.com/buckleson/h0x-ADE-Core-Releases/releases", "https://github.com/vjk7989/h0x-ADE-core/releases")
}

fn runtime_sources() -> Vec<String> {
    sources(env_url("H0X_RUNTIME_RELEASES_URL"), "https://github.com/buckleson/h0x-ADE-Runtime-Releases/releases", "https://github.com/vjk7989/h0x-ADE-Runtime/releases")
}

/// `f` on each source until one works (a fetch or signature failure moves on); the first
/// error if none does.
fn first_ok<T>(sources: &[String], mut f: impl FnMut(&str) -> Result<T, String>) -> Result<T, String> {
    let mut first = None;
    for source in sources {
        match f(source) {
            Ok(v) => return Ok(v),
            Err(e) => {
                tracing::warn!("{source}: {e}");
                first.get_or_insert(e);
            }
        }
    }
    Err(first.unwrap_or_else(|| "no release source".into()))
}

pub(crate) fn core_releases() -> String {
    env_url("H0X_CORE_RELEASES_URL").unwrap_or_else(|| "https://github.com/vjk7989/h0x-ADE-core/releases".into())
}

/// What the 2-minute idle gate waits on: when an agent last worked (unix seconds; 0 = never).
static LAST_WORKING: AtomicU64 = AtomicU64::new(0);
/// Core and runtime updates wait until no agent has worked for this long.
const IDLE: u64 = 2 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The window's tick: whether any agent is working now.
pub fn note_agents(working: bool) {
    if working {
        LAST_WORKING.store(unix_now(), Ordering::Relaxed);
    }
}

fn idle(last_working: u64, now: u64) -> bool {
    now.saturating_sub(last_working) >= IDLE
}

/// A line for the person about what an update did; the window shows each as a toast.
pub struct Notice {
    pub level: &'static str,
    pub text: String,
}

static NOTICES: Mutex<Vec<Notice>> = Mutex::new(Vec::new());

fn notify(level: &'static str, text: String) {
    NOTICES.lock().unwrap().push(Notice { level, text });
}

pub fn take_notices() -> Vec<Notice> {
    std::mem::take(&mut *NOTICES.lock().unwrap())
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

/// On a first run the schedule waits until the welcome screen is dismissed, so
/// no check goes out before the person has seen the update setting. "Check now"
/// is never held.
static HELD: AtomicBool = AtomicBool::new(false);

pub fn hold_until_welcomed() {
    HELD.store(true, Ordering::Relaxed);
}

/// The welcome screen was dismissed (shared key `frame.welcomed`).
pub fn welcomed() {
    HELD.store(false, Ordering::Relaxed);
}

/// Whether the schedule may look: automatic checks are on and no welcome is pending.
fn scheduled(auto: bool, held: bool) -> bool {
    auto && !held
}

/// Turning automatic checks back on checks now, not at the next hour; setting
/// the value it already has does nothing.
pub fn set_auto(on: bool) {
    if !AUTO.swap(on, Ordering::Relaxed) && on {
        wake();
    }
}

fn wake() {
    *WAKE.0.lock().unwrap() = true;
    WAKE.1.notify_all();
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
    wake();
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
    if core {
        std::thread::spawn(apply_when_idle);
    }
    std::thread::spawn(move || {
        let mut asked = sleep(Duration::from_secs(10));
        loop {
            if !due(scheduled(AUTO.load(Ordering::Relaxed), HELD.load(Ordering::Relaxed)), asked) {
                CHECKING.store(false, Ordering::Relaxed);
                // Held for the welcome: look again soon, so the first check follows its dismissal.
                asked = sleep(if HELD.load(Ordering::Relaxed) { Duration::from_secs(5) } else { EVERY });
                continue;
            }
            CHECKING.store(true, Ordering::Relaxed);
            let mut wait = EVERY;
            let mut succeeded = true;
            if let Some(root) = &app {
                match check(root) {
                    Ok(found) => {
                        crate::events::update_checked("app");
                        if let Some(version) = found {
                            if READY.lock().unwrap().as_deref() != Some(&version) {
                                crate::events::update_downloaded("app", &version);
                            }
                            *READY.lock().unwrap() = Some(version);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("update check: {e}");
                        crate::analytics::report("update_failed", "check", &e);
                        wait = RETRY;
                        succeeded = false;
                    }
                }
            }
            if core {
                match check_core() {
                    Ok(()) => crate::events::update_checked("core"),
                    Err(e) => {
                        tracing::warn!("core update check: {e}");
                        crate::analytics::report("core_update_failed", "check", &e);
                        wait = RETRY;
                        succeeded = false;
                    }
                }
                // Its problems are reported on their own; they never move the app and core status.
                let _ = check_runtime();
            }
            let now = unix_now();
            *CHECKED.lock().unwrap() = checked_at(succeeded, now);
            CHECKING.store(false, Ordering::Relaxed);
            asked = sleep(wait);
        }
    });
}

/// A newer core or runtime waits until no agent has worked for two minutes, then goes in.
/// Automatic checks off: nothing goes in on its own.
fn apply_when_idle() {
    loop {
        std::thread::sleep(Duration::from_secs(30));
        if !AUTO.load(Ordering::Relaxed) || !idle(LAST_WORKING.load(Ordering::Relaxed), unix_now()) {
            continue;
        }
        if core_waiting(&core()) {
            update_core();
        }
        update_runtime();
    }
}

/// A newer core is ready, nobody else owns it, none is going in and the last try didn't fail
/// (a failed one is tried again by hand).
fn core_waiting(core: &CoreInfo) -> bool {
    core.ready.is_some() && core.managed.is_none() && !core.updating && core.error.is_none()
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
    first_ok(&core_sources(), |releases| {
        let text = curl_text(&format!("{releases}/latest/download/latest.json"))?;
        serde_json::from_str(&text).map_err(|e| format!("core latest.json: {e}"))
    })
}

/// The runtime (`h0x` on PATH) and its newer release, waiting for the idle gate.
struct RuntimeInfo {
    ready: Option<String>,
    /// The `h0x` that was checked: the one an update runs.
    bin: Option<PathBuf>,
    /// A version whose update failed: not tried again until a newer one is out.
    failed: Option<String>,
    updating: bool,
}

static RUNTIME: Mutex<RuntimeInfo> = Mutex::new(RuntimeInfo { ready: None, bin: None, failed: None, updating: false });

/// What the runtime's own `h0x update --yes --json` prints, and how to find it. Not installed,
/// or owned by a package manager: left alone.
mod runtime {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;

    /// The one JSON line `h0x update --yes --json` prints.
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    pub struct Update {
        pub updated: bool,
        pub to: Option<String>,
        pub rolled_back: bool,
        pub error: Option<serde_json::Value>,
    }

    impl Update {
        /// Whether it failed, and whether the old runtime was put back.
        pub fn failure(&self) -> Option<(bool, String)> {
            let text = |v: &serde_json::Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            match (&self.error, self.rolled_back) {
                (Some(e), rolled_back) => Some((rolled_back, text(e))),
                (None, true) => Some((true, "rolled back".into())),
                (None, false) => None,
            }
        }
    }

    /// The last stdout line that is a JSON object (lines after it, such as warnings, are skipped).
    pub fn parse(stdout: &[u8]) -> Result<Update, String> {
        let text = String::from_utf8_lossy(stdout);
        if text.trim().is_empty() {
            return Err("h0x update printed nothing".into());
        }
        text.lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok().filter(serde_json::Value::is_object))
            .find_map(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| "h0x update: unreadable reply".to_owned())
    }

    pub fn run(bin: &Path) -> Result<Update, String> {
        let out = crate::spawn::command(bin).args(["update", "--yes", "--json"]).stdin(Stdio::null()).output().map_err(|e| format!("cannot run h0x update: {e}"))?;
        parse(&out.stdout).map_err(|e| format!("{e} {}", String::from_utf8_lossy(&out.stderr).trim()).trim().to_owned())
    }

    /// The first absolute PATH entry holding `h0x`.
    pub fn find_in(path: &OsStr) -> Option<PathBuf> {
        let name = if cfg!(windows) { "h0x.exe" } else { "h0x" };
        std::env::split_paths(path).filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).find(|bin| bin.is_file())
    }

    pub fn find() -> Option<PathBuf> {
        find_in(&std::env::var_os("PATH")?)
    }

    /// A nix profile or store path: nix owns that runtime.
    pub fn managed(bin: &Path) -> bool {
        let nix = |p: &Path| {
            let p = p.to_string_lossy().replace('\\', "/");
            ["/nix/store/", "/.nix-profile/", "/etc/profiles/", "/run/current-system/"].iter().any(|marker| p.contains(marker))
        };
        nix(bin) || std::fs::canonicalize(bin).is_ok_and(|real| nix(&real))
    }

    /// `X.Y.Z` from `h0x X.Y.Z`.
    pub fn version_in(text: &str) -> Option<String> {
        text.trim().strip_prefix("h0x ")?.split_whitespace().next().map(str::to_owned)
    }

    pub fn version(bin: &Path) -> Option<String> {
        let out = crate::spawn::command(bin).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| version_in(&String::from_utf8_lossy(&out.stdout))).flatten()
    }

    pub fn latest_version(manifest: &str) -> Result<String, String> {
        let value: serde_json::Value = serde_json::from_str(manifest).map_err(|e| format!("runtime latest.json: {e}"))?;
        value["version"].as_str().map(str::to_owned).ok_or_else(|| "runtime latest.json: no version".into())
    }
}

/// Looks for a newer runtime; nothing to do when none is installed.
fn check_runtime() -> Result<(), String> {
    check_runtime_of(runtime::find())
}

/// Problems already reported in this run (runtime path and error text): each is reported once,
/// not at every hourly check.
static RUNTIME_REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn first_report(key: String) -> bool {
    let mut seen = RUNTIME_REPORTED.lock().unwrap();
    if seen.contains(&key) {
        return false;
    }
    if seen.len() >= 20 {
        seen.clear();
    }
    seen.push(key);
    true
}

/// Never an error for the caller: a runtime problem is reported (once) and logged, and leaves
/// the app and core check status alone.
fn check_runtime_of(bin: Option<PathBuf>) -> Result<(), String> {
    let Some(bin) = bin.filter(|bin| !runtime::managed(bin)) else {
        let mut r = RUNTIME.lock().unwrap();
        r.ready = None;
        r.bin = None;
        return Ok(());
    };
    match latest_runtime_for(&bin) {
        Ok(latest) => {
            crate::events::update_checked("runtime");
            let mut r = RUNTIME.lock().unwrap();
            r.ready = latest.filter(|v| r.failed.as_deref() != Some(v.as_str()));
            r.bin = r.ready.is_some().then_some(bin);
        }
        Err(e) => {
            tracing::warn!("runtime update check: {e}");
            if first_report(format!("{}: {e}", bin.display())) {
                crate::analytics::report("runtime_update_failed", "check", &e);
            }
        }
    }
    Ok(())
}

/// The release version of the runtime at `bin`, when it is newer than the one installed.
fn latest_runtime_for(bin: &Path) -> Result<Option<String>, String> {
    let have = runtime::version(bin).ok_or("h0x --version doesn't run")?;
    let latest = first_ok(&runtime_sources(), |releases| runtime::latest_version(&curl_text(&format!("{releases}/latest/download/latest.json"))?))?;
    Ok(newer(&latest, &have).then_some(latest))
}

/// `h0x update --yes --json`, in the background, for a newer runtime found by the check.
fn update_runtime() {
    let (version, bin) = {
        let mut r = RUNTIME.lock().unwrap();
        if r.updating {
            return;
        }
        let (Some(version), Some(bin)) = (r.ready.take(), r.bin.take()) else { return };
        // A package manager may have taken it over since the check.
        if runtime::managed(&bin) {
            return;
        }
        r.updating = true;
        (version, bin)
    };
    std::thread::spawn(move || {
        let from = runtime::version(&bin).unwrap_or_default();
        let started = unix_now();
        let result = runtime::run(&bin);
        let mut r = RUNTIME.lock().unwrap();
        r.updating = false;
        let failure = match &result {
            Ok(update) => update.failure(),
            Err(e) => Some((false, e.clone())),
        };
        match (failure, result) {
            (Some((rolled_back, error)), _) => {
                r.failed = Some(version.clone());
                crate::analytics::report("runtime_update_failed", if rolled_back { "rollback" } else { "update" }, &error);
                if rolled_back {
                    crate::events::update_rolled_back("runtime", known(Some(from.as_str())), &version);
                    notify("error", rolled_back_text("Runtime", &version, &from));
                } else {
                    notify("error", "Runtime update failed. A report was sent.".into());
                }
            }
            (None, Ok(update)) if update.updated => {
                let to = update.to.unwrap_or(version);
                crate::events::update_applied("runtime", known(Some(from.as_str())), &to, unix_now().saturating_sub(started));
                notify("success", format!("Runtime updated to {to}"));
            }
            // Deferred, or nothing to do: the next check looks again.
            _ => {}
        }
    });
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
        let (from, target) = {
            let c = CORE.lock().unwrap();
            (c.version.clone(), c.ready.clone())
        };
        let started = unix_now();
        let result = crate::core_bin::update_detailed(None);
        let mut c = CORE.lock().unwrap();
        c.updating = false;
        match result {
            Ok(_) => {
                // The core that runs now, else the release it was offered; else unknown.
                let now = crate::core_bin::version(&crate::core_bin::path()).map(|v| v.trim_start_matches("h0x-core ").to_owned()).or(target);
                crate::events::update_applied("core", known(from.as_deref()), known(now.as_deref()), unix_now().saturating_sub(started));
                notify("success", now.as_deref().map_or_else(|| "Core updated".to_owned(), |v| format!("Core updated to {v}")));
                if now.is_some() {
                    c.version = now;
                }
                c.ready = None;
            }
            Err(e) => {
                if e.rolled_back {
                    crate::analytics::report("core_update_failed", "rollback", &e.message);
                    crate::events::update_rolled_back("core", known(from.as_deref()), known(target.as_deref()));
                    notify("error", rolled_back_text("Core", target.as_deref().unwrap_or(""), from.as_deref().unwrap_or("")));
                } else {
                    crate::analytics::report("core_update_failed", "update", &e.message);
                }
                c.error = Some(e.message);
            }
        }
    });
}

/// An empty `to` or `from` (not known) is left out of the text.
fn rolled_back_text(part: &str, to: &str, from: &str) -> String {
    let to = if to.is_empty() { String::new() } else { format!(" to {to}") };
    let from = if from.is_empty() { String::new() } else { format!(" to {from}") };
    format!("{part} update{to} failed, HyperOx went back{from}. A report was sent.")
}

/// A version for an event: `unknown` when it isn't.
fn known(version: Option<&str>) -> &str {
    version.filter(|v| !v.is_empty()).unwrap_or("unknown")
}

/// Downloads the latest release if it's newer. Ok(Some): a version is ready.
fn check(root: &Path) -> Result<Option<String>, String> {
    first_ok(&releases(), |source| check_from(root, source))
}

fn check_from(root: &Path, releases: &str) -> Result<Option<String>, String> {
    let path = "latest/download/latest.json";
    let text = curl_text(&format!("{releases}/{path}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let version = manifest["version"].as_str().ok_or(format!("{path}: no version"))?;
    if !newer(version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    if State::load().skips(version) {
        tracing::info!("update: {version} failed to start twice here; waiting for a newer release");
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
    if !swap(&root, version) {
        return false;
    }
    relaunch_expected(Some(version))
}

/// First thing in a start. Notes that this version is launching; a version that was
/// launched before, never started OK and did not quit cleanly (a crash, a kill) is put
/// back to `<install>.previous` here, for installs updated without the relaunch helper
/// (applied on quit). True if it did: the old version is opening, this one must exit.
pub fn begin_start() -> bool {
    let Some(root) = root() else { return false };
    let version = env!("CARGO_PKG_VERSION");
    let mut state = State::load();
    let crashed = state.launch(version);
    state.save();
    if !crashed {
        return false;
    }
    tracing::warn!("update: {version} never started OK and its last start did not end cleanly; going back");
    let to = last_good().unwrap_or_else(|| "unknown".to_owned());
    if roll_back(&root).is_err() {
        return false;
    }
    write_pending(&pending_path(), version, &to, "the new version crashed before it started OK");
    state.trying = None;
    state.launching = None;
    state.save();
    relaunch_expected(None)
}

/// Window shown and core connected: this version started OK. Writes the marker the
/// relaunch helper waits for and `last-good.json`, counts the start, and drops
/// `<install>.previous` after the 3rd.
pub fn started_ok() {
    static DONE: AtomicBool = AtomicBool::new(false);
    let Some(root) = root() else { return };
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut state = State::load();
    let drop_previous = state.started(version);
    state.save();
    let _ = std::fs::write(started_marker(version), "");
    let _ = std::fs::write(crate::mods::app_dir().join("last-good.json"), serde_json::json!({ "version": version }).to_string());
    if drop_previous {
        let _ = std::fs::remove_dir_all(beside(&root, ".previous"));
    }
}

/// The app is quitting on purpose: a start that ends like this was not a crash.
pub fn clean_exit() {
    let mut state = State::load();
    state.launching = None;
    state.save();
}

/// After analytics has started: a rollback the relaunch helper (or `begin_start`) left
/// is reported, counted, and shown once.
pub fn after_start() {
    let Some(pending) = take_pending(&pending_path()) else { return };
    let mut state = State::load();
    let failures = state.rolled_back(&pending.from);
    state.save();
    tracing::warn!("update: {} failed ({}), went back to {}; failure {failures}", pending.from, pending.reason, pending.to);
    crate::analytics::report("app_update_failed", "rollback", &format!("{} to {}: {}", pending.to, pending.from, pending.reason));
    crate::events::update_rolled_back("app", known(Some(&pending.to)), known(Some(&pending.from)));
    notify("error", rolled_back_text("App", &pending.from, &pending.to));
}

/// Puts the downloaded version in place of this one. The running app keeps
/// its files open, so it can go on until it quits. True if it swapped.
/// Windows lets a running program's folder be renamed, not deleted: the old
/// one is kept as `<install>.previous` (see `swap`).
pub fn apply() -> bool {
    let Some(root) = root() else { return false };
    let staged = staged_version(&root);
    let Some(version) = staged.as_deref().filter(|version| newer(version, env!("CARGO_PKG_VERSION"))) else { return false };
    if ready().as_deref() != Some(version) {
        return false;
    }
    swap(&root, version)
}

/// Swaps `version` in. The working version stays as `<install>.previous` until the
/// new one has started OK 3 times (`started_ok`), so a rollback has it to go back to.
fn swap(root: &Path, version: &str) -> bool {
    let staged = beside(root, ".new");
    let old = beside(root, ".previous");
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = std::fs::rename(root, &old) {
        tracing::warn!("update: cannot move {} aside: {e}", root.display());
        crate::analytics::report("update_failed", "swap", &format!("cannot move the old version aside: {e}"));
        return false;
    }
    if let Err(e) = std::fs::rename(&staged, root) {
        tracing::warn!("update: cannot put the new version in place: {e}");
        crate::analytics::report("update_failed", "swap", &format!("cannot put the new version in place: {e}"));
        if let Err(rollback) = std::fs::rename(&old, root) {
            tracing::error!("update: cannot restore the previous version: {rollback}");
            crate::analytics::report("update_failed", "rollback", &rollback.to_string());
        }
        return false;
    }
    let _ = std::fs::remove_file(beside(root, ".new.json"));
    *READY.lock().unwrap() = None;
    let _ = std::fs::write(marker(), "");
    let mut state = State::load();
    state.swapped(version);
    state.save();
    // A marker an earlier try of this version left must not count for this one.
    let _ = std::fs::remove_file(started_marker(version));
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

/// What the relaunch helper needs to watch a new version and, if it never starts OK,
/// put `previous` back. Paths travel as separate values, never inside script text
/// on unix, and quoted by `ps_quote` on Windows.
struct Watch {
    root: PathBuf,
    previous: PathBuf,
    exe: PathBuf,
    marker: PathBuf,
    pending: PathBuf,
    /// The version that must start OK.
    new: String,
    /// The version that is running now (the one to go back to).
    old: String,
}

/// A version fit for a JSON string typed by a script.
fn plain_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The watch for a version `swap` just put in place. None: nothing to go back to.
fn watch(root: &Path) -> Option<Watch> {
    let new = State::load().trying.filter(|v| plain_version(v))?;
    let old = env!("CARGO_PKG_VERSION").to_owned();
    let previous = beside(root, ".previous");
    (plain_version(&old) && previous.is_dir()).then(|| Watch {
        root: root.to_path_buf(),
        previous,
        exe: exe_in(root),
        marker: started_marker(&new),
        pending: pending_path(),
        new,
        old,
    })
}

/// The sh helper. `$0` is the pid to wait for, then `$1` root, `$2` previous, `$3` exe,
/// `$4` marker, `$5` pending-rollback file, `$6` new version, `$7` old version. Nothing
/// from the paths is ever parsed as shell text. `ps` ends the wait early when the new
/// version has died (a dead child stays a zombie, which `kill -0` still accepts).
#[cfg(any(unix, test))]
const WATCH_SH: &str = r#"while kill -0 "$0" 2>/dev/null; do sleep 0.1; done
"$3" &
new=$!
n=0
while [ ! -e "$4" ] && [ "$n" -lt 60 ]; do
  case "$(ps -o stat= -p "$new" 2>/dev/null)" in ""|Z*) break;; esac
  sleep 1
  n=$((n + 1))
done
[ -e "$4" ] && exit 0
case "$(ps -o stat= -p "$new" 2>/dev/null)" in
  ""|Z*) reason="the new version exited before it started OK";;
  *) reason="the new version did not start OK within 60 seconds"; kill "$new" 2>/dev/null; sleep 2; kill -9 "$new" 2>/dev/null;;
esac
i=1
while [ -e "$1.failed-$i" ]; do i=$((i + 1)); done
mv "$1" "$1.failed-$i" || exit 1
mv "$2" "$1" || { mv "$1.failed-$i" "$1"; exit 1; }
printf '{"from":"%s","to":"%s","reason":"%s"}' "$6" "$7" "$reason" > "$5"
exec "$3"
"#;

/// The PowerShell helper: the values first (each through `ps_quote`), then fixed text.
#[cfg(any(windows, test))]
fn watch_script(pid: u32, w: &Watch) -> String {
    let q = |p: &Path| ps_quote(&p.to_string_lossy());
    format!(
        "$old = {pid}; $exe = {}; $root = {}; $previous = {}; $marker = {}; $pending = {}; $newVersion = {}; $oldVersion = {}\n{WATCH_PS}",
        q(&w.exe),
        q(&w.root),
        q(&w.previous),
        q(&w.marker),
        q(&w.pending),
        ps_quote(&w.new),
        ps_quote(&w.old),
    )
}

#[cfg(any(windows, test))]
const WATCH_PS: &str = r"Wait-Process -Id $old -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $exe -PassThru
$deadline = (Get-Date).AddSeconds(60)
while (-not (Test-Path -LiteralPath $marker) -and -not $p.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
if (Test-Path -LiteralPath $marker) { exit 0 }
if ($p.HasExited) { $reason = 'the new version exited before it started OK' } else { $reason = 'the new version did not start OK within 60 seconds'; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 2 }
function Move-Retry($source, $dest) { for ($i = 0; $i -lt 10; $i++) { try { Move-Item -LiteralPath $source -Destination $dest -ErrorAction Stop; return $true } catch { Start-Sleep -Seconds 1 } }; return $false }
$n = 1
while (Test-Path -LiteralPath ($root + '.failed-' + $n)) { $n++ }
$failed = $root + '.failed-' + $n
if (-not (Move-Retry $root $failed)) { exit 1 }
if (-not (Move-Retry $previous $root)) { Move-Retry $failed $root | Out-Null; exit 1 }
$json = @{ from = $newVersion; to = $oldVersion; reason = $reason } | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText($pending, $json, (New-Object System.Text.UTF8Encoding($false)))
Start-Process -FilePath $exe
";

#[cfg(windows)]
fn relaunch_expected(expected: Option<&str>) -> bool {
    let Some(root) = root() else { return false };
    // The system's own PowerShell: one earlier on PATH must not run here.
    let Some(system_root) = std::env::var_os("SystemRoot") else { return false };
    let script = match watch(&root) {
        Some(w) => watch_script(std::process::id(), &w),
        None => relaunch_script(std::process::id(), &exe_in(&root)),
    };
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
    let plain = format!("while kill -0 \"$0\" 2>/dev/null; do sleep 0.1; done; {open}");
    let pid = std::process::id().to_string();
    let (script, args): (&str, Vec<&std::ffi::OsStr>) = match &watch(&root) {
        Some(w) => (WATCH_SH, vec![w.root.as_os_str(), w.previous.as_os_str(), w.exe.as_os_str(), w.marker.as_os_str(), w.pending.as_os_str(), std::ffi::OsStr::new(&w.new), std::ffi::OsStr::new(&w.old)]),
        None => (plain.as_str(), vec![root.as_os_str()]),
    };
    let spawned = {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        if let Some(expected) = expected {
            command.env("H0X_ADE_UPDATE_EXPECTED", expected);
        }
        command
            .arg("-c")
            .arg(script)
            .arg(&pid)
            .args(args)
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

/// Written by `started_ok`; the relaunch helper waits for it.
fn started_marker(version: &str) -> PathBuf {
    crate::mods::app_dir().join(format!("started-{version}"))
}

/// Left by a rollback for the old version to report on its next start.
fn pending_path() -> PathBuf {
    crate::mods::app_dir().join("pending-rollback.json")
}

/// The version that last started OK (`last-good.json`).
fn last_good() -> Option<String> {
    let bytes = std::fs::read(crate::mods::app_dir().join("last-good.json")).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["version"].as_str().map(str::to_owned)
}

/// Starts of a new version that must succeed before `<install>.previous` goes.
const OK_STARTS: u32 = 3;
/// Failures in a row of one version after which it is skipped.
const FAILURES: u32 = 2;

/// What the updater remembers between starts (`update-state.json` in the app home).
#[derive(Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct State {
    /// The version `ok` counts.
    current: String,
    /// How many times `current` started OK.
    ok: u32,
    /// Swapped in and not started OK yet.
    trying: Option<String>,
    /// Launched and neither started OK nor quit on purpose since.
    launching: Option<String>,
    /// The version whose update last failed, and how many times in a row.
    failed: Option<String>,
    failures: u32,
    /// Skipped until a newer release appears.
    bad: Option<String>,
}

impl State {
    fn load() -> Self {
        std::fs::read(crate::mods::app_dir().join("update-state.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let dir = crate::mods::app_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(dir.join("update-state.json"), text);
        }
    }

    /// `version` was put in place.
    fn swapped(&mut self, version: &str) {
        self.current = version.to_owned();
        self.ok = 0;
        self.trying = Some(version.to_owned());
        self.launching = None;
    }

    /// `version` is starting. True: it was launched before, never started OK and did not quit on purpose.
    fn launch(&mut self, version: &str) -> bool {
        if self.trying.as_deref() != Some(version) {
            return false;
        }
        let crashed = self.launching.as_deref() == Some(version);
        self.launching = Some(version.to_owned());
        crashed
    }

    /// `version` started OK. True: it has now done so `OK_STARTS` times, so `previous` can go.
    fn started(&mut self, version: &str) -> bool {
        if self.current != version {
            self.current = version.to_owned();
            self.ok = 0;
        }
        self.ok += 1;
        if self.trying.as_deref() == Some(version) {
            self.trying = None;
        }
        self.launching = None;
        if self.failed.as_deref() == Some(version) {
            self.failed = None;
            self.failures = 0;
        }
        self.ok >= OK_STARTS
    }

    /// The update to `from` was undone. Returns the failures in a row for it.
    fn rolled_back(&mut self, from: &str) -> u32 {
        self.trying = None;
        self.launching = None;
        self.failures = if self.failed.as_deref() == Some(from) { self.failures + 1 } else { 1 };
        self.failed = Some(from.to_owned());
        if self.failures >= FAILURES {
            self.bad = Some(from.to_owned());
        }
        self.failures
    }

    /// The updater leaves `version` alone: it failed twice, and nothing newer is out.
    fn skips(&self, version: &str) -> bool {
        self.bad.as_deref().is_some_and(|bad| !newer(version, bad))
    }
}

/// `<install>.failed-<n>`: where a version that did not start is kept.
fn failed_path(root: &Path, n: u32) -> PathBuf {
    let mut name = root.as_os_str().to_os_string();
    name.push(format!(".failed-{n}"));
    PathBuf::from(name)
}

/// Moves the install to a free `.failed-<n>` and `<install>.previous` into its place.
fn roll_back(root: &Path) -> Result<PathBuf, String> {
    let previous = beside(root, ".previous");
    if !previous.is_dir() {
        return Err(format!("{} is not there", previous.display()));
    }
    let failed = (1..1000).map(|n| failed_path(root, n)).find(|p| !p.exists()).ok_or("too many failed versions kept")?;
    std::fs::rename(root, &failed).map_err(|e| format!("{}: {e}", root.display()))?;
    if let Err(e) = std::fs::rename(&previous, root) {
        let _ = std::fs::rename(&failed, root);
        return Err(format!("{}: {e}", previous.display()));
    }
    Ok(failed)
}

struct Pending {
    from: String,
    to: String,
    reason: String,
}

fn write_pending(path: &Path, from: &str, to: &str, reason: &str) {
    let _ = std::fs::write(path, serde_json::json!({ "from": from, "to": to, "reason": reason }).to_string());
}

/// Reads and deletes what a rollback left. None: no rollback, or the file is not ours.
fn take_pending(path: &Path) -> Option<Pending> {
    let bytes = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let text = |key: &str| value[key].as_str().map(str::to_owned);
    Some(Pending { from: text("from")?, to: text("to")?, reason: text("reason")? })
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
                tracing::info!("h0x-core is {now}");
                core.version = Some(now.trim_start_matches("h0x-core ").to_owned());
                core.error = None;
            }
            Err(e) => {
                tracing::warn!("h0x-core update: {e}");
                crate::analytics::report("core_update_failed", "after_update", &e);
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
        let manifest: serde_json::Value = serde_json::from_str(&curl_text(&format!("{}/download/v{version}/latest.json", releases()[0])).unwrap()).unwrap();
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
            (
                "H0X_ADE_RELEASES_URL",
                releases as fn() -> Vec<String>,
                vec!["https://github.com/buckleson/h0x-ADE-Releases/releases".to_owned(), "https://github.com/vjk7989/h0x-ADE-app/releases".to_owned()],
            ),
            (
                "H0X_RUNTIME_RELEASES_URL",
                runtime_sources as fn() -> Vec<String>,
                vec!["https://github.com/buckleson/h0x-ADE-Runtime-Releases/releases".to_owned(), "https://github.com/vjk7989/h0x-ADE-Runtime/releases".to_owned()],
            ),
            (
                "H0X_CORE_RELEASES_URL",
                core_sources as fn() -> Vec<String>,
                vec!["https://github.com/buckleson/h0x-ADE-Core-Releases/releases".to_owned(), "https://github.com/vjk7989/h0x-ADE-core/releases".to_owned()],
            ),
        ] {
            unsafe { std::env::set_var(key, "https://mirror.example/releases/") };
            assert_eq!(read(), ["https://mirror.example/releases"], "a test site is the only source");
            for empty in ["", " ", "\t\n"] {
                unsafe { std::env::set_var(key, empty) };
                assert_eq!(read(), default, "{key}={empty:?} counts as unset");
            }
            unsafe { std::env::remove_var(key) };
            assert_eq!(read(), default, "the part's own repo first, then the older one");
        }
        assert_eq!(core_releases(), "https://github.com/vjk7989/h0x-ADE-core/releases", "the installer still reads the older repo");
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
    fn the_first_scheduled_check_waits_for_the_welcome_but_asking_does_not() {
        assert!(!scheduled(true, true), "nothing runs on the schedule before the welcome is dismissed");
        assert!(due(scheduled(true, true), true), "Check now is never held");
        assert!(scheduled(true, false));
        assert!(!scheduled(false, false), "the off switch still wins after the welcome");
        hold_until_welcomed();
        assert!(HELD.load(Ordering::Relaxed));
        welcomed();
        assert!(!HELD.load(Ordering::Relaxed));
    }

    // HELD and the shared values are process-wide: runs in a child process on an empty app home.
    #[test]
    fn dismissing_the_welcome_mid_run_releases_the_hold_and_no_other_key_does() {
        const CHILD: &str = "H0X_TEST_WELCOME_HOLD";
        if std::env::var_os(CHILD).is_none() {
            let home = std::env::temp_dir().join(format!("h0x-welcome-hold-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).unwrap();
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "update::tests::dismissing_the_welcome_mid_run_releases_the_hold_and_no_other_key_does", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .env("H0X_ADE_HOME", &home)
                .output()
                .unwrap();
            let _ = std::fs::remove_dir_all(&home);
            let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && log.contains("1 passed"), "{log}");
            return;
        }
        let held = || HELD.load(Ordering::Relaxed);
        // A first run: the welcome was never finished.
        assert_eq!(crate::mods::shared("frame.welcomed"), None);
        hold_until_welcomed();
        // Keys that only look like it leave the hold on.
        for key in ["frame.welcome", "welcomed", "frame.welcomed.", "Frame.Welcomed", ""] {
            crate::mods::set_shared(key.into(), serde_json::json!(true));
            assert!(held(), "{key:?} lifted the hold");
        }
        assert!(!due(scheduled(true, held()), false), "nothing on the schedule while held");
        assert!(due(scheduled(true, held()), true), "Check now works while held");
        assert!(due(scheduled(false, held()), true), "Check now works while held with automatic checks off");
        // Dismissed mid-run: the next scheduled round looks.
        crate::mods::set_shared("frame.welcomed".into(), serde_json::json!(true));
        assert!(!held());
        assert!(due(scheduled(true, held()), false));
        assert!(!due(scheduled(false, held()), false), "check_updates off stays off after the welcome");
        // Dismissing twice is harmless.
        crate::mods::set_shared("frame.welcomed".into(), serde_json::json!(true));
        assert!(!held());
    }

    #[test]
    fn turning_automatic_checks_back_on_wakes_the_checker() {
        let woken = || std::mem::take(&mut *WAKE.0.lock().unwrap());
        set_auto(true);
        woken();
        set_auto(true);
        assert!(!woken(), "already on: nothing to wake");
        set_auto(false);
        assert!(!woken(), "turning off wakes nothing");
        set_auto(false);
        set_auto(true);
        assert!(woken(), "off to on checks right away");
        assert!(AUTO.load(Ordering::Relaxed));
        // The woken checker counts it as asked, so it runs even if the setting flips back.
        assert!(due(false, true));
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
    fn quotes_only_and_other_powershell_specials_stay_one_literal() {
        assert_eq!(ps_quote("'"), "''''");
        assert_eq!(ps_quote("''"), "''''''");
        assert_eq!(ps_quote("'\u{2018}\u{2019}\u{201a}\u{201b}"), "'''\u{2018}\u{2018}\u{2019}\u{2019}\u{201a}\u{201a}\u{201b}\u{201b}'");
        // Inside single quotes nothing else is special: no expansion, no escapes.
        for text in ["$env:PATH", "`n", "\"double\"", "\u{201c}curly\u{201d}", "a;b|c&d", "$(calc)"] {
            assert_eq!(ps_quote(text), format!("'{text}'"));
        }
        let exe = Path::new(r"C:\Users\O’Neil\App Data\it's\HyperOx.exe");
        assert_eq!(
            relaunch_script(u32::MAX, exe),
            r"Wait-Process -Id 4294967295 -ErrorAction SilentlyContinue; Start-Process 'C:\Users\O’’Neil\App Data\it''s\HyperOx.exe'"
        );
    }

    #[test]
    fn seal_takes_any_file_name_and_leaves_nothing_when_it_fails() {
        let parent = std::env::temp_dir().join(format!("h0x-ade-seal-edges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        let (dir, file) = seal(&parent, b"\0\xff checked", "O\u{2019}Neil app 1.tar.gz").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"\0\xff checked");
        std::fs::remove_dir_all(&dir).unwrap();
        // Back-to-back copies never share (or fail on) a folder name.
        let dirs: Vec<PathBuf> = (0..64).map(|_| seal(&parent, b"", "x").unwrap().0).collect();
        let unique: std::collections::BTreeSet<&PathBuf> = dirs.iter().collect();
        assert_eq!(unique.len(), dirs.len());
        for d in &dirs {
            std::fs::remove_dir_all(d).unwrap();
        }
        // The file can't be made: the new folder is removed again.
        assert!(seal(&parent, b"x", "missing/install.sh").is_err());
        assert!(seal(&parent, b"x", "").is_err());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0, "no sealed folder left behind");
        // The parent is a file, not a folder.
        let not_dir = parent.join("file");
        std::fs::write(&not_dir, b"").unwrap();
        assert!(seal(&not_dir, b"x", "x").is_err());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn check_now_cuts_one_wait_short_and_is_not_remembered() {
        check_now();
        let started = std::time::Instant::now();
        assert!(sleep(Duration::from_secs(60)), "asked");
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(!sleep(Duration::from_millis(10)), "the next round is a scheduled one");
    }

    #[test]
    fn turning_automatic_checks_off_and_on_takes_effect_at_runtime() {
        set_auto(false);
        assert!(!due(AUTO.load(Ordering::Relaxed), false));
        assert!(due(AUTO.load(Ordering::Relaxed), true), "Check now while off");
        set_auto(true);
        assert!(due(AUTO.load(Ordering::Relaxed), false));
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_part_reads_its_own_repo_first_then_the_older_one() {
        assert_eq!(sources(None, "own", "older"), strings(&["own", "older"]));
        assert_eq!(sources(Some("site".into()), "own", "older"), strings(&["site"]), "a test site stands alone");
        assert_eq!(runtime_sources(), strings(&["https://github.com/buckleson/h0x-ADE-Runtime-Releases/releases", "https://github.com/vjk7989/h0x-ADE-Runtime/releases"]));
    }

    #[test]
    fn a_failed_source_falls_back_and_all_failing_reports_the_first_error() {
        let both = strings(&["own", "older"]);
        let mut tried = Vec::new();
        let got = first_ok(&both, |s| {
            tried.push(s.to_owned());
            if s == "own" { Err("not signed".to_owned()) } else { Ok(s.len()) }
        });
        assert_eq!((got, tried), (Ok(5), both.clone()));
        let mut tried = 0;
        let got: Result<(), String> = first_ok(&both, |s| {
            tried += 1;
            Err(format!("{s} down"))
        });
        assert_eq!((got, tried), (Err("own down".to_owned()), 2));
        let mut tried = 0;
        assert_eq!(first_ok(&strings(&["own", "older"]), |s| { tried += 1; Ok::<_, String>(s.to_owned()) }), Ok("own".to_owned()));
        assert_eq!(tried, 1, "a source that works ends the search");
        assert!(first_ok::<()>(&[], |_| Ok(())).is_err(), "no source is an error, not a success");
    }

    #[test]
    fn core_and_runtime_wait_for_two_quiet_minutes() {
        assert!(!idle(1000, 1000), "an agent is working right now");
        assert!(!idle(1000, 1000 + IDLE - 1));
        assert!(idle(1000, 1000 + IDLE));
        assert!(idle(0, unix_now()), "no agent ever worked");
        assert!(!idle(2000, 1000), "a clock that went back is not idle");
        note_agents(false);
        let before = LAST_WORKING.load(Ordering::Relaxed);
        note_agents(true);
        assert!(LAST_WORKING.load(Ordering::Relaxed) >= before);
        assert!(!idle(LAST_WORKING.load(Ordering::Relaxed), unix_now()), "a working agent starts the wait");
    }

    #[test]
    fn a_core_goes_in_only_when_ready_unowned_idle_and_not_failed() {
        let ready = CoreInfo { version: Some("0.1.0".into()), ready: Some("0.1.1".into()), ..CoreInfo::default() };
        assert!(core_waiting(&ready));
        assert!(!core_waiting(&CoreInfo { ready: None, ..ready.clone() }), "nothing newer");
        assert!(!core_waiting(&CoreInfo { managed: Some("nix".into()), ..ready.clone() }), "a package manager owns it");
        assert!(!core_waiting(&CoreInfo { updating: true, ..ready.clone() }));
        assert!(!core_waiting(&CoreInfo { error: Some("x".into()), ..ready }), "a failed try is not repeated on its own");
    }

    #[test]
    fn the_rollback_notice_names_both_versions() {
        assert_eq!(rolled_back_text("Core", "0.1.2", "0.1.1"), "Core update to 0.1.2 failed, HyperOx went back to 0.1.1. A report was sent.");
    }

    #[test]
    fn notices_are_handed_over_once() {
        let _ = take_notices();
        notify("success", "Core updated to 0.1.2".into());
        let got = take_notices();
        assert_eq!((got.len(), got[0].level, got[0].text.as_str()), (1, "success", "Core updated to 0.1.2"));
        assert!(take_notices().is_empty());
        // Core and runtime both went in during the same quiet spell: two toasts, in order.
        // (Kept in this test: a second test draining the same queue in parallel would race.)
        notify("success", "Core updated to 0.1.3".into());
        notify("error", rolled_back_text("Runtime", "0.9.7", "0.9.6"));
        let got: Vec<_> = take_notices().into_iter().map(|n| (n.level, n.text)).collect();
        assert_eq!(
            got,
            [("success", "Core updated to 0.1.3".to_owned()), ("error", "Runtime update to 0.9.7 failed, HyperOx went back to 0.9.6. A report was sent.".to_owned())]
        );
        assert!(take_notices().is_empty(), "each notice is shown once");
    }

    #[test]
    fn the_runtime_reply_is_one_json_line() {
        let ok = runtime::parse(b"{\"updated\":true,\"from\":\"0.9.5\",\"to\":\"0.9.6\",\"rolled_back\":false,\"deferred\":false}\n").unwrap();
        assert!(ok.updated && ok.to.as_deref() == Some("0.9.6") && ok.failure().is_none());
        let back = runtime::parse(b"warming up\n{\"updated\":false,\"rolled_back\":true,\"error\":\"smoke test failed\"}\n\n").unwrap();
        assert_eq!(back.failure(), Some((true, "smoke test failed".to_owned())), "the last line counts");
        let err = runtime::parse(b"{\"updated\":false,\"error\":{\"message\":\"no space\"}}").unwrap();
        assert_eq!(err.failure(), Some((false, "{\"message\":\"no space\"}".to_owned())));
        let bare = runtime::parse(b"{\"rolled_back\":true}").unwrap();
        assert_eq!(bare.failure(), Some((true, "rolled back".to_owned())));
        let deferred = runtime::parse(b"{\"updated\":false,\"deferred\":true}").unwrap();
        assert!(!deferred.updated && deferred.failure().is_none());
        assert!(runtime::parse(b"").is_err());
        assert!(runtime::parse(b"  \n\n").is_err());
        assert!(runtime::parse(b"not json").is_err());
        assert!(runtime::parse(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn the_runtime_is_the_first_absolute_path_entry_holding_h0x() {
        let root = std::env::temp_dir().join(format!("h0x-ade-runtime-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (empty, with) = (root.join("empty"), root.join("with"));
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&with).unwrap();
        let name = if cfg!(windows) { "h0x.exe" } else { "h0x" };
        std::fs::write(with.join(name), b"").unwrap();
        let path = |dirs: &[PathBuf]| std::env::join_paths(dirs).unwrap();
        assert_eq!(runtime::find_in(&path(&[empty.clone(), with.clone()])), Some(with.join(name)));
        assert_eq!(runtime::find_in(&path(&[empty.clone()])), None, "not installed");
        assert_eq!(runtime::find_in(std::ffi::OsStr::new("")), None, "an empty PATH");
        assert_eq!(runtime::find_in(&path(&[PathBuf::from("rel")])), None, "relative entries are skipped");
        std::fs::create_dir_all(empty.join(name)).unwrap();
        assert_eq!(runtime::find_in(&path(&[empty])), None, "a folder named h0x is not the program");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_nix_runtime_is_left_alone() {
        for owned in ["/nix/store/abc-h0x-0.9.5/bin/h0x", "/home/me/.nix-profile/bin/h0x", "/etc/profiles/per-user/me/bin/h0x", "/run/current-system/sw/bin/h0x"] {
            assert!(runtime::managed(Path::new(owned)), "{owned}");
        }
        for own in ["/usr/local/bin/h0x", "/home/me/.local/bin/h0x", r"C:\Users\me\AppData\Local\Programs\h0x\h0x.exe", ""] {
            assert!(!runtime::managed(Path::new(own)), "{own}");
        }
        assert!(runtime::managed(Path::new(r"\nix\store\abc\bin\h0x")), "separators don't matter");
    }

    #[test]
    fn no_runtime_or_a_nix_one_means_no_update_check() {
        RUNTIME.lock().unwrap().ready = Some("9.9.9".into());
        assert_eq!(check_runtime_of(None), Ok(()));
        assert_eq!(RUNTIME.lock().unwrap().ready, None, "not installed: nothing waits");
        RUNTIME.lock().unwrap().ready = Some("9.9.9".into());
        assert_eq!(check_runtime_of(Some(PathBuf::from("/nix/store/abc-h0x/bin/h0x"))), Ok(()), "no fetch for a package-managed one");
        assert_eq!(RUNTIME.lock().unwrap().ready, None);
    }

    #[test]
    fn runtime_versions_and_manifests_are_read_strictly() {
        assert_eq!(runtime::version_in("h0x 0.9.5\n").as_deref(), Some("0.9.5"));
        assert_eq!(runtime::version_in("h0x 0.9.5 (abc123)").as_deref(), Some("0.9.5"));
        assert_eq!(runtime::version_in("h0x-core 0.1.0"), None);
        assert_eq!(runtime::version_in("h0x "), None);
        assert_eq!(runtime::version_in(""), None);
        assert_eq!(runtime::latest_version("{\"version\":\"0.9.6\"}"), Ok("0.9.6".to_owned()));
        assert!(runtime::latest_version("{}").is_err());
        assert!(runtime::latest_version("<html>").is_err());
    }

    #[test]
    fn a_source_saying_nothing_newer_ends_the_search() {
        let mut tried = Vec::new();
        let got = first_ok(&strings(&["own", "older"]), |s| {
            tried.push(s.to_owned());
            Ok::<Option<String>, String>((s == "older").then(|| "9.9.9".to_owned()))
        });
        assert_eq!(got, Ok(None), "a reachable own repo with nothing newer is the answer");
        assert_eq!(tried, ["own"], "the older repo is not asked");
    }

    #[test]
    fn the_idle_gate_is_exactly_two_minutes_and_a_working_agent_restarts_it() {
        assert_eq!(IDLE, 120);
        let last = 1_700_000_000;
        assert!(!idle(last, last + 119));
        assert!(idle(last, last + 120), "exactly 120 s quiet is idle");
        assert!(idle(last, u64::MAX), "no overflow at the far end");
        note_agents(true);
        note_agents(false);
        let last = LAST_WORKING.load(Ordering::Relaxed);
        let now = unix_now();
        assert!(!idle(last, now), "an agent that started while an update waits holds it back, even once it stops");
        assert!(idle(last, now + IDLE), "two quiet minutes later it goes in");
    }

    #[test]
    fn a_runtime_in_a_folder_with_spaces_is_found() {
        let root = std::env::temp_dir().join(format!("h0x-ade-runtime-spaces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let spaced = root.join("Program Files").join("h0x runtime");
        std::fs::create_dir_all(&spaced).unwrap();
        let name = if cfg!(windows) { "h0x.exe" } else { "h0x" };
        std::fs::write(spaced.join(name), b"").unwrap();
        let path = std::env::join_paths([root.join("missing dir"), spaced.clone()]).unwrap();
        assert_eq!(runtime::find_in(&path), Some(spaced.join(name)));
        assert!(!runtime::managed(&spaced.join(name)), "a folder with spaces is not a nix one");
        assert!(!runtime::managed(Path::new(r"C:\Program Files\h0x\h0x.exe")));
        #[cfg(windows)]
        {
            let quoted = std::ffi::OsString::from(format!("C:\\nowhere;\"{}\"", spaced.display()));
            assert_eq!(runtime::find_in(&quoted), Some(spaced.join(name)), "a quoted PATH entry is the same folder");
            let bare = root.join("bare");
            std::fs::create_dir_all(&bare).unwrap();
            std::fs::write(bare.join("h0x"), b"").unwrap();
            assert_eq!(runtime::find_in(bare.as_os_str()), None, "on Windows only h0x.exe is the program");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_runtime_version_is_not_reported_every_hour() {
        let root = std::env::temp_dir().join(format!("h0x-ade-runtime-badversion-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Not a program at all, so `--version` gives nothing usable (and nothing runs).
        let bin = root.join(if cfg!(windows) { "h0x.exe" } else { "h0x" });
        std::fs::write(&bin, b"not a program").unwrap();
        let key = format!("{}: h0x --version doesn't run", bin.display());
        assert_eq!(check_runtime_of(Some(bin.clone())), Ok(()), "a runtime problem never fails the check round");
        assert!(!first_report(key.clone()), "the first check already reported it");
        assert_eq!(check_runtime_of(Some(bin)), Ok(()));
        assert!(!first_report(key), "an hour later it is still the same problem, reported once");
        assert!(first_report("another binary: another problem".into()), "a different problem is reported");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_versions_stay_out_of_texts_and_events() {
        assert_eq!(known(None), "unknown");
        assert_eq!(known(Some("")), "unknown");
        assert_eq!(known(Some("0.1.2")), "0.1.2");
        assert_eq!(rolled_back_text("Core", "", "0.1.1"), "Core update failed, HyperOx went back to 0.1.1. A report was sent.");
        assert_eq!(rolled_back_text("Core", "0.1.2", ""), "Core update to 0.1.2 failed, HyperOx went back. A report was sent.");
        assert!(!rolled_back_text("Runtime", "", "").contains("the latest"));
    }

    #[test]
    fn the_runtime_reply_skips_trailing_text_lines() {
        let u = runtime::parse(b"{\"updated\":true,\"to\":\"0.9.6\"}\nnote: restart your shell\n[1, 2]\n").unwrap();
        assert!(u.updated && u.to.as_deref() == Some("0.9.6"));
        assert!(runtime::parse(b"[1]\n\"text\"\n").is_err(), "only a JSON object is a reply");
    }

    #[test]
    fn a_checked_runtime_is_the_one_that_updates_and_a_nix_one_is_refused_at_apply_time() {
        {
            let mut r = RUNTIME.lock().unwrap();
            r.ready = Some("9.9.9".into());
            r.bin = Some(PathBuf::from("/nix/store/abc-h0x/bin/h0x"));
            r.updating = false;
        }
        update_runtime();
        let r = RUNTIME.lock().unwrap();
        assert!(!r.updating, "nix took it over after the check: nothing runs");
    }

    #[test]
    fn sha256_is_hex() {
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn previous_goes_only_after_three_ok_starts() {
        let mut state = State::default();
        state.swapped("0.2.0");
        assert_eq!(state.trying.as_deref(), Some("0.2.0"));
        assert!(!state.started("0.2.0"));
        assert!(state.trying.is_none(), "one OK start ends the trial");
        assert!(!state.started("0.2.0"));
        assert!(state.started("0.2.0"), "the third OK start drops previous");
        // A newer swap starts the count again.
        state.swapped("0.3.0");
        assert!(!state.started("0.3.0"));
        assert_eq!(state.ok, 1);
        // A state file that never saw a swap counts too (an empty or older file).
        let mut fresh = State::default();
        assert!(!fresh.started("0.1.0") && !fresh.started("0.1.0") && fresh.started("0.1.0"));
    }

    #[test]
    fn a_version_that_failed_twice_in_a_row_is_skipped_until_a_newer_one() {
        let mut state = State::default();
        assert!(!state.skips("0.2.0"), "nothing is skipped at first");
        state.swapped("0.2.0");
        assert_eq!(state.rolled_back("0.2.0"), 1);
        assert!(!state.skips("0.2.0"), "one failure still retries");
        state.swapped("0.2.0");
        assert_eq!(state.rolled_back("0.2.0"), 2);
        assert!(state.skips("0.2.0"));
        assert!(state.skips("0.1.9"), "older than the bad one is skipped too");
        assert!(!state.skips("0.2.1"), "a newer release is tried");
        assert!(!state.skips("0.3.0"));
    }

    #[test]
    fn failures_must_be_in_a_row_for_the_same_version() {
        let mut state = State::default();
        assert_eq!(state.rolled_back("0.2.0"), 1);
        assert_eq!(state.rolled_back("0.2.1"), 1, "another version starts the count over");
        assert!(state.bad.is_none());
        state.swapped("0.2.1");
        assert!(!state.started("0.2.1"));
        assert!(state.failed.is_none(), "a good start clears the failure");
        assert_eq!(state.rolled_back("0.2.1"), 1);
    }

    #[test]
    fn a_launch_that_neither_started_ok_nor_quit_is_a_crash() {
        let mut state = State::default();
        assert!(!state.launch("0.2.0"), "a version that was not swapped in is not watched");
        state.swapped("0.2.0");
        assert!(!state.launch("0.2.0"), "the first start of a new version");
        assert!(state.launch("0.2.0"), "the next one finds it never ended well");
        // A clean quit (launching cleared) is not a crash.
        let mut quit = State::default();
        quit.swapped("0.2.0");
        assert!(!quit.launch("0.2.0"));
        quit.launching = None;
        assert!(!quit.launch("0.2.0"));
        // Started OK ends the watch.
        let mut ok = State::default();
        ok.swapped("0.2.0");
        assert!(!ok.launch("0.2.0"));
        ok.started("0.2.0");
        assert!(!ok.launch("0.2.0"));
        // Another version's record does not count.
        let mut other = State::default();
        other.swapped("0.2.0");
        assert!(!other.launch("0.2.0"));
        assert!(!other.launch("0.1.9"));
    }

    #[test]
    fn state_survives_a_round_trip_and_garbage_is_empty() {
        let mut state = State::default();
        state.swapped("0.2.0");
        state.rolled_back("0.2.0");
        state.rolled_back("0.2.0");
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&text).unwrap(), state);
        assert_eq!(serde_json::from_str::<State>("{}").unwrap(), State::default(), "missing fields default");
        assert!(serde_json::from_str::<State>("[1]").is_err());
        assert!(serde_json::from_str::<State>("").is_err());
    }

    #[test]
    fn roll_back_keeps_each_failed_install_and_restores_previous() {
        let parent = std::env::temp_dir().join(format!("h0x-ade-roll-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let root = parent.join("My Apps").join("it's h0x-ade");
        let previous = beside(&root, ".previous");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::write(root.join("which"), "new").unwrap();
        std::fs::write(previous.join("which"), "old").unwrap();
        let failed = roll_back(&root).unwrap();
        assert_eq!(failed, failed_path(&root, 1));
        assert_eq!(std::fs::read_to_string(root.join("which")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(failed.join("which")).unwrap(), "new");
        assert!(!previous.exists());
        // Nothing left to go back to: an error, and the install stays as it is.
        assert!(roll_back(&root).is_err());
        assert_eq!(std::fs::read_to_string(root.join("which")).unwrap(), "old");
        // A second failure does not overwrite the first.
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::write(previous.join("which"), "older").unwrap();
        assert_eq!(roll_back(&root).unwrap(), failed_path(&root, 2));
        assert!(failed.is_dir());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn pending_rollback_is_read_once_and_deleted() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending-rollback.json");
        assert!(take_pending(&path).is_none(), "no file, no rollback");
        write_pending(&path, "0.2.0", "0.1.9", "it \"crashed\"");
        let pending = take_pending(&path).unwrap();
        assert_eq!((pending.from.as_str(), pending.to.as_str(), pending.reason.as_str()), ("0.2.0", "0.1.9", "it \"crashed\""));
        assert!(!path.exists());
        assert!(take_pending(&path).is_none());
        // Not ours, or missing a field: ignored and still deleted.
        for text in ["", "[]", "{\"from\":\"0.2.0\"}", "{\"from\":1,\"to\":2,\"reason\":3}"] {
            std::fs::write(&path, text).unwrap();
            assert!(take_pending(&path).is_none(), "{text}");
            assert!(!path.exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_versions_are_safe_to_type_into_a_script() {
        for v in ["0.2.0", "0.2.0-rc.1", "1_2"] {
            assert!(plain_version(v), "{v}");
        }
        for v in ["", "0.2.0\"", "a b", "$(x)", "0.2.0'", "a;b", "0.2.0\n", "1".repeat(41).as_str()] {
            assert!(!plain_version(v), "{v:?}");
        }
    }

    fn watch_of(root: &str, previous: &str, marker: &str, pending: &str) -> Watch {
        Watch {
            root: PathBuf::from(root),
            previous: PathBuf::from(previous),
            exe: PathBuf::from(root).join("h0x-ade.exe"),
            marker: PathBuf::from(marker),
            pending: PathBuf::from(pending),
            new: "0.2.0".into(),
            old: "0.1.9".into(),
        }
    }

    #[test]
    fn watch_script_quotes_every_path_even_with_spaces_quotes_and_non_ascii() {
        let w = watch_of(
            "C:/Users/O\u{2019}Neil/My Apps/it's here/h0x-ade",
            "C:/Users/O\u{2019}Neil/My Apps/it's here/h0x-ade.previous",
            "C:/Users/Zo\u{eb} \u{4e2d}\u{6587}/h0x-ade/started-0.2.0",
            "C:/Users/Zo\u{eb} \u{4e2d}\u{6587}/h0x-ade/pending-rollback.json",
        );
        let script = watch_script(42, &w);
        let head = script.lines().next().unwrap();
        assert_eq!(
            head,
            "$old = 42; $exe = 'C:/Users/O\u{2019}\u{2019}Neil/My Apps/it''s here/h0x-ade/h0x-ade.exe'; \
             $root = 'C:/Users/O\u{2019}\u{2019}Neil/My Apps/it''s here/h0x-ade'; \
             $previous = 'C:/Users/O\u{2019}\u{2019}Neil/My Apps/it''s here/h0x-ade.previous'; \
             $marker = 'C:/Users/Zo\u{eb} \u{4e2d}\u{6587}/h0x-ade/started-0.2.0'; \
             $pending = 'C:/Users/Zo\u{eb} \u{4e2d}\u{6587}/h0x-ade/pending-rollback.json'; \
             $newVersion = '0.2.0'; $oldVersion = '0.1.9'"
        );
        // After the values only fixed text: no path can reach it.
        assert!(script.ends_with(WATCH_PS));
        assert!(!WATCH_PS.contains("C:/"));
    }

    #[test]
    fn watch_script_cannot_be_broken_out_of_by_a_hostile_path() {
        let w = watch_of("C:/a'; Remove-Item -Recurse C:/ ; '", "C:/p$(calc)", "C:/m`n", "C:/q\u{201b}x");
        let script = watch_script(1, &w);
        let head = script.lines().next().unwrap();
        assert!(head.contains("$root = 'C:/a''; Remove-Item -Recurse C:/ ; '''; "), "{head}");
        assert!(head.contains("$previous = 'C:/p$(calc)'; "), "single quotes keep $( literal");
        assert!(head.contains("$pending = 'C:/q\u{201b}\u{201b}x'; "), "{head}");
        assert_eq!(script.lines().count(), 1 + WATCH_PS.lines().count());
    }

    #[test]
    fn watch_script_waits_60_seconds_kills_moves_back_and_leaves_the_note() {
        let ps = WATCH_PS;
        assert!(ps.contains("AddSeconds(60)"));
        assert!(ps.contains("Test-Path -LiteralPath $marker"));
        assert!(ps.contains("Stop-Process -Id $p.Id -Force"), "a hung new process is killed");
        assert!(ps.contains("$root + '.failed-' + $n"));
        let failed = ps.find("Move-Retry $root $failed").unwrap();
        let restore = ps.find("Move-Retry $previous $root").unwrap();
        let note = ps.find("WriteAllText($pending").unwrap();
        let start_old = ps.rfind("Start-Process -FilePath $exe").unwrap();
        assert!(failed < restore && restore < note && note < start_old, "the note is written before the old version starts");
        assert!(ps.contains("UTF8Encoding($false)"), "no byte order mark: the reader is serde_json");
        assert!(ps.contains("exit 0"), "a marker ends the watch");
        assert!(!ps.contains('"'), "no double quote to escape through the command line");
    }

    #[test]
    fn watch_sh_takes_every_path_as_an_argument() {
        let sh = WATCH_SH;
        for arg in ["$0", "$1", "$2", "$3", "$4", "$5", "$6", "$7"] {
            assert!(sh.contains(arg), "{arg}");
        }
        // Every expansion is double-quoted, so a path with spaces or a quote stays one word.
        for line in sh.lines() {
            for arg in ["$1", "$2", "$3", "$4", "$5"] {
                for (at, _) in line.match_indices(arg) {
                    assert!(line[..at].ends_with('"'), "{arg} unquoted in: {line}");
                }
            }
        }
        assert!(sh.contains("-lt 60"), "waits 60 seconds");
        assert!(sh.contains("kill \"$new\"") && sh.contains("kill -9 \"$new\""));
        assert!(sh.contains("\"$1.failed-$i\""));
        let failed = sh.find("mv \"$1\" \"$1.failed-$i\"").unwrap();
        let restore = sh.find("mv \"$2\" \"$1\"").unwrap();
        let note = sh.find("> \"$5\"").unwrap();
        let start_old = sh.rfind("exec \"$3\"").unwrap();
        assert!(failed < restore && restore < note && note < start_old, "the note is written before the old version starts");
        // The command line never carries path text.
        assert!(!sh.contains("eval") && !sh.contains("sh -c"));
    }

    #[test]
    fn rollback_text_and_report_wording() {
        assert_eq!(rolled_back_text("App", "0.2.0", "0.1.9"), "App update to 0.2.0 failed, HyperOx went back to 0.1.9. A report was sent.");
    }
}
