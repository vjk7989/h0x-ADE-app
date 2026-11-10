//! Finding h0x-core on this machine. The app doesn't carry one: it
//! uses the core people installed, or installs the core's own release.
//!
//! Where it looks, in order: `core` in settings.json; this process's PATH;
//! the person's login shell's PATH (an app opened from the Dock or a
//! launcher doesn't get it); then the usual install places. A candidate
//! counts only if `--version` says `h0x-core …`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

#[cfg(any(target_os = "macos", all(test, unix)))]
mod cli_link;

static BIN: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(target_os = "macos")]
pub fn install_cli_link() {
    if let Err(e) = std::thread::Builder::new().name("cli-link".into()).spawn(|| {
        let result = std::env::current_exe().and_then(|exe| cli_link::ensure(&crate::mods::home(), &exe));
        if let Err(e) = result {
            tracing::warn!("cannot install h0x-ade CLI link: {e}");
        }
    }) {
        tracing::warn!("cannot start h0x-ade CLI setup: {e}");
    }
}

/// The core the app runs (`h0x-core` until one is found).
pub fn path() -> PathBuf {
    BIN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| PathBuf::from("h0x-core"))
}

/// Uses this core from now on.
pub fn use_core(bin: PathBuf) {
    *BIN.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(bin);
}

/// The PATH for the programs mods start: the chosen core's folder first, so
/// their `core()` finds the same core the app runs. Given to those children,
/// never set in this process: `set_var` while other threads run is a data race
/// on unix. Only an absolute folder goes on it (a relative or empty one means
/// "here"); None if there is none.
pub fn path_with_core() -> Option<std::ffi::OsString> {
    let bin = BIN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()?;
    let dir = bin.parent().filter(|d| d.is_absolute())?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = vec![dir.to_path_buf()];
    // An empty or relative entry means "here": never carried over.
    dirs.extend(std::env::split_paths(&path).filter(|d| d != dir && d.is_absolute()));
    std::env::join_paths(dirs).ok()
}

/// Finder and Dock launches don't inherit the person's shell PATH.
#[cfg(target_os = "macos")]
pub fn fill_path() {
    if std::os::unix::process::parent_id() != 1 {
        return;
    }
    let home = crate::mods::home();
    let current = std::env::var_os("PATH").unwrap_or_default();
    if let Some(path) = merged_path(login_shell_path().as_deref(), &usual_dirs(&home)[..3], &current) {
        // SAFETY: called at startup, before other threads read the environment.
        unsafe { std::env::set_var("PATH", path) };
    }
}

#[cfg(any(target_os = "macos", test))]
fn merged_path(login: Option<&std::ffi::OsStr>, usual: &[PathBuf], current: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = login.map(|path| dirs_on(path).collect()).unwrap_or_default();
    dirs.extend_from_slice(usual);
    dirs.extend(dirs_on(current));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    std::env::join_paths(dirs).ok()
}

/// `h0x-core X.Y.Z` if this binary is h0x-core and runs.
pub fn version(bin: &Path) -> Option<String> {
    let out = crate::spawn::command(bin).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && text.starts_with("h0x-core ")).then_some(text)
}

/// The first h0x-core found, or None.
pub fn find(configured: Option<&str>) -> Option<PathBuf> {
    let home = crate::mods::home();
    // Absolute only: an empty or relative PATH entry or configured path means the
    // current folder, where a planted h0x-core must never be found.
    let pick = |candidates: Vec<PathBuf>| candidates.into_iter().filter(|c| c.is_absolute() && c.is_file()).find(|c| version(c).is_some());
    if let Some(found) = pick(configured.map(|c| vec![expand(c, &home)]).unwrap_or_default()) {
        return Some(found);
    }
    // Tests (debug and test builds only): H0X_CORE_DIRS replaces every place HyperOx would look.
    if let Some(dirs) = std::env::var_os("H0X_CORE_DIRS").filter(|_| crate::update::OVERRIDES) {
        return pick(std::env::split_paths(&dirs).map(|d| core_in(&d)).collect());
    }
    // The quick places first; the login shell (slow) only if they have none.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let quick: Vec<PathBuf> = dirs_on(&path).chain(usual_dirs(&home)).map(|d| core_in(&d)).collect();
    pick(quick).or_else(|| pick(dirs_on(&login_shell_path()?).map(|d| core_in(&d)).collect()))
}

fn expand(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    }
}

