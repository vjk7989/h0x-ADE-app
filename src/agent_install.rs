//! Installing the free OpenCode CLI from onboarding (`installOpenCode()`).
//!
//! It runs here, not in the welcome mod, so it outlives the welcome screen: one
//! background thread, one fixed recipe, its toasts queued for the window.
//!
//! Package and URLs, checked against opencode.ai/docs and the npm registry on
//! 2026-11-13: the docs install `@opencode/cli` (OpenCode 2, bin `opencode`) and
//! `curl -fsSL https://opencode.ai/v2/install | bash`; its native binaries are the
//! optionalDependencies `@opencode/cli-<os>-<arch>`. (`opencode-ai` is OpenCode 1.)

use serde_json::{Value, json};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const PACKAGE: &str = "@opencode/cli";
const SCRIPT: &str = "set -o pipefail; curl -fsSL https://opencode.ai/v2/install | bash -s -- --no-modify-path";
const REGISTRY: &str = "https://registry.npmjs.org/";
const RC_LINE: &str = r#"export PATH="$HOME/.opencode/bin:$PATH""#;
const LIMIT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Os {
    Windows,
    Mac,
    Linux,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Method {
    Npm,
    /// The official install script (macOS, Linux).
    Script,
    /// Windows without npm: this registry package's binary, downloaded and checked here.
    Native(&'static str),
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Npm => "npm",
            Method::Script => "script",
            Method::Native(_) => "native",
        }
    }
}

fn this_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// The methods to try, in order; none when this machine has no build.
fn plan_for(os: Os, npm_present: bool, arch: &str) -> Vec<Method> {
    match os {
        Os::Windows if npm_present => vec![Method::Npm],
        Os::Windows => match arch {
            "x86_64" => vec![Method::Native("@opencode/cli-windows-x64")],
            "aarch64" => vec![Method::Native("@opencode/cli-windows-arm64")],
            _ => Vec::new(),
        },
        // A global npm folder the person can't write (EACCES) falls through to the script.
        Os::Mac | Os::Linux if npm_present => vec![Method::Npm, Method::Script],
        Os::Mac | Os::Linux => vec![Method::Script],
    }
}

/// `dist.integrity` (`sha512-<base64>`, possibly several hashes) against the file's SHA-512.
fn integrity_ok(integrity: &str, digest: &[u8]) -> Result<(), String> {
    use base64::Engine;
    let b64 = integrity.split_whitespace().find_map(|t| t.strip_prefix("sha512-")).ok_or("the registry gave no sha512 hash")?;
    let want = base64::engine::general_purpose::STANDARD.decode(b64).ok().filter(|w| w.len() == 64).ok_or("the registry's hash is malformed")?;
    if want == digest { Ok(()) } else { Err("the download does not match its hash".into()) }
}

fn sha512_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path)?;
    let mut hash = sha2::Sha512::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hash.finalize().to_vec());
        }
        hash.update(&buf[..n]);
    }
}

/// The tarball URL and hash from a registry `.../latest` answer; only the registry's own host.
fn parse_meta(text: &str) -> Result<(String, String), String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "the registry's answer is not JSON")?;
    let tarball = v["dist"]["tarball"].as_str().filter(|t| t.starts_with(REGISTRY)).ok_or("the registry gave no tarball")?;
    let integrity = v["dist"]["integrity"].as_str().ok_or("the registry gave no hash")?;
    Ok((tarball.to_owned(), integrity.to_owned()))
}

/// Whether `dir` is in a PATH list (Windows: any case, `/` or `\`, no matter a trailing slash).
fn path_has(list: &str, dir: &str, windows: bool) -> bool {
    let norm = |s: &str| {
        let s = s.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        if windows { s.replace('/', "\\").to_lowercase() } else { s.to_owned() }
    };
    let want = norm(dir);
    list.split(if windows { ';' } else { ':' }).any(|p| !p.trim().is_empty() && norm(p) == want)
}

