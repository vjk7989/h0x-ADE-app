//! End to end: `h0x-ade` typed in a terminal opens HyperOx in that folder, a second
//! launch from another folder shows that folder in the running window, the same
//! folder twice makes no second space, and a launch that is not from a shell
//! changes nothing (LF plan AC1-AC4, AC6). The terminal gets only the one line, and
//! on Linux and macOS the launch exits at once while the window runs on in its own
//! session, with no terminal (LF-QUIET, LF-DETACH).
//!
//! It runs the real app, so it needs a desktop and h0x-core: CI's `launch-e2e` job
//! sets `H0X_E2E=1` and `H0X_E2E_CORE_ARCHIVE` (the h0x-core release archive, its
//! `.minisig` beside it). The archive is checked against the release key before
//! anything in it runs. Run by hand only with an isolated machine in mind:
//! `H0X_E2E=1 H0X_E2E_CORE_ARCHIVE=... cargo test --test launch_folder -- --ignored --nocapture`.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_h0x-ade");
/// The release key every app and core release file is signed with (src/update.rs).
const PUBLIC_KEY: &str = "RWTEdqtOBn6VKQ/x9x3qWk/gIWUb/I1OlZXsTlHyrkp9Yq30qBki+tdt";
const NAME: &str = "a_shell_launch_opens_hyperox_in_its_folder";
/// Windows: set in the helper process that launches HyperOx with no console and no std handles.
const DETACHED: &str = "H0X_E2E_DETACHED_LAUNCH";

/// One isolated HyperOx: its own home, core session and core.
struct Isolated {
    root: PathBuf,
    home: PathBuf,
    session: String,
    core_dir: PathBuf,
}

impl Isolated {
    fn apply(&self, cmd: &mut Command) {
        cmd.env("H0X_ADE_HOME", &self.home)
            .env("H0X_CORE_SESSION", &self.session)
            .env("H0X_ADE_SKIP_SIGNIN", "1")
            .env("H0X_CORE_DIRS", &self.core_dir)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env_remove("H0X_TERMINAL_ID")
            .env_remove("H0X_TERMINAL_SOCKET")
            .env_remove("H0X_SCOPE");
    }

    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        self.apply(&mut cmd);
        cmd
    }

    fn core(&self) -> PathBuf {
        self.core_dir.join(format!("h0x-core{}", std::env::consts::EXE_SUFFIX))
    }

    /// `h0x-ade ls`, once the window answers.
    fn ls(&self) -> Option<Value> {
        let out = self.command(EXE).arg("ls").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| serde_json::from_slice(&out.stdout).ok()).flatten()
    }
}

/// HyperOx started as from a terminal in `dir`: under a pty on Linux and macOS
/// (`script`), with its output piped on Windows (which counts as a shell there).
fn in_shell(env: &Isolated, dir: &Path) -> Command {
    let mut cmd = if cfg!(target_os = "linux") {
        let mut c = env.command("script");
        // The path goes through the environment: no quoting of it in a shell line.
        c.args(["-qec", "exec \"$H0X_E2E_EXE\"", "/dev/null"]).env("H0X_E2E_EXE", EXE);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = env.command("script");
        c.args(["-q", "/dev/null", EXE]);
        c
    } else {
        env.command(EXE)
    };
    cmd.current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    cmd
}

/// A running launch whose output is collected as it comes.
struct Launch {
    child: Child,
    _stdin: Option<ChildStdin>,
    out: Arc<Mutex<String>>,
}

impl Launch {
    fn start(mut cmd: Command) -> Launch {
        let mut child = cmd.spawn().expect("cannot start h0x-ade");
        // Held open: `script` must not see the end of its input.
        let stdin = child.stdin.take();
        let out = Arc::new(Mutex::new(String::new()));
        if let Some(stdout) = child.stdout.take() {
            let sink = out.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = Vec::new();
                while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                    sink.lock().unwrap().push_str(&String::from_utf8_lossy(&line));
                    line.clear();
                }
            });
        }
        Launch { child, _stdin: stdin, out }
    }

    fn output(&self) -> String {
        self.out.lock().unwrap().clone()
    }

    /// Waits up to `secs` for it to exit; killed and None if it doesn't.
    fn wait(&mut self, secs: u64) -> Option<ExitStatus> {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            if let Ok(Some(status)) = self.child.try_wait() {
                // Let the reader take the last of the output.
                std::thread::sleep(Duration::from_millis(300));
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        None
    }
}

