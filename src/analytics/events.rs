//! The usage events the app raises (docs/security.md, "What HyperOx sends"): names, counts and
//! sizes only, never what a terminal shows, nor a path, folder name or title. Each kind is
//! throttled here so a busy session stays at a few hundred events an hour.

use crate::analytics::{device_details, event};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// At most this many throttled events (shortcuts, panels, themes) an hour.
const HOURLY_BUDGET: u32 = 300;
/// The agent command lines looked for on PATH: presence only.
const TOOLS: [&str; 10] = [
    "claude",
    "codex",
    "gemini",
    "opencode",
    "amp",
    "cursor-agent",
    "aider",
    "goose",
    "copilot",
    "git",
];
/// Shell programs named in the daily event; any other program is not reported.
const SHELLS: [&str; 10] = [
    "pwsh",
    "powershell",
    "cmd",
    "bash",
    "zsh",
    "fish",
    "sh",
    "wsl",
    "nu",
    "dash",
];

struct State {
    last: BTreeMap<String, Instant>,
    since: Option<Instant>,
    count: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    last: BTreeMap::new(),
    since: None,
    count: 0,
});
static PROCESS: Mutex<Option<Instant>> = Mutex::new(None);
static DAY: AtomicU64 = AtomicU64::new(0);

/// True once per `every` for `key`, while the hourly budget lasts.
fn allow(key: &str, every: Duration) -> bool {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if s.since
        .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(3600))
    {
        s.since = Some(now);
        s.count = 0;
    }
    if s.last
        .get(key)
        .is_some_and(|t| now.duration_since(*t) < every)
        || s.count >= HOURLY_BUDGET
    {
        return false;
    }
    if s.last.len() > 500 {
        s.last.clear();
    }
    s.last.insert(key.to_owned(), now);
    s.count += 1;
    true
}

/// A short fixed name: letters, digits and `._-` only, else nothing is sent.
fn word(s: &str) -> Option<&str> {
    (!s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
    .then_some(s)
}

/// The process began (first line of `main`), for `app_start_ms`.
pub fn process_started() {
    *PROCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// The first window was opened.
pub fn window_shown() {
    if let Some(t) = *PROCESS.lock().unwrap_or_else(|e| e.into_inner()) {
        event(
            "app_start_ms",
            json!({ "ms": t.elapsed().as_millis() as u64 }),
        );
    }
}

/// A keys.json action ran (`name`, `name:value` or `<mod>.<command>`): the name only.
pub fn shortcut_used(action: &str) {
    let name = action.split(':').next().unwrap_or_default();
    if let Some(name) = word(name)
        && allow(&format!("shortcut:{name}"), Duration::from_secs(60))
    {
        event("shortcut_used", json!({ "action": name }));
    }
}

/// A panel (a shipped mod such as `todo`, `files`, `changes`, or `settings`) was opened.
pub fn panel_opened(panel: &str) {
    if word(panel).is_some() && allow(&format!("panel:{panel}"), Duration::from_secs(60)) {
        event("panel_opened", json!({ "panel": panel }));
    }
}

pub fn theme_changed(theme: &str) {
    if word(theme).is_some() && allow("theme", Duration::from_secs(10)) {
        event("theme_changed", json!({ "theme": theme }));
    }
}

/// The welcome's one-time "How did you find HyperOx?" was answered.
pub fn found_via(answer: &str) {
    if let Some(answer) = word(answer) {
        event("found_via", json!({ "answer": answer }));
    }
}

/// An update check for `part` (`app`, `core` or `runtime`) finished.
pub fn update_checked(part: &str) {
    if word(part).is_some() && allow(&format!("update_checked:{part}"), Duration::from_secs(600)) {
        event("update_checked", json!({ "part": part }));
    }
}

/// A newer `version` of `part` was downloaded and is waiting.
pub fn update_downloaded(part: &str, version: &str) {
    if let (Some(part), Some(version)) = (word(part), word(version))
        && allow(&format!("update_downloaded:{part}"), Duration::from_secs(60))
    {
        event(
            "update_downloaded",
            json!({ "part": part, "version": version }),
        );
    }
}

/// `part` went from `from` to `to`, `seconds` after the update started.
pub fn update_applied(part: &str, from: &str, to: &str, seconds: u64) {
    if let (Some(part), Some(from), Some(to)) = (word(part), word(from), word(to))
        && allow(&format!("update_applied:{part}"), Duration::from_secs(60))
    {
        event(
            "update_applied",
            json!({ "part": part, "from": from, "to": to, "seconds": seconds }),
        );
    }
}

/// An update of `part` to `to` failed and `from` was put back.
pub fn update_rolled_back(part: &str, from: &str, to: &str) {
    if let (Some(part), Some(from), Some(to)) = (word(part), word(from), word(to))
        && allow(&format!("update_rolled_back:{part}"), Duration::from_secs(60))
    {
        event(
            "update_rolled_back",
            json!({ "part": part, "from": from, "to": to }),
        );
    }
}

/// Once an hour, from the window's tick: memory in use (Linux only; the others have no cheap
/// standard-library way).
pub fn hourly() {
    if allow("hourly", Duration::from_secs(3600))
        && let Some(mb) = memory_mb()
    {
        event("memory_mb", json!({ "mb": mb }));
    }
}

#[cfg(target_os = "linux")]
fn memory_mb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096 / 1_048_576)
}