/// The shell rc file's new text, or None when it already puts OpenCode's folder on PATH
/// (a non-comment line that names PATH and the folder; an alias to the binary does not).
fn rc_update(existing: &str) -> Option<String> {
    if existing.lines().any(|l| !l.trim_start().starts_with('#') && l.contains(".opencode/bin") && l.to_ascii_lowercase().contains("path")) {
        return None;
    }
    let mut text = existing.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(RC_LINE);
    text.push('\n');
    Some(text)
}

/// One line for a toast: the first that names an error (`error`, or npm 9's `ERR!`), else the last that says anything.
fn failure_line(output: &str) -> String {
    let lines = || output.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines().find(|l| { let l = l.to_lowercase(); l.contains("error") || l.contains("err!") }).or_else(|| lines().last()).unwrap_or("no details in the log");
    line.chars().take(120).collect()
}

/// The failure toast's button: the log, opened by the system's own viewer in a terminal.
fn failure_actions(log: &Path, os: Os) -> Value {
    let log = log.to_string_lossy().into_owned();
    let log = log.as_str();
    let argv = match os {
        Os::Windows => vec!["notepad.exe", log],
        Os::Mac => vec!["open", "-t", log],
        Os::Linux => vec!["xdg-open", log],
    };
    json!([{ "label": "Show log", "name": "terminal.open", "args": { "argv": argv, "name": "OpenCode install log" } }])
}

static TOASTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Notifications for the window to show (`notify` objects).
pub fn take_toasts() -> Vec<Value> {
    std::mem::take(&mut *TOASTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

fn toast(level: &str, body: &str, actions: Value) {
    let n = json!({ "title": "OpenCode", "body": body, "level": level, "actions": actions });
    TOASTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(n);
}

static RUNNING: AtomicBool = AtomicBool::new(false);

struct Done;

impl Drop for Done {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Starts the install on a background thread: "started", or "running" while one is.
pub fn start() -> &'static str {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return "running";
    }
    std::thread::spawn(|| {
        let _done = Done;
        install(&crate::mods::app_dir().join("logs").join("opencode-install.log"));
    });
    "started"
}

fn have(command: &str) -> bool {
    !crate::programs::installed(&[command.to_owned()], None).is_empty()
}

fn install(log: &Path) {
    if have("opencode") {
        toast("info", "OpenCode is already installed. Type `opencode` in a terminal.", json!([]));
        return;
    }
    toast("info", "Installing OpenCode…", json!([]));
    if std::fs::metadata(log).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::remove_file(log);
    }
    let plan = plan_for(this_os(), have("npm"), std::env::consts::ARCH);
    let mut tried = "none";
    let mut result = Err("OpenCode has no build for this system".to_owned());
    for method in plan {
        tried = method.name();
        result = attempt(method, log);
        let fall_through = matches!(&result, Err(why) if method == Method::Npm && this_os() != Os::Windows && why.contains("EACCES"));
        if !fall_through {
            break;
        }
    }
    let result = result.and_then(|()| if have("opencode") { Ok(()) } else { Err("installed, but not on PATH yet: open a new terminal".to_owned()) });
    crate::mods::forget_known_agents();
    crate::analytics::event("agent_install", json!({ "agent": "opencode", "method": tried, "ok": result.is_ok() }));
    match result {
        Ok(()) => toast(
            "success",
            "Installed. Open a new terminal, type `opencode`, then `/connect` to pick a model.",
            json!([{ "label": "Open OpenCode", "name": "terminal.open", "args": { "argv": ["opencode"], "name": "OpenCode" } }]),
        ),
        Err(why) => toast("error", &format!("OpenCode couldn't be installed: {}", failure_line(&why)), failure_actions(log, this_os())),
    }
}

/// `program args...` at low priority: nice(1), or Windows' below-normal class.
fn command(program: impl AsRef<OsStr>, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        let mut c = crate::spawn::command(program);
        c.args(args).creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("nice");
        c.args(["-n", "10"]).arg(program).args(args);
        c
    }
}