fn core_in(dir: &Path) -> PathBuf {
    dir.join(format!("h0x-core{}", std::env::consts::EXE_SUFFIX))
}

fn dirs_on(path: &std::ffi::OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path)
}

/// Where h0x-core gets installed: its own installer (~/.local/bin),
/// Homebrew, mise, Nix, cargo.
fn usual_dirs(home: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Local"));
        return vec![local.join("Programs/h0x-core"), home.join(".cargo/bin")];
    }
    vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/share/mise/shims"),
        home.join(".nix-profile/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
        home.join(".cargo/bin"),
    ]
}

/// The PATH the person's login shell sets up, or None if it doesn't answer quickly.
fn login_shell_path() -> Option<std::ffi::OsString> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // A slow or interactive shell profile must not hold up the app.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.into())
}

/// Installs the core version this app pins (distribution/core-version, never
/// "latest") with its own installer (into ~/.local/bin, or H0X_CORE_INSTALL_DIR),
/// then finds it. Err: what went wrong.
/// The script is downloaded (https, GitHub only) to a file, run only if its
/// `.minisig` is the release key's signature, and run as one argument; values
/// reach it through the environment, never a command string. The installer then
/// checks the release archive's signature itself, unless a releases site was
/// set by H0X_CORE_RELEASES_URL (inherited as it is, never set here: a value
/// set here would switch that check off).
pub fn install() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("H0X_CORE_INSTALL_DIR").map(PathBuf::from).filter(|d| !d.is_absolute()) {
        return Err(format!("H0X_CORE_INSTALL_DIR must be an absolute folder, not {}", dir.display()));
    }
    let releases = crate::update::core_releases();
    let script = if cfg!(windows) { "install.ps1" } else { "install.sh" };
    let home = crate::mods::ensure_app_dir().map_err(|e| format!("{}: {e}", crate::mods::app_dir().display()))?;
    let work = home.join(format!("core-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let file = work.join(script);
    // Run a fresh copy of the checked bytes, not the downloaded path.
    let pinned = crate::core_link::core_min();
    let result = crate::update::download(&format!("{releases}/download/v{pinned}/{script}"), &file, "120").and_then(|bytes| {
        let (sealed_dir, sealed) = crate::update::seal(&home, &bytes, script)?;
        let ran = run_installer(&sealed, pinned);
        let _ = std::fs::remove_dir_all(&sealed_dir);
        ran
    });
    let _ = std::fs::remove_dir_all(&work);
    result?;
    let home = crate::mods::home();
    let dir = std::env::var_os("H0X_CORE_INSTALL_DIR").map(PathBuf::from).unwrap_or_else(|| {
        if cfg!(windows) {
            usual_dirs(&home).remove(0)
        } else {
            home.join(".local/bin")
        }
    });
    let bin = core_in(&dir);
    let installed = version(&bin).ok_or("installed, but h0x-core doesn't run")?;
    if !is_pinned(&installed, pinned) {
        return Err(format!("installed, but it is {installed}, not the pinned h0x-core {pinned}"));
    }
    Ok(bin)
}

/// The releases-site variables the core installer must not see: with one set it
/// fetches from that site and skips the signature check. Test builds keep them (their stub sites).
fn releases_site_vars(overrides: bool) -> &'static [&'static str] {
    if overrides { &[] } else { &["H0X_CORE_RELEASES_URL", "H0X_ADE_RELEASES_URL"] }
}

fn run_installer(file: &Path, version: &str) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        // Absolute: a powershell.exe earlier on PATH must not run the installer.
        let root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
        let mut c = crate::spawn::command(PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(file);
        c
    } else {
        let mut c = crate::spawn::command("/bin/sh");
        c.arg(file);
        c
    };
    for var in releases_site_vars(crate::update::OVERRIDES) {
        cmd.env_remove(var);
    }
    let out = cmd.env("H0X_CORE_VERSION", version).stdin(Stdio::null()).output().map_err(|e| format!("cannot run the installer: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if err.is_empty() { "the installer failed".into() } else { err });
    }
    Ok(())
}

/// The installer's signature binds the file name, not the version: the core that
/// landed must say it is exactly the pinned one.
fn is_pinned(version_line: &str, pinned: &str) -> bool {
    version_line.trim() == format!("h0x-core {pinned}")
}

/// Updates the core to its latest release (`h0x-core update`: same generation only,
/// terminals kept). Ok: the version it runs now. Err: what it said.
pub fn update() -> Result<String, String> {
    update_to(None)
}