/// What a terminal shows of `out`: no `\r`, colour or title escapes or other
/// control characters, no blank lines, and none of `script`'s own lines.
fn terminal_lines(out: &str) -> Vec<String> {
    let mut text = String::new();
    let mut chars = out.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: up to its final byte.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\n' || !c.is_control() {
            text.push(c);
        }
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "^D" && !l.starts_with("Script started") && !l.starts_with("Script done"))
        .map(str::to_owned)
        .collect()
}

/// Linux and macOS: the window this test started, a detached copy found by the
/// test's folder in its hidden `--launched-in` argument (the app writes no pid).
#[cfg(unix)]
fn window_pids(env: &Isolated) -> Vec<u32> {
    let pattern = format!("launched-in .*{}/", env.root.file_name().unwrap().to_str().unwrap());
    let out = Command::new("pgrep").args(["-f", &pattern]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|p| p.parse().ok()).collect()).unwrap_or_default()
}

/// Stops whatever is left, also when an assertion failed half way.
struct Cleanup<'a> {
    env: &'a Isolated,
    /// The first launch: the window itself on Windows, on Linux and macOS only
    /// the launcher (the window is then found by `window_pids`).
    launch: Option<Launch>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if let Some(mut launch) = self.launch.take() {
            let _ = launch.child.kill();
            let _ = launch.child.wait();
        }
        #[cfg(unix)]
        for pid in window_pids(self.env) {
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).stdin(Stdio::null()).status();
        }
        let _ = self.env.command(self.env.core()).args(["server", "stop"]).stdin(Stdio::null()).output();
        let _ = std::fs::remove_dir_all(&self.env.root);
    }
}

/// Whether `folder` (as HyperOx reports it) names `dir`: the real folder on
/// Linux and macOS (/tmp is /private/tmp there); on Windows case and a
/// trailing separator don't count.
fn names(folder: &str, dir: &Path) -> bool {
    if cfg!(windows) {
        let key = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
        key(folder) == key(dir.to_str().unwrap())
    } else {
        std::fs::canonicalize(folder).ok() == std::fs::canonicalize(dir).ok()
    }
}

/// The folder as the launched process sees it (`getcwd`), as printed in its line.
fn seen(dir: &Path) -> String {
    if cfg!(windows) { dir.to_str().unwrap().to_owned() } else { std::fs::canonicalize(dir).unwrap().to_str().unwrap().to_owned() }
}

fn spaces(world: &Value) -> Vec<Value> {
    world["spaces"].as_array().cloned().unwrap_or_default()
}

fn local_spaces_in(world: &Value, dir: &Path) -> Vec<Value> {
    spaces(world).into_iter().filter(|s| s["machine"].is_null() && s["folder"].as_str().is_some_and(|f| names(f, dir))).collect()
}

fn active(world: &Value) -> Option<Value> {
    spaces(world).into_iter().find(|s| s["active"] == Value::Bool(true))
}