/// Runs it with its output appended to the log, 5 minutes at most. The error is
/// what it printed (or why it did not run).
fn run(mut command: Command, log: &Path) -> Result<(), String> {
    std::fs::create_dir_all(log.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let start = std::fs::metadata(log).map(|m| m.len()).unwrap_or(0);
    let out = std::fs::OpenOptions::new().create(true).append(true).open(log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let name = command.get_program().to_string_lossy().into_owned();
    let mut child = command.stdin(Stdio::null()).stdout(out).stderr(err).spawn().map_err(|e| format!("cannot start {name}: {e}"))?;
    let deadline = Instant::now() + LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out after 5 minutes".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut text = Vec::new();
    if let Ok(mut f) = std::fs::File::open(log) {
        let _ = std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(start));
        let _ = f.read_to_end(&mut text);
    }
    let text = String::from_utf8_lossy(&text).into_owned();
    Err(if text.trim().is_empty() { format!("{name} failed ({status})") } else { text })
}

fn attempt(method: Method, log: &Path) -> Result<(), String> {
    match method {
        Method::Npm => {
            let line = format!("npm i -g {PACKAGE}");
            let shell = crate::window::shell();
            if cfg!(windows) {
                // The core refreshes PATH from the registry: npm may be newer than this app.
                let core = crate::core_bin::path();
                let script = format!("{line}; exit $LASTEXITCODE");
                run(command(core, &["exec", "--", &shell, "-NoLogo", "-NonInteractive", "-Command", &script]), log)
            } else {
                run(command(&shell, &["-ic", &line]), log)
            }
        }
        Method::Script => {
            run(command("bash", &["-c", SCRIPT]), log)?;
            let rc = crate::mods::home().join(if this_os() == Os::Mac { ".zshrc" } else { ".bashrc" });
            let existing = std::fs::read_to_string(&rc).unwrap_or_default();
            if let Some(new) = rc_update(&existing) {
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&rc).map_err(|e| format!("cannot update {}: {e}", rc.display()))?;
                f.write_all(new[existing.len()..].as_bytes()).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Method::Native(package) => {
            let work = crate::mods::app_dir().join("tmp").join("opencode-install");
            let _ = std::fs::remove_dir_all(&work);
            std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
            let result = native(package, &work, log);
            let _ = std::fs::remove_dir_all(&work);
            result
        }
    }
}

fn find_exe(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe(&path) {
                return Some(found);
            }
        } else if entry.file_name() == "opencode.exe" {
            return Some(path);
        }
    }
    None
}

/// Windows without npm: the registry's own binary package, checked against its hash
/// before it is unpacked; copied to `%USERPROFILE%\.opencode\bin`, which joins the user PATH.
fn native(package: &str, work: &Path, log: &Path) -> Result<(), String> {
    let curl = crate::update::system("curl");
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let meta = work.join("latest.json");
    let url = format!("{REGISTRY}{}/latest", package.replace('/', "%2f"));
    run(command(&curl, &["-fsSL", "--proto", "=https", "--max-time", "60", "-o", &s(&meta), &url]), log)?;
    let (tarball, integrity) = parse_meta(&std::fs::read_to_string(&meta).map_err(|e| e.to_string())?)?;
    let pkg = work.join("package.tgz");
    run(command(&curl, &["-fsSL", "--proto", "=https", "--max-time", "280", "-o", &s(&pkg), &tarball]), log)?;
    integrity_ok(&integrity, &sha512_file(&pkg).map_err(|e| e.to_string())?)?;
    let unpacked = work.join("unpacked");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    run(command(crate::update::system("tar"), &["-xzf", &s(&pkg), "-C", &s(&unpacked)]), log)?;
    let exe = find_exe(&unpacked).ok_or("the package holds no opencode.exe")?;
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(crate::mods::home);
    let bin = profile.join(".opencode").join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let partial = bin.join("opencode.exe.new");
    std::fs::copy(&exe, &partial).map_err(|e| format!("cannot copy to {}: {e}", bin.display()))?;
    std::fs::rename(&partial, bin.join("opencode.exe")).map_err(|e| format!("cannot replace opencode.exe: {e}"))?;
    add_user_path(&s(&bin), log)
}