#[cfg(not(target_os = "linux"))]
fn memory_mb() -> Option<u64> {
    None
}

fn today() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
}

/// True once per calendar day (UTC) across runs, so the daily event goes out once; the day is
/// kept in `analytics/daily_device` in the app home.
pub fn daily_due() -> bool {
    let day = today();
    if day == 0 || DAY.swap(day, Ordering::Relaxed) == day {
        return false;
    }
    let path = crate::mods::app_dir()
        .join("analytics")
        .join("daily_device");
    if std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        == Some(day)
    {
        return false;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, day.to_string());
    true
}

/// How the workspace looks, counted by the window.
pub struct Shape {
    pub spaces: usize,
    pub tabs: usize,
    pub panes: usize,
    pub terminals: usize,
    pub remote_machines: usize,
    pub layout: String,
    pub window_size: &'static str,
    /// First words of the programs the shell panes started with, already lowercase.
    pub programs: Vec<String>,
}

/// S, M, L or XL for a window's width in logical pixels.
pub fn size_bucket(width: f32) -> &'static str {
    match width {
        w if w < 1000.0 => "S",
        w if w < 1400.0 => "M",
        w if w < 1900.0 => "L",
        _ => "XL",
    }
}

/// A shell's name from the program a terminal started with (`C:\\Windows\\pwsh.exe` is `pwsh`).
pub fn shell_name(program: &str) -> Option<&'static str> {
    let base = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let base = base.strip_suffix(".exe").unwrap_or(&base);
    SHELLS.iter().copied().find(|s| *s == base)
}

/// The `daily_device` event: what the machine is, which agent tools are installed, how big the
/// workspace is. The PATH scan and the to-do count run off the UI thread.
pub fn daily_device(shape: Shape) {
    let spawned = std::thread::Builder::new()
        .name("analytics-daily".into())
        .spawn(move || {
            let mut shells: Vec<&str> = shape
                .programs
                .iter()
                .filter_map(|p| shell_name(p))
                .collect();
            shells.sort_unstable();
            shells.dedup();
            let todo_lists = std::fs::read_dir(crate::mods::app_dir().join("todo"))
                .map(|d| {
                    d.flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                        .count()
                })
                .unwrap_or(0);
            event(
                "daily_device",
                json!({
                    "device": device_details(),
                    "tools": tools_on_path(),
                    "shells": shells,
                    "spaces": shape.spaces,
                    "tabs": shape.tabs,
                    "panes": shape.panes,
                    "terminals": shape.terminals,
                    "remote_machines": shape.remote_machines,
                    "todo_lists": todo_lists,
                    "layout": shape.layout,
                    "window_size": shape.window_size,
                }),
            );
        });
    if let Err(e) = spawned {
        tracing::warn!("daily device event not sent: {e}");
    }
}

/// Which of `TOOLS` have a file of that name in a PATH folder (versions are not read).
fn tools_on_path() -> Vec<&'static str> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    TOOLS
        .iter()
        .copied()
        .filter(|t| dirs.iter().any(|d| has_tool(d, t)))
        .collect()
}

fn has_tool(dir: &Path, tool: &str) -> bool {
    ["", ".exe", ".cmd", ".bat", ".ps1"]
        .iter()
        .any(|ext| dir.join(format!("{tool}{ext}")).is_file())
}