/// Polls `ls` until `ready` holds, for up to `secs`; the last listing either way.
fn wait_for(env: &Isolated, secs: u64, ready: impl Fn(&Value) -> bool) -> Option<Value> {
    let until = Instant::now() + Duration::from_secs(secs);
    let mut last = None;
    while Instant::now() < until {
        if let Some(world) = env.ls() {
            if ready(&world) {
                return Some(world);
            }
            last = Some(world);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    last
}

/// The h0x-core release archive, checked against the release key, unpacked: the
/// folder with `h0x-core` in it. None (with why) when there is none to use.
fn core_from_archive(into: &Path) -> Result<PathBuf, String> {
    let archive = PathBuf::from(std::env::var_os("H0X_E2E_CORE_ARCHIVE").ok_or("H0X_E2E_CORE_ARCHIVE is not set")?);
    let bytes = std::fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let sig_path = PathBuf::from(format!("{}.minisig", archive.display()));
    let sig = std::fs::read_to_string(&sig_path).map_err(|e| format!("{}: {e}", sig_path.display()))?;
    let key = minisign_verify::PublicKey::from_base64(PUBLIC_KEY).unwrap();
    let signature = minisign_verify::Signature::decode(&sig).map_err(|e| format!("unreadable signature: {e}"))?;
    // A failed check is not a skip: the archive is not what it claims to be.
    key.verify(&bytes, &signature, false).expect("the h0x-core archive is not signed by the release key");
    let file = archive.file_name().unwrap().to_str().unwrap();
    assert!(signature.trusted_comment().split('\t').any(|f| f == format!("file:{file}")), "the signature is for another file than {file}");
    std::fs::create_dir_all(into).unwrap();
    let tar = if cfg!(windows) {
        // Windows' own tar (it reads zip), not one earlier on PATH.
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    };
    let status = Command::new(tar).arg("-xf").arg(&archive).arg("-C").arg(into).status().unwrap();
    assert!(status.success(), "cannot unpack {}", archive.display());
    let exe = format!("h0x-core{}", std::env::consts::EXE_SUFFIX);
    // The Windows zip has a folder (h0x-core.exe and conpty); the tarballs don't.
    [into.to_path_buf(), into.join("h0x-core")]
        .into_iter()
        .find(|d| d.join(&exe).is_file())
        .ok_or_else(|| format!("no {exe} in {}", archive.display()))
}

/// Windows: HyperOx as Explorer starts it: no std handles and no console to attach
/// to. This helper process is detached (no console) and passes its std handles on
/// after setting them to none; HyperOx's parent is then this helper.
#[cfg(windows)]
fn detached_launch() -> ! {
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe { SetStdHandle(which, std::ptr::null_mut()) };
    }
    // Inherited null handles stay null in the child.
    let mut child = Command::new(EXE).env_remove(DETACHED).spawn().expect("cannot start h0x-ade");
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if let Ok(Some(status)) = child.try_wait() {
            std::process::exit(status.code().unwrap_or(101));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    std::process::exit(102)
}

/// A launch that is not from a shell, in `dir`; its exit status and output.
fn not_from_shell(env: &Isolated, dir: &Path) -> (Option<ExitStatus>, String) {
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let mut c = env.command(std::env::current_exe().unwrap());
        c.args([NAME, "--exact", "--ignored", "--test-threads=1"]).env(DETACHED, "1").creation_flags(DETACHED_PROCESS);
        c
    };
    #[cfg(not(windows))]
    let cmd = env.command(EXE);
    let mut cmd = cmd;
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut launch = Launch::start(cmd);
    (launch.wait(90), launch.output())
}

#[test]
#[ignore = "runs the real app on a desktop with h0x-core: H0X_E2E=1 and H0X_E2E_CORE_ARCHIVE (CI's launch-e2e job)"]
fn a_shell_launch_opens_hyperox_in_its_folder() {
    #[cfg(windows)]
    if std::env::var_os(DETACHED).is_some() {
        detached_launch();
    }
    if std::env::var("H0X_E2E").as_deref() != Ok("1") {
        eprintln!("skipped: set H0X_E2E=1 (and H0X_E2E_CORE_ARCHIVE) to run the launch end-to-end test");
        return;
    }
    // Short on Unix: the window's socket path in the home must fit (104 bytes on macOS).
    let root = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") }.join(format!("h0x-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let core_dir = match core_from_archive(&root.join("core")) {
        Ok(dir) => dir,
        Err(why) => {
            let _ = std::fs::remove_dir_all(&root);
            eprintln!("skipped: no h0x-core to run against: {why}");
            return;
        }
    };
    let env = Isolated { home: root.join("home"), session: format!("e2e-launch-{}", std::process::id()), core_dir, root: root.clone() };
    // Folders as people have them: a space, `&` and a non-ASCII letter (AC6).
    let (a, b, c) = (root.join("proj a & \u{fc}"), root.join("b"), root.join("never opened"));
    for dir in [&env.home, &a, &b, &c] {
        std::fs::create_dir_all(dir).unwrap();
    }
    // Open and done tasks and steps for the To-do panel screenshot below (the app home's todo/
    // folder): the progress bar, step counts and the Completed count all have something to show.
    std::fs::create_dir_all(env.home.join("todo")).unwrap();
    std::fs::write(env.home.join("todo/lists.md"), "- [Tasks](tasks.md)\n").unwrap();
    std::fs::write(
        env.home.join("todo/tasks.md"),
        "# Tasks\n- [ ] Write the release notes\n  - [x] Collect the changes\n  - [x] Draft the summary\n  - [ ] Ask for a review\n- [ ] Ship HyperOx\n- [ ] Plan the next stage\n  - [ ] Ask the user\n- [x] Polish the to-do panel\n  - [x] Tick tasks and steps together\n  - [x] Strike done items\n- [x] Pick a handwritten font\n",
    )
    .unwrap();
    let mut cleanup = Cleanup { env: &env, launch: None };

    // AC1: not running; from a shell in A the window opens on a focused local space in A.
    #[cfg(unix)]
    let launched = Instant::now();
    cleanup.launch = Some(Launch::start(in_shell(&env, &a)));
    // LF-DETACH AC-D1, Linux and macOS: the launch lets go of the terminal at once (the
    // window is a detached copy). Windows: the launch is the window, a window app the
    // shell doesn't wait for, and it lets go of the console instead.
    #[cfg(unix)]
    {
        let launch = cleanup.launch.as_mut().unwrap();
        let status = launch.wait(20).unwrap_or_else(|| panic!("the first launch still holds the terminal after 20 s; output: {}", launch.output()));
        assert!(status.success(), "{status:?}: {}", launch.output());
        eprintln!("the first launch exited after {:?}", launched.elapsed());
    }
    let world = wait_for(&env, 240, |w| active(w).is_some_and(|s| s["machine"].is_null() && s["folder"].as_str().is_some_and(|f| names(f, &a))));
    let world = world.unwrap_or_else(|| panic!("no active space in {} after the first launch; output: {}", a.display(), cleanup.launch.as_ref().unwrap().output()));
    assert!(active(&world).is_some_and(|s| names(s["folder"].as_str().unwrap(), &a)), "{world:#}");
    // LF-QUIET AC-Q1: the window is up and logging by now; the terminal has the started line and nothing else.
    let started = format!("HyperOx started in {}.", seen(&a));
    let window_out = cleanup.launch.as_ref().unwrap().output();
    assert_eq!(terminal_lines(&window_out), [started.clone()], "expected only {started:?} in {window_out:?}");

    // LF-DETACH AC-D2, Linux and macOS: the terminal is gone (`script` exited and closed
    // its pty); the window has no controlling terminal, is in a session of its own,
    // and still answers.
    #[cfg(unix)]
    {
        let pids = window_pids(&env);
        assert!(!pids.is_empty(), "no detached window found after {:?}", launched.elapsed());
        for pid in &pids {
            let tty = Command::new("ps").args(["-o", "tty=", "-p", &pid.to_string()]).output().unwrap();
            let tty = String::from_utf8_lossy(&tty.stdout).trim().to_owned();
            assert!(tty == "?" || tty == "??", "window {pid} has the terminal {tty:?}");
        }
        #[cfg(target_os = "linux")]
        {
            // Field 6 of /proc/<pid>/stat; the 4th after `(comm)`, which may hold spaces.
            let session = |pid: &str| {
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
                stat.rsplit_once(')').unwrap().1.split_whitespace().nth(3).unwrap().to_owned()
            };
            let own = session("self");
            for pid in &pids {
                assert_ne!(session(&pid.to_string()), own, "window {pid} is in the test's session");
            }
            assert!(pids.iter().any(|p| session(&p.to_string()) == p.to_string()), "no window leads its own session: {pids:?}");
        }
        std::thread::sleep(Duration::from_secs(2));
        assert!(env.ls().is_some(), "the window went away with the terminal");
    }

    // AC2: running; from B the same window shows B, and the launch says so and exits 0.
    let mut second = Launch::start(in_shell(&env, &b));
    let status = second.wait(90).expect("the second launch did not exit");
    assert!(status.success(), "{status:?}: {}", second.output());
    // LF-QUIET AC-Q2: that line and nothing else.
    let running = format!("HyperOx is already running. Opened {}.", seen(&b));
    assert_eq!(terminal_lines(&second.output()), [running.clone()], "expected only {running:?} in {:?}", second.output());
    let world = wait_for(&env, 30, |w| active(w).is_some_and(|s| s["folder"].as_str().is_some_and(|f| names(f, &b))));
    let world = world.expect("the window answers ls");
    assert!(active(&world).is_some_and(|s| s["machine"].is_null() && names(s["folder"].as_str().unwrap(), &b)), "{world:#}");
    assert_eq!(local_spaces_in(&world, &b).len(), 1, "{world:#}");
    let count = spaces(&world).len();

    // AC3: A again (on Windows in another case, with a trailing separator): A's space, no new one.
    // (ASCII letters only: non-ASCII case is the unit test's
    // `windows_folder_case_is_ignored_beyond_ascii_too`.)
    let again = if cfg!(windows) { PathBuf::from(format!("{}\\", a.to_str().unwrap().to_ascii_uppercase())) } else { PathBuf::from(format!("{}/", a.to_str().unwrap())) };
    let mut third = Launch::start(in_shell(&env, &again));
    let status = third.wait(90).expect("the third launch did not exit");
    assert!(status.success(), "{status:?}: {}", third.output());
    let lines = terminal_lines(&third.output());
    assert!(lines.len() == 1 && lines[0].starts_with("HyperOx is already running. Opened "), "{}", third.output());
    let world = wait_for(&env, 30, |w| active(w).is_some_and(|s| s["folder"].as_str().is_some_and(|f| names(f, &a)))).expect("the window answers ls");
    assert!(active(&world).is_some_and(|s| names(s["folder"].as_str().unwrap(), &a)), "{world:#}");
    assert_eq!(local_spaces_in(&world, &a).len(), 1, "{world:#}");
    assert_eq!(spaces(&world).len(), count, "{world:#}");

    // AC4: not from a shell (a shortcut, the Dock, Explorer) in a new folder: raised only.
    let (status, out) = not_from_shell(&env, &c);
    let status = status.expect("the launch that is not from a shell did not exit");
    assert!(status.success(), "{status:?}");
    assert!(out.is_empty(), "nothing is printed: {out:?}");
    // Give a wrongly made space time to show up before looking.
    std::thread::sleep(Duration::from_secs(3));
    let world = env.ls().expect("the window answers ls");
    assert!(local_spaces_in(&world, &c).is_empty(), "{world:#}");
    assert_eq!(spaces(&world).len(), count, "{world:#}");
    assert!(active(&world).is_some_and(|s| names(s["folder"].as_str().unwrap(), &a)), "{world:#}");

    // For a person to look at, not an assertion: the To-do panel opened by its key's command
    // (Ctrl/Cmd+Shift+J), saved where CI uploads it with the Info page below.
    let todo_shot = std::env::var_os("RUNNER_TEMP").map(PathBuf::from).unwrap_or_else(|| root.clone()).join("todo-panel.png");
    // A fresh home shows the welcome over everything: close it so the panel is visible.
    let _ = env.command(EXE).args(["do", "overlay.close"]).stdin(Stdio::null()).output();
    let opened = env.command(EXE).args(["do", "tabs.todo"]).stdin(Stdio::null()).output();
    std::thread::sleep(Duration::from_secs(3));
    let saved = env.command(EXE).arg("shot").arg(&todo_shot).stdin(Stdio::null()).output();
    eprintln!("to-do panel: open {:?}, screenshot {} {:?}", opened.map(|o| (o.status, String::from_utf8_lossy(&o.stdout).into_owned())), todo_shot.display(), saved.map(|o| (o.status, String::from_utf8_lossy(&o.stderr).into_owned())));

    // For a person to look at, not an assertion: Settings on the Info page, saved where CI
    // uploads it (artifact `info-page-<platform>`).
    let shot = std::env::var_os("RUNNER_TEMP").map(PathBuf::from).unwrap_or_else(|| root.clone()).join("info-page.png");
    let opened = env.command(EXE).args(["do", "app.settings", "--section", "info"]).stdin(Stdio::null()).output();
    std::thread::sleep(Duration::from_secs(3));
    let saved = env.command(EXE).arg("shot").arg(&shot).stdin(Stdio::null()).output();
    eprintln!("info page: settings {:?}, screenshot {} {:?}", opened.map(|o| o.status), shot.display(), saved.map(|o| (o.status, String::from_utf8_lossy(&o.stderr).into_owned())));

    // For a person to look at, not an assertion: Settings > Look with the shipped themes, then the
    // window in a light and a dark new theme (artifact `info-page-<platform>`). A light theme is
    // the layout's light half, so the appearance is set to match through settings.json.
    let shots = std::env::var_os("RUNNER_TEMP").map(PathBuf::from).unwrap_or_else(|| root.clone());
    let _ = env.command(EXE).args(["do", "overlay.close"]).stdin(Stdio::null()).output();
    let opened = env.command(EXE).args(["do", "app.settings", "--section", "look"]).stdin(Stdio::null()).output();
    std::thread::sleep(Duration::from_secs(3));
    let saved = env.command(EXE).arg("shot").arg(shots.join("look-themes.png")).stdin(Stdio::null()).output();
    eprintln!("look page: settings {:?}, screenshot {:?}", opened.map(|o| o.status), saved.map(|o| (o.status, String::from_utf8_lossy(&o.stderr).into_owned())));
    let _ = env.command(EXE).args(["do", "overlay.close"]).stdin(Stdio::null()).output();
    for (theme, appearance) in [("neo-brutalism", "light"), ("amber-minimal", "dark")] {
        let settings = env.home.join("settings.json");
        let mut person: Value = std::fs::read_to_string(&settings).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
        if let Some(object) = person.as_object_mut() {
            object.insert("appearance".into(), appearance.into());
            let _ = std::fs::write(&settings, serde_json::to_string_pretty(&person).unwrap());
        }
        let applied = env.command(EXE).args(["do", "app.theme", "--name", theme]).stdin(Stdio::null()).output();
        std::thread::sleep(Duration::from_secs(3));
        let saved = env.command(EXE).arg("shot").arg(shots.join(format!("theme-{theme}.png"))).stdin(Stdio::null()).output();
        eprintln!("theme {theme}: {:?}, screenshot {:?}", applied.map(|o| (o.status, String::from_utf8_lossy(&o.stdout).into_owned())), saved.map(|o| (o.status, String::from_utf8_lossy(&o.stderr).into_owned())));
    }

    // Quit; nothing of HyperOx is left running.
    let _ = env.command(EXE).args(["do", "app.quit"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    #[cfg(windows)]
    {
        let mut window = cleanup.launch.take().unwrap();
        let status = window.wait(60);
        assert!(status.is_some(), "HyperOx did not quit; output: {}", window.output());
    }
    #[cfg(unix)]
    {
        let until = Instant::now() + Duration::from_secs(60);
        while !window_pids(&env).is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(window_pids(&env).is_empty(), "HyperOx did not quit: {:?} still running", window_pids(&env));
    }
    assert!(env.ls().is_none(), "a window still answers after quit");
    let stopped = env.command(env.core()).args(["server", "stop"]).stdin(Stdio::null()).output().unwrap();
    eprintln!("h0x-core server stop: {} {}{}", stopped.status, String::from_utf8_lossy(&stopped.stdout), String::from_utf8_lossy(&stopped.stderr));
}