/// PowerShell that prints the user PATH as stored (`%VAR%` entries unexpanded) and expanded.
/// The markers tell a real (maybe empty) PATH from a failed read: never write over one.
const READ_USER_PATH: &str = "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); \
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment'); \
    $raw = [string]$k.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); \
    'h0x-path:' + $raw; 'h0x-expanded:' + [Environment]::ExpandEnvironmentVariables($raw)";

/// PowerShell that stores the user PATH as REG_EXPAND_SZ, so `%VAR%` entries stay as they are,
/// then broadcasts the change (setting and clearing a throwaway user variable sends WM_SETTINGCHANGE).
fn write_user_path_script(new: &str) -> String {
    format!(
        "$k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true); \
        $k.SetValue('Path', {}, [Microsoft.Win32.RegistryValueKind]::ExpandString); $k.Close(); \
        [Environment]::SetEnvironmentVariable('H0X_PATH_REFRESH', '1', 'User'); \
        [Environment]::SetEnvironmentVariable('H0X_PATH_REFRESH', $null, 'User')",
        crate::programs::powershell_quote(new)
    )
}

/// Adds the folder (as a literal path) to the user PATH (no admin), unless it is there.
fn add_user_path(dir: &str, log: &Path) -> Result<(), String> {
    let root = std::env::var_os("SystemRoot").ok_or("no SystemRoot")?;
    let powershell = PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let out = crate::mods::run_here(&powershell.to_string_lossy(), &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), READ_USER_PATH.into()]).ok_or("cannot read the user PATH")?;
    let field = |marker: &str| out.lines().find_map(|l| l.trim_end().strip_prefix(marker));
    let raw = field("h0x-path:").ok_or("cannot read the user PATH")?;
    let expanded = field("h0x-expanded:").ok_or("cannot read the user PATH")?;
    if path_has(raw, dir, true) || path_has(expanded, dir, true) {
        return Ok(());
    }
    let new = if raw.trim().is_empty() { dir.to_owned() } else { format!("{};{dir}", raw.trim_end_matches(';')) };
    run(command(&powershell, &["-NoProfile", "-NonInteractive", "-Command", &write_user_path_script(&new)]), log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::Digest;

    #[test]
    fn plan_for_each_system() {
        let (win, mac, lin) = (Os::Windows, Os::Mac, Os::Linux);
        assert_eq!(plan_for(win, true, "x86_64"), vec![Method::Npm]);
        assert_eq!(plan_for(win, true, "aarch64"), vec![Method::Npm]);
        assert_eq!(plan_for(win, false, "x86_64"), vec![Method::Native("@opencode/cli-windows-x64")]);
        assert_eq!(plan_for(win, false, "aarch64"), vec![Method::Native("@opencode/cli-windows-arm64")]);
        assert!(plan_for(win, false, "x86").is_empty());
        for os in [mac, lin] {
            assert_eq!(plan_for(os, true, "aarch64"), vec![Method::Npm, Method::Script]);
            assert_eq!(plan_for(os, false, "x86_64"), vec![Method::Script]);
        }
    }

    #[test]
    fn integrity_matches_mismatches_and_malformed() {
        let digest = sha2::Sha512::digest(b"opencode").to_vec();
        let good = format!("sha512-{}", base64::engine::general_purpose::STANDARD.encode(&digest));
        assert!(integrity_ok(&good, &digest).is_ok());
        assert!(integrity_ok(&format!("sha1-abc {good}"), &digest).is_ok(), "the sha512 among several");
        let other = sha2::Sha512::digest(b"other").to_vec();
        assert!(integrity_ok(&good, &other).unwrap_err().contains("does not match"));
        for bad in ["", "sha1-abcd", "sha512-", "sha512-%%%", "sha512-AAAA", &good[..good.len() - 4]] {
            assert!(integrity_ok(bad, &digest).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_hash_of_a_file_is_its_sha512() {
        let dir = std::env::temp_dir().join(format!("h0x-oc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        std::fs::write(&file, vec![7u8; 200_000]).unwrap();
        assert_eq!(sha512_file(&file).unwrap(), sha2::Sha512::digest(vec![7u8; 200_000]).to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_metadata_must_name_a_tarball_on_the_registry() {
        let ok = r#"{"dist":{"tarball":"https://registry.npmjs.org/@opencode/cli-windows-x64/-/x.tgz","integrity":"sha512-AA=="}}"#;
        assert_eq!(parse_meta(ok).unwrap().1, "sha512-AA==");
        for bad in ["", "{}", r#"{"dist":{"tarball":"https://evil.example/x.tgz","integrity":"sha512-AA=="}}"#, r#"{"dist":{"tarball":"https://registry.npmjs.org/x.tgz"}}"#] {
            assert!(parse_meta(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn path_detection_ignores_case_slashes_and_quotes_on_windows_only() {
        let list = r#"C:\Windows;"C:\Users\Me\.OpenCode\bin\";D:\x"#;
        for dir in [r"C:\Users\Me\.opencode\bin", r"c:/users/me/.opencode/bin/", r"C:\Users\Me\.opencode\bin\\"] {
            assert!(path_has(list, dir, true), "{dir}");
        }
        assert!(!path_has(list, r"C:\Users\Me\.opencode", true));
        assert!(!path_has("", r"C:\x", true));
        assert!(!path_has(r"C:\Users\Me\.OpenCode\bin", r"C:\Users\Me\.opencode\bin", false), "case counts elsewhere");
        assert!(path_has("/usr/bin:/home/me/.opencode/bin/", "/home/me/.opencode/bin", false));
    }

    #[test]
    fn the_user_path_is_written_unexpanded_and_broadcast() {
        let script = write_user_path_script(r"%USERPROFILE%\bin;C:\it's");
        assert!(script.contains(r"'%USERPROFILE%\bin;C:\it''s'"), "{script}");
        assert!(script.contains("RegistryValueKind]::ExpandString"), "{script}");
        assert!(!script.contains("SetEnvironmentVariable('Path'"), "{script}");
        assert!(script.matches("H0X_PATH_REFRESH").count() == 2, "{script}");
        assert!(READ_USER_PATH.contains("DoNotExpandEnvironmentNames") && READ_USER_PATH.contains("h0x-path:"));
    }

    #[test]
    fn the_rc_line_is_added_once() {
        let first = rc_update("alias a=b").unwrap();
        assert_eq!(first, format!("alias a=b\n{RC_LINE}\n"));
        assert_eq!(rc_update(&first), None);
        assert_eq!(rc_update("export PATH=$HOME/.opencode/bin:$PATH\n"), None, "any spelling of the folder counts");
        assert_eq!(rc_update(""), Some(format!("{RC_LINE}\n")));
        assert!(rc_update("# export PATH=\"$HOME/.opencode/bin\"\n").is_some(), "a comment is not a PATH line");
    }

    #[test]
    fn a_failure_toast_gets_one_short_line() {
        assert_eq!(failure_line("added 1 package\nnpm error code EACCES\nnpm error path /usr\n"), "npm error code EACCES");
        assert_eq!(failure_line("\n  one\ntwo  \n"), "two");
        assert_eq!(failure_line(" \n"), "no details in the log");
        assert_eq!(failure_line(&"x".repeat(500)).chars().count(), 120);
    }

    #[test]
    fn the_failure_toast_offers_the_log() {
        let log = Path::new("/h/logs/opencode-install.log");
        for (os, program) in [(Os::Windows, "notepad.exe"), (Os::Mac, "open"), (Os::Linux, "xdg-open")] {
            let a = failure_actions(log, os);
            assert_eq!(a[0]["label"], "Show log");
            assert_eq!(a[0]["name"], "terminal.open");
            let argv = a[0]["args"]["argv"].as_array().unwrap();
            assert_eq!(argv[0], program);
            assert_eq!(argv.last().unwrap(), "/h/logs/opencode-install.log");
        }
    }

    #[test]
    fn a_second_start_while_one_runs_says_so() {
        RUNNING.store(true, Ordering::SeqCst);
        assert_eq!(start(), "running");
        RUNNING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn plan_for_odd_architectures_and_method_names() {
        for arch in ["", "x86", "arm", "X86_64", "x86_64 ", "riscv64"] {
            assert!(plan_for(Os::Windows, false, arch).is_empty(), "{arch:?}");
            assert_eq!(plan_for(Os::Windows, true, arch), vec![Method::Npm], "npm builds for any arch: {arch:?}");
            assert_eq!(plan_for(Os::Linux, false, arch), vec![Method::Script], "the script decides: {arch:?}");
        }
        assert_eq!((Method::Npm.name(), Method::Script.name(), Method::Native("x").name()), ("npm", "script", "native"));
    }

    #[test]
    fn integrity_tolerates_spacing_but_only_a_padded_standard_sha512() {
        let b64 = base64::engine::general_purpose::STANDARD;
        let digest = sha2::Sha512::digest(b"opencode").to_vec();
        let good = format!("sha512-{}", b64.encode(&digest));
        for spaced in [format!("  {good}  "), format!("\t{good}\n"), format!("sha256-AAAA\n\t{good}"), format!("sha1-x   sha256-y {good} md5-z")] {
            assert!(integrity_ok(&spaced, &digest).is_ok(), "{spaced:?}");
        }
        let sha256 = format!("sha256-{}", b64.encode(sha2::Sha256::digest(b"opencode")));
        assert!(integrity_ok(&sha256, &sha2::Sha256::digest(b"opencode")).unwrap_err().contains("no sha512"), "a sha256 alone is not enough");
        let unpadded = good.trim_end_matches('=').to_owned();
        let url_safe = format!("sha512-{}", base64::engine::general_purpose::URL_SAFE.encode(&digest));
        let bad = [
            unpadded,
            good.to_uppercase(),
            good.replacen("sha512-", "SHA512-", 1),
            good.replacen("sha512-", "sha512 -", 1),
            good.replacen("sha512-", "sha512=", 1),
            format!("{good}?opt"),
            format!("{good}=="),
            format!("sha512-{}", b64.encode(&digest[..32])),
            format!("sha512-{}", b64.encode([&digest[..], &[0u8][..]].concat())),
        ];
        for bad in &bad {
            assert!(integrity_ok(bad, &digest).is_err(), "{bad:?}");
        }
        if url_safe != good {
            assert!(integrity_ok(&url_safe, &digest).is_err(), "{url_safe:?}");
        }
        assert!(integrity_ok(&good, &[]).unwrap_err().contains("does not match"), "an empty digest");
        assert!(integrity_ok(&good, &digest[..63]).is_err());
    }

    #[test]
    fn the_hash_of_an_empty_file_and_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("h0x-oc-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("empty");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(sha512_file(&file).unwrap(), sha2::Sha512::digest(b"").to_vec());
        // Exactly one buffer, and one byte past it.
        for len in [64 * 1024, 64 * 1024 + 1] {
            std::fs::write(&file, vec![1u8; len]).unwrap();
            assert_eq!(sha512_file(&file).unwrap(), sha2::Sha512::digest(vec![1u8; len]).to_vec(), "{len}");
        }
        assert!(sha512_file(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_metadata_refuses_other_hosts_plain_http_and_odd_shapes() {
        let meta = |tarball: &str| format!(r#"{{"dist":{{"tarball":"{tarball}","integrity":"sha512-AA=="}}}}"#);
        for tarball in [
            "http://registry.npmjs.org/@opencode/cli-windows-x64/-/x.tgz",
            "https://registry.npmjs.org.evil.example/x.tgz",
            "https://registry.npmjs.org@evil.example/x.tgz",
            "https://registry.npmjs.org:443@evil.example/x.tgz",
            "https://REGISTRY.npmjs.org/x.tgz",
            " https://registry.npmjs.org/x.tgz",
            "https://registry.yarnpkg.com/x.tgz",
            "file:///C:/x.tgz",
            "https://registry.npmjs.org",
            "",
        ] {
            assert!(parse_meta(&meta(tarball)).is_err(), "{tarball:?}");
        }
        for bad in [
            "null",
            "[]",
            "\"dist\"",
            r#"{"dist":null}"#,
            r#"{"dist":"https://registry.npmjs.org/x.tgz"}"#,
            r#"{"dist":{"tarball":7,"integrity":"sha512-AA=="}}"#,
            r#"{"dist":{"tarball":"https://registry.npmjs.org/x.tgz","integrity":null}}"#,
            r#"{"dist":{"tarball":"https://registry.npmjs.org/x.tgz","integrity":["sha512-AA=="]}}"#,
            r#"{"tarball":"https://registry.npmjs.org/x.tgz","integrity":"sha512-AA=="}"#,
            r#"{"dist":{"tarball":"https://registry.npmjs.org/x.tgz","integrity":"sha512-AA=="}"#,
        ] {
            assert!(parse_meta(bad).is_err(), "{bad:?}");
        }
        let (tarball, integrity) = parse_meta(&meta("https://registry.npmjs.org/a/-/a-1.tgz")).unwrap();
        assert_eq!((tarball.as_str(), integrity.as_str()), ("https://registry.npmjs.org/a/-/a-1.tgz", "sha512-AA=="));
        // Extra fields and whitespace around the JSON are fine.
        let roomy = "\n  { \"name\": \"x\", \"dist\": { \"shasum\": \"ab\", \"tarball\": \"https://registry.npmjs.org/x.tgz\", \"integrity\": \"sha512-AA==\" } }\n";
        assert!(parse_meta(roomy).is_ok());
    }

    #[test]
    fn path_detection_with_empty_entries_env_vars_and_roots() {
        assert!(path_has(r";;C:\x;;", r"C:\x", true));
        assert!(!path_has(";;", "", true), "an empty folder is never on PATH");
        assert!(!path_has(r"C:\x;;D:\y", "", true));
        assert!(!path_has("::", "", false));
        assert!(!path_has("/usr/bin::/bin", "", false));
        assert!(path_has(r"  C:\x  ;D:\y", r"C:\x", true), "blanks around an entry");
        assert!(path_has(r#"C:\a;"C:\Program Files\x\";D:\y"#, r"C:\Program Files\x", true));
        // Stored unexpanded, the entry is not the expanded folder: the expanded list catches it.
        let raw = r"%USERPROFILE%\.opencode\bin;C:\Windows";
        assert!(!path_has(raw, r"C:\Users\Me\.opencode\bin", true));
        assert!(path_has(&raw.replace("%USERPROFILE%", r"C:\Users\Me"), r"C:\Users\Me\.opencode\bin", true));
        assert!(path_has(raw, r"%userprofile%\.OpenCode\bin\", true), "the same unexpanded spelling");
        assert!(path_has("C:\\Users\\\u{c4}nne\\.opencode\\bin", "c:\\users\\\u{e4}nne\\.opencode\\bin", true), "non-ASCII case");
        assert!(!path_has(r"C:\Users\Me\.opencode\bin;C:\x", r"C:\Users\Me\.opencode", true), "a parent is not the folder");
        assert!(!path_has(r"C:\Users\Me\.opencode\bin2", r"C:\Users\Me\.opencode\bin", true), "a prefix is not the folder");
        assert!(!path_has(r"C:\Users\Me\.opencode\bin", r"C:\Users\Me\.opencode\bin2", true));
        assert!(path_has("/usr/bin:/", "/", false), "the root folder");
        assert!(!path_has("/home/me/.opencode/bin", "/home/me/.opencode/bin;", false));
    }

    #[test]
    fn rc_update_only_appends_and_keeps_crlf_and_unterminated_files() {
        for existing in ["", "\n", "alias a=b", "alias a=b\n", "alias a=b\r\n", "a\r\nb\r\n", "a\rb", "export X=\u{e9}\u{4f60}", "   ", "# .opencode/bin is mentioned only in a comment"] {
            let new = rc_update(existing).unwrap_or_else(|| panic!("{existing:?}"));
            assert!(new.starts_with(existing), "the old text stays first: {existing:?}");
            let added = &new[existing.len()..];
            assert!(added.ends_with(&format!("{RC_LINE}\n")), "{existing:?}: {added:?}");
            assert!(added == format!("{RC_LINE}\n") || added == format!("\n{RC_LINE}\n"), "{existing:?}: {added:?}");
            assert_eq!(new.matches(RC_LINE).count(), 1, "{existing:?}");
            assert!(new.lines().any(|l| l == RC_LINE), "the line stands alone: {existing:?}");
            assert_eq!(rc_update(&new), None, "added once: {existing:?}");
        }
        assert_eq!(rc_update("alias a=b\r\n").unwrap(), format!("alias a=b\r\n{RC_LINE}\n"));
        assert_eq!(rc_update("x").unwrap(), format!("x\n{RC_LINE}\n"));
        assert!(rc_update("    # export PATH=$HOME/.opencode/bin:$PATH").is_some(), "an indented comment");
        assert_eq!(rc_update("  export PATH=\"$HOME/.opencode/bin:$PATH\"\r\n"), None, "indented, CRLF");
        assert_eq!(rc_update("path+=(~/.opencode/bin)"), None, "zsh's path array");
    }

    /// A line that only mentions the folder (an alias to the binary) does not put it on PATH.
    #[test]
    fn rc_update_is_not_fooled_by_an_alias_to_the_binary() {
        assert!(rc_update("alias oc=\"$HOME/.opencode/bin/opencode\"\n").is_some());
    }

    #[test]
    fn failure_line_with_crlf_unicode_and_huge_logs() {
        assert_eq!(failure_line(""), "no details in the log");
        assert_eq!(failure_line("\r\n\r\n\t \n"), "no details in the log");
        assert_eq!(failure_line("one\r\ntwo\r\n"), "two", "a CR is no detail");
        assert_eq!(failure_line("ok\r\nERROR: disk full\r\nbye\r\n"), "ERROR: disk full", "any case");
        let wide = "\u{e9}".repeat(500);
        assert_eq!(failure_line(&wide), "\u{e9}".repeat(120), "cut on characters, not bytes");
        assert_eq!(failure_line(&"\u{1f600}".repeat(130)).chars().count(), 120);
        let mut huge = "progress line\n".repeat(200_000);
        huge.push_str("npm error code E404\n");
        huge.push_str(&"trailing\n".repeat(100_000));
        assert_eq!(failure_line(&huge), "npm error code E404");
        assert_eq!(failure_line(&"y".repeat(5_000_000)).len(), 120);
    }

    /// npm before 10 prints `npm ERR!`: that names the error as much as `npm error` does.
    #[test]
    fn failure_line_takes_old_npm_err_lines_as_errors() {
        let log = "npm ERR! code EACCES\nnpm ERR! syscall mkdir\nnpm ERR! A complete log of this run can be found in: C:\\x.log\n";
        assert_eq!(failure_line(log), "npm ERR! code EACCES");
    }

    #[test]
    fn the_user_path_script_quotes_every_odd_path() {
        assert!(write_user_path_script("").contains("$k.SetValue('Path', '', "), "an empty PATH is a quoted empty string");
        for odd in ["C:\\a'b", "C:\\\u{2019}x\u{2018}", "$(calc)", "C:\\x`;y", "C:\\a;\"b\";%TEMP%"] {
            let script = write_user_path_script(odd);
            assert!(script.contains(&crate::programs::powershell_quote(odd)), "{odd:?}: {script}");
            if odd.contains(['\'', '\u{2018}', '\u{2019}']) {
                assert!(!script.contains(&format!("'{odd}'")), "a quote never closes the string: {odd:?}");
            }
        }
        assert!(READ_USER_PATH.contains("'h0x-expanded:'") && READ_USER_PATH.contains("ExpandEnvironmentVariables"));
    }
}