/// `h0x-core update --version X`: at least X (a newer core stays).
pub fn update_to(at_least: Option<&str>) -> Result<String, String> {
    let mut cmd = crate::spawn::command(path());
    cmd.args(["update", "--json"]);
    if let Some(v) = at_least {
        cmd.args(["--version", v]);
    }
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("cannot run h0x-core update: {e}"))?;
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    if out.status.success() {
        return Ok(reply.get("now").and_then(|v| v.as_str()).unwrap_or("the latest").to_owned());
    }
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap_or_default();
    let message = err.pointer("/error/message").and_then(|v| v.as_str()).map(str::to_owned);
    let hint = err.pointer("/error/hint").and_then(|v| v.as_str()).map(|h| format!(" ({h})")).unwrap_or_default();
    Err(message.map(|m| m + &hint).unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn setup_can_replace_an_unusable_core_before_reconnecting() {
        const CHILD: &str = "H0X_TEST_RESELECT_CORE";
        // BIN is process-wide: run in a child process so other tests keep their own.
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "core_bin::tests::setup_can_replace_an_unusable_core_before_reconnecting", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(output.status.success(), "{}
{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("h0x-core-reselect-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let bad = root.join("bad-core");
        std::fs::write(&bad, "#!/bin/sh
if [ \"$1\" = --version ]; then echo h0x-core 0.1.0; exit; fi
read hello
echo unusable >&2
exit 1
").unwrap();
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
        for replacement in ["chosen", "installed"] {
            use_core(find(Some(bad.to_str().unwrap())).unwrap());
            assert!(crate::core_link::Core::connect().is_err());
            let good = root.join(replacement);
            let hello = serde_json::json!({"result": {"selected": replacement}});
            std::fs::write(&good, format!("#!/bin/sh
read hello
echo '{hello}'
")).unwrap();
            std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o755)).unwrap();
            use_core(good.clone());
            let core = crate::core_link::Core::connect().unwrap_or_else(|e| panic!("{replacement}: {e:#}"));
            assert_eq!(path(), good);
            assert_eq!(core.hello["selected"], replacement);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // BIN and PATH are process-wide: runs in a child process.
    #[test]
    fn the_core_path_survives_a_poisoned_lock_and_concurrent_replacement() {
        const CHILD: &str = "H0X_TEST_CORE_PATH_LOCK";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "core_bin::tests::the_core_path_survives_a_poisoned_lock_and_concurrent_replacement", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let (a, b) = if cfg!(windows) {
            (PathBuf::from(r"C:\Users\me\AppData\Local\Programs\h0x-core\h0x-core.exe"), PathBuf::from(r"D:\Tools\h0x core\h0x-core.exe"))
        } else {
            (PathBuf::from("/home/me/.local/bin/h0x-core"), PathBuf::from("/opt/h0x core/bin/h0x-core"))
        };
        // A thread that panicked while holding the lock must not take the core away.
        let _ = std::thread::spawn(|| {
            let _held = BIN.lock().unwrap();
            panic!("poisons the lock");
        })
        .join();
        assert!(BIN.is_poisoned());
        assert_eq!(path(), PathBuf::from("h0x-core"));
        use_core(a.clone());
        assert_eq!(path(), a);
        // Readers during replacements see one whole path or the other.
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let (a, b) = (a.clone(), b.clone());
                std::thread::spawn(move || (0..2000).all(|_| [&a, &b].contains(&&path())))
            })
            .collect();
        for i in 0..2000 {
            use_core(if i % 2 == 0 { b.clone() } else { a.clone() });
        }
        assert!(readers.into_iter().all(|r| r.join().unwrap()));
        assert_eq!(path(), a);
        // Each replacement puts only its own folder in front, without piling up PATH entries,
        // and the process's own PATH is left alone.
        let before = std::env::var_os("PATH");
        use_core(b.clone());
        assert_eq!(std::env::var_os("PATH"), before);
        let dirs: Vec<PathBuf> = std::env::split_paths(&path_with_core().unwrap()).collect();
        assert_eq!(dirs[0], b.parent().unwrap());
        // Only the core in use is added: a replaced core's folder is not left behind.
        assert_eq!(dirs.iter().filter(|d| d.as_path() == a.parent().unwrap()).count(), 0, "{dirs:?}");
        assert_eq!(dirs.iter().filter(|d| d.as_path() == b.parent().unwrap()).count(), 1, "{dirs:?}");
    }

    // BIN and PATH are process-wide: runs in a child process.
    #[test]
    fn the_core_folder_goes_first_once_whatever_its_name() {
        const CHILD: &str = "H0X_TEST_PATH_WITH_CORE";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "core_bin::tests::the_core_folder_goes_first_once_whatever_its_name", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let log = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(output.status.success() && log.contains("1 passed"), "{log}");
            return;
        }
        let (dir, written, others) = if cfg!(windows) {
            (r"C:\Users\Zoë\Programme mit Leerzeichen\h0x-核心", r"C:\Users\Zoë\Programme mit Leerzeichen\h0x-核心\", [r"C:\Windows\System32", r"C:\Tools"])
        } else {
            ("/home/zoë/my tools/h0x-核心", "/home/zoë/my tools/h0x-核心/", ["/usr/bin", "/bin"])
        };
        let dir = PathBuf::from(dir);
        // SAFETY: this child process runs only this test.
        unsafe { std::env::set_var("PATH", std::env::join_paths([others[0], written, others[1]]).unwrap()) };
        let before = std::env::var_os("PATH");
        use_core(dir.join(if cfg!(windows) { "h0x-core.exe" } else { "h0x-core" }));
        let dirs: Vec<PathBuf> = std::env::split_paths(&path_with_core().unwrap()).collect();
        // Already on PATH (here with a trailing separator): moved to the front, not added twice.
        assert_eq!(dirs, [dir.clone(), PathBuf::from(others[0]), PathBuf::from(others[1])], "{dirs:?}");
        assert_eq!(std::env::var_os("PATH"), before, "the process's own PATH is left alone");
    }

    // BIN and PATH are process-wide: runs in a child process.
    #[test]
    fn an_empty_or_unset_path_gets_the_core_folder_and_never_the_current_one() {
        const CHILD: &str = "H0X_TEST_PATH_WITH_CORE_EMPTY";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "core_bin::tests::an_empty_or_unset_path_gets_the_core_folder_and_never_the_current_one", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let log = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(output.status.success() && log.contains("1 passed"), "{log}");
            return;
        }
        let dir = PathBuf::from(if cfg!(windows) { r"C:\Users\me\AppData\Local\Programs\h0x-core" } else { "/home/me/.local/bin" });
        use_core(dir.join("h0x-core"));
        // An empty PATH entry means the current folder: a mod's program must not be looked up there.
        // SAFETY: this child process runs only this test.
        unsafe { std::env::set_var("PATH", "") };
        assert_eq!(path_with_core(), Some(dir.clone().into_os_string()), "PATH empty");
        // SAFETY: this child process runs only this test.
        unsafe { std::env::remove_var("PATH") };
        assert_eq!(path_with_core(), Some(dir.into_os_string()), "PATH unset");
    }

    // The committed fixture core sits in the child's current folder, with `.`
    // and an empty entry on PATH. BIN, PATH and the folder are process-wide.
    #[cfg(unix)]
    #[test]
    fn a_core_in_the_current_folder_or_on_a_relative_path_is_never_used() {
        const CHILD: &str = "H0X_TEST_RELATIVE_CORE";
        if std::env::var_os(CHILD).is_none() {
            let home = std::env::temp_dir().join(format!("h0x-core-relative-home-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "core_bin::tests::a_core_in_the_current_folder_or_on_a_relative_path_is_never_used", "--nocapture"])
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core-bin"))
                .env(CHILD, "1")
                .env("PATH", ".:")
                .env("HOME", &home)
                .env("SHELL", "/nonexistent/sh")
                .env_remove("H0X_CORE_DIRS")
                .output()
                .unwrap();
            let _ = std::fs::remove_dir_all(&home);
            assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let here = std::env::current_dir().unwrap();
        assert!(here.join("h0x-core").is_file());
        let safe = |found: &Option<PathBuf>| found.as_deref().is_none_or(|p| p.is_absolute() && !p.starts_with(&here));
        let found = find(None);
        assert!(safe(&found), "found {found:?} through a relative PATH entry");
        for configured in ["h0x-core", "./h0x-core"] {
            let found = find(Some(configured));
            assert!(safe(&found), "configured {configured:?} gave {found:?}");
        }
        // SAFETY: this child process runs only this test.
        unsafe { std::env::set_var("PATH", "/usr/bin:/bin") };
        use_core(PathBuf::from("h0x-core"));
        assert_eq!(path_with_core(), None, "a relative core put a folder on PATH");
        // A relative install folder is refused before anything is downloaded.
        // SAFETY: this child process runs only this test.
        unsafe { std::env::set_var("H0X_CORE_INSTALL_DIR", "relative/bin") };
        let error = install().unwrap_err();
        assert!(error.contains("absolute") && error.contains("relative/bin"), "{error}");
    }

    #[test]
    fn the_core_installer_never_sees_a_releases_site_in_a_release_build() {
        assert_eq!(releases_site_vars(false), ["H0X_CORE_RELEASES_URL", "H0X_ADE_RELEASES_URL"]);
        assert!(releases_site_vars(true).is_empty());
    }

    #[test]
    fn only_the_exact_pinned_core_counts_as_installed() {
        assert!(is_pinned("h0x-core 0.1.13", "0.1.13"));
        assert!(is_pinned("h0x-core 0.1.13 ", "0.1.13"));
        for other in ["h0x-core 0.1.14", "h0x-core 0.1.130", "h0x-core 0.1.13-rc.1", "h0x-core 0.1.1", "0.1.13", ""] {
            assert!(!is_pinned(other, "0.1.13"), "{other:?}");
        }
    }

    #[test]
    fn startup_path_keeps_shell_order_and_falls_back_to_usual_dirs() {
        let usual = [PathBuf::from("/home/test/.local/bin"), PathBuf::from("/opt/homebrew/bin")];
        let current = std::env::join_paths(["/usr/bin", "/bin"]).unwrap();
        let login = std::env::join_paths(["/custom/bin", "/opt/homebrew/bin", "/usr/bin"]).unwrap();
        let merged = merged_path(Some(&login), &usual, &current).unwrap();
        assert_eq!(std::env::split_paths(&merged).collect::<Vec<_>>(), ["/custom/bin", "/opt/homebrew/bin", "/usr/bin", "/home/test/.local/bin", "/bin"].map(PathBuf::from));
        let fallback = merged_path(None, &usual, &current).unwrap();
        assert_eq!(std::env::split_paths(&fallback).collect::<Vec<_>>(), ["/home/test/.local/bin", "/opt/homebrew/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    }

    // Fakes the binary with a shell script.
    #[cfg(unix)]
    #[test]
    fn only_a_binary_that_says_h0x_core_counts() {
        // Runtime writes can leave a writer fd in a parallel fork, causing ETXTBSY.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core-bin");
        assert_eq!(version(&dir.join("missing")), None);
        let real = core_in(&dir);
        assert_eq!(version(&real).as_deref(), Some("h0x-core 0.1.0"));

        assert_eq!(find(Some(real.to_str().unwrap())), Some(real));
    }

    // Fakes the binary with a shell script (committed fixtures, see above).
    #[cfg(unix)]
    #[test]
    fn upstream_version_lines_and_failing_cores_are_not_accepted() {
        use std::os::unix::fs::PermissionsExt;
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core-bin");
        for (dir, says) in [("says-hc", "hc 0.1.0"), ("says-herdr-core", "herdr-core 0.1.0"), ("exits-1", "h0x-core 0.1.0 (exit 1)")] {
            let bin = core_in(&fixtures.join(dir));
            // A fixture without its exec bit would be rejected for the wrong reason.
            assert_ne!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o111, 0, "{} must be executable", bin.display());
            assert_eq!(version(&bin), None, "a core saying {says}");
        }
    }

    #[test]
    fn the_core_is_looked_for_only_as_h0x_core() {
        let windows = core_in(Path::new(r"C:\Users\me\AppData\Local\Programs\h0x-core"));
        assert_eq!(windows.file_name().unwrap(), format!("h0x-core{}", std::env::consts::EXE_SUFFIX).as_str());
        assert!(windows.to_string_lossy().starts_with(r"C:\Users\me\AppData\Local\Programs\h0x-core"));
        assert_eq!(core_in(Path::new("")), PathBuf::from(format!("h0x-core{}", std::env::consts::EXE_SUFFIX)));
        #[cfg(windows)]
        {
            let dirs = usual_dirs(Path::new(r"C:\Users\me"));
            assert!(dirs[0].ends_with(r"Programs\h0x-core"), "{}", dirs[0].display());
            assert!(core_in(&dirs[0]).ends_with(r"Programs\h0x-core\h0x-core.exe"));
            assert!(dirs.iter().all(|d| !d.to_string_lossy().contains("herdr")), "{dirs:?}");
        }
    }
}