/// An agent seen running now: terminal address, its 7501 `app`, its folder.
pub struct Seen {
    pub address: String,
    pub app: String,
    pub folder: Option<String>,
}

struct Running {
    kind: &'static str,
    since: Instant,
    started: SystemTime,
    folder: Option<String>,
}

static AGENTS: Mutex<BTreeMap<String, Running>> = Mutex::new(BTreeMap::new());

/// claude, codex, gemini, opencode or other, from the agent's `app` name.
fn agent_kind(app: &str) -> &'static str {
    let app = app.to_ascii_lowercase();
    ["claude", "codex", "gemini", "opencode"]
        .into_iter()
        .find(|k| app.contains(k))
        .unwrap_or("other")
}

/// Compares the agents running now with the ones seen last time: `agent_started` for new ones
/// (not on the first listing, `first`: they were already there), `agent_ended` for gone ones, and
/// for claude and codex their token counts, read off the UI thread.
pub fn agents(now: &[Seen], first: bool) {
    let mut map = AGENTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut ended = Vec::new();
    map.retain(|addr, run| {
        let alive = now
            .iter()
            .any(|s| &s.address == addr && agent_kind(&s.app) == run.kind);
        if !alive {
            ended.push((
                addr.clone(),
                run.kind,
                run.since,
                run.started,
                run.folder.clone(),
            ));
        }
        alive
    });
    let before = map.len();
    for s in now {
        if !map.contains_key(&s.address) {
            let kind = agent_kind(&s.app);
            map.insert(
                s.address.clone(),
                Running {
                    kind,
                    since: Instant::now(),
                    started: SystemTime::now(),
                    folder: s.folder.clone(),
                },
            );
            if !first && allow(&format!("agent_started:{kind}"), Duration::from_secs(5)) {
                event(
                    "agent_started",
                    json!({ "agent": kind, "concurrent": map.len(), "remote": s.address.contains('/') }),
                );
            }
        }
    }
    let concurrent = before + ended.len();
    drop(map);
    for (addr, kind, since, started, folder) in ended {
        let seconds = since.elapsed().as_secs();
        if seconds < 5 {
            continue;
        }
        let remote = addr.contains('/');
        event(
            "agent_ended",
            json!({ "agent": kind, "seconds": seconds, "concurrent": concurrent, "remote": remote }),
        );
        let usage_kind = match kind {
            "claude" => crate::agent_usage::AgentKind::Claude,
            "codex" => crate::agent_usage::AgentKind::Codex,
            _ => continue,
        };
        let Some(folder) = folder.filter(|_| !remote) else {
            continue;
        };
        let _ = std::thread::Builder::new().name("analytics-tokens".into()).spawn(move || {
            let Some(u) = crate::agent_usage::usage(usage_kind, Path::new(&folder), started, SystemTime::now()) else { return };
            let model: Value = u.model.map_or(Value::Null, Value::from);
            event(
                "agent_tokens",
                json!({
                    "agent": kind, "model": model, "input": u.input, "output": u.output, "cache_read": u.cache_read,
                    "cache_write": u.cache_write, "reasoning": u.reasoning, "cost_usd": u.cost_usd,
                }),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_names_keep_only_fixed_looking_text() {
        assert_eq!(word("catppuccin-mocha"), Some("catppuccin-mocha"));
        assert_eq!(word(""), None);
        assert_eq!(word("C:\\Users\\me"), None);
        assert_eq!(word("two words"), None);
        assert_eq!(word("0.1.25-rc.1"), Some("0.1.25-rc.1"));
        assert_eq!(word("h0x-core 0.1.2"), None);
        assert_eq!(shell_name("C:\\Windows\\System32\\PWSH.EXE"), Some("pwsh"));
        assert_eq!(shell_name("/usr/bin/zsh"), Some("zsh"));
        assert_eq!(shell_name("htop"), None);
        assert_eq!(agent_kind("claude-code"), "claude");
        assert_eq!(agent_kind("Kiro"), "other");
        assert_eq!(
            (
                size_bucket(999.0),
                size_bucket(1000.0),
                size_bucket(1899.0),
                size_bucket(4000.0)
            ),
            ("S", "M", "L", "XL")
        );
    }
}
