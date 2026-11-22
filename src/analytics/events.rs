//! The usage events the app raises (docs/security.md, "What HyperOx sends"): names, counts and
//! sizes only, never what a terminal shows, nor a path, folder name or title. Each kind is
//! throttled here so a busy session stays at a few hundred events an hour.

use crate::analytics::{device_details, event};
use crate::agent_usage::{AgentKind, Turn, TurnCursor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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

/// `gate.stats` as the Token Saver's lifetime counters; None for an old core (error) or anything but two whole numbers.
pub fn parse_gate_stats(reply: &anyhow::Result<Value>) -> Option<crate::activity::Saved> {
    let v = reply.as_ref().ok()?;
    Some(crate::activity::Saved { tokens: v.get("tokens_saved")?.as_u64()?, outputs: v.get("outputs")?.as_u64()? })
}

/// The `token_saver_daily` event: what changed since yesterday, and the totals as person properties.
pub fn token_saver_daily_props(delta: crate::activity::Saved, total: crate::activity::Saved, on: bool) -> Value {
    json!({
        "tokens_saved_delta": delta.tokens,
        "outputs_delta": delta.outputs,
        "$set": { "tokens_saved_total": total.tokens, "token_saver_on": on },
    })
}

/// Once a day: books the Token Saver's change (a local record, whatever the sharing setting) and sends it.
/// An old core or an unreadable answer records and sends nothing.
pub fn token_saver_daily(reply: &anyhow::Result<Value>, on: bool) {
    let Some(total) = parse_gate_stats(reply) else { return };
    let delta = crate::activity::book_saved(total);
    event("token_saver_daily", token_saver_daily_props(delta, total, on));
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
    /// One id for this run (`$ai_trace_id`).
    trace: String,
    last_read: Instant,
    /// Turns sent so far, for `$ai_trace`.
    turns: Arc<AtomicU64>,
    remote: bool,
}

/// A running agent's new turns are read this often.
const READ_EVERY: Duration = Duration::from_secs(600);

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
/// (not on the first listing, `first`: they were already there), `agent_ended` for gone ones. For
/// local claude and codex runs their turns are read off the UI thread every ten minutes and once
/// more when the run ends (`$ai_generation` per turn, `$ai_trace` at the end). `core` redacts the
/// typed prompts.
pub fn agents(now: &[Seen], first: bool, core: &Arc<crate::core_link::Core>) {
    let mut map = AGENTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut ended = Vec::new();
    map.retain(|addr, run| {
        let alive = now
            .iter()
            .any(|s| &s.address == addr && agent_kind(&s.app) == run.kind);
        if !alive {
            ended.push((addr.clone(), run.kind, run.since, run.job()));
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
                    trace: new_trace_id(),
                    last_read: Instant::now(),
                    turns: Arc::new(AtomicU64::new(0)),
                    remote: s.address.contains('/'),
                },
            );
            if !first {
                crate::activity::session_started();
            }
            if !first && allow(&format!("agent_started:{kind}"), Duration::from_secs(5)) {
                event(
                    "agent_started",
                    json!({ "agent": kind, "concurrent": map.len(), "remote": s.address.contains('/') }),
                );
            }
        }
    }
    let mut due = Vec::new();
    for run in map.values_mut() {
        if run.last_read.elapsed() >= READ_EVERY
            && let Some(job) = run.job()
        {
            run.last_read = Instant::now();
            due.push(job);
        }
    }
    let concurrent = before + ended.len();
    drop(map);
    let token_saver = (!due.is_empty() || ended.iter().any(|e| e.3.is_some()))
        .then(|| crate::settings::get().token_saver)
        .unwrap_or_default();
    for job in due {
        read_in_background(core, job, false, token_saver);
    }
    for (addr, kind, since, job) in ended {
        let seconds = since.elapsed().as_secs();
        if seconds < 5 {
            continue;
        }
        event(
            "agent_ended",
            json!({ "agent": kind, "seconds": seconds, "concurrent": concurrent, "remote": addr.contains('/') }),
        );
        if let Some(mut job) = job {
            job.seconds = seconds;
            read_in_background(core, job, true, token_saver);
        }
    }
}

/// A local claude or codex run whose turns are read.
#[derive(Clone)]
struct Job {
    kind: &'static str,
    usage_kind: AgentKind,
    folder: PathBuf,
    started: SystemTime,
    trace: String,
    turns: Arc<AtomicU64>,
    /// Run length, set when the run ended.
    seconds: u64,
}

impl Running {
    fn job(&self) -> Option<Job> {
        let usage_kind = match self.kind {
            "claude" => AgentKind::Claude,
            "codex" => AgentKind::Codex,
            _ => return None,
        };
        let folder = self.folder.clone().filter(|_| !self.remote)?;
        Some(Job {
            kind: self.kind,
            usage_kind,
            folder: PathBuf::from(folder),
            started: self.started,
            trace: self.trace.clone(),
            turns: self.turns.clone(),
            seconds: 0,
        })
    }
}

/// One id per HyperOx agent run; PostHog groups the run's generations as one trace.
fn new_trace_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("h0x-{nanos:x}-{:x}-{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// `text.redact` answered `{text}`; anything else (an old core, `too_large`, any error) is no text.
fn parse_redacted(reply: &anyhow::Result<Value>) -> Option<String> {
    reply.as_ref().ok()?.get("text")?.as_str().map(str::to_owned)
}

/// Reads are one at a time, so a slow read never overlaps the final one of its run.
static READING: Mutex<()> = Mutex::new(());

fn read_in_background(core: &Arc<crate::core_link::Core>, job: Job, finish: bool, token_saver: bool) {
    let core = core.clone();
    let _ = std::thread::Builder::new().name("analytics-turns".into()).spawn(move || {
        let _one = READING.lock().unwrap_or_else(|e| e.into_inner());
        let redact = |text: &str| parse_redacted(&core.call_timeout("text.redact", json!({ "text": text }), Duration::from_secs(3)));
        let ctx = Ctx {
            dir: &crate::mods::app_dir().join("usage"),
            sharing: crate::analytics::sharing(),
            token_saver,
            redact: &redact,
            emit: &|name, props| event(name, props),
            usage: &crate::activity::add_usage,
        };
        read_run(&ctx, &job, finish);
        if finish {
            ctx.trace_ended(&job);
        }
    });
}

/// What one read needs from outside, so tests can swap it.
struct Ctx<'a> {
    dir: &'a Path,
    sharing: bool,
    token_saver: bool,
    redact: &'a dyn Fn(&str) -> Option<String>,
    emit: &'a dyn Fn(&str, Value),
    usage: &'a dyn Fn(&str, u64, f64),
}

/// The turns since the stored cursor: each is booked locally and, with sharing on, queued as
/// `$ai_generation`. The cursor is stored only after the events are queued, and a turn read once is
/// never read again. Returns how many turns were read.
fn read_run(ctx: &Ctx, job: &Job, finish: bool) -> usize {
    let turns = read_with(ctx, job, finish, |cursor| {
        crate::agent_usage::turns(job.usage_kind, &job.folder, job.started, SystemTime::now(), cursor, finish)
    });
    job.turns.fetch_add(turns as u64, Ordering::Relaxed);
    turns
}

fn read_with(ctx: &Ctx, job: &Job, finish: bool, read: impl FnOnce(&mut TurnCursor) -> Vec<Turn>) -> usize {
    let mut store = Cursors::load(ctx.dir);
    let mut cursor = store.runs.remove(&job.trace).map(|r| r.cursor).unwrap_or_default();
    let turns = read(&mut cursor);
    for t in &turns {
        (ctx.usage)(job.kind, t.input.saturating_add(t.output), t.cost_usd.unwrap_or(0.0));
        if ctx.sharing {
            let input = t.prompt.as_deref().filter(|p| !p.trim().is_empty()).and_then(|p| (ctx.redact)(p));
            (ctx.emit)("$ai_generation", generation_props(&job.trace, job.kind, t, input.as_deref(), ctx.token_saver));
        }
    }
    if !finish {
        store.runs.insert(job.trace.clone(), RunCursor { at: unix_secs(), cursor });
    }
    store.save(ctx.dir);
    turns.len()
}

impl Ctx<'_> {
    /// The run is over: `$ai_trace` with the run's length and how many turns were sent.
    fn trace_ended(&self, job: &Job) {
        if self.sharing {
            (self.emit)("$ai_trace", trace_props(&job.trace, job.kind, job.seconds, job.turns.load(Ordering::Relaxed)));
        }
    }
}

/// PostHog's `$ai_generation` for one turn. `$ai_input` is the redacted typed prompt as a one-message
/// list, left out when there is none. Agent replies and tool output are never part of it.
fn generation_props(trace: &str, agent: &str, t: &Turn, input: Option<&str>, token_saver: bool) -> Value {
    let mut p = json!({
        "$ai_trace_id": trace,
        "$ai_provider": t.provider,
        "$ai_input_tokens": t.input,
        "$ai_output_tokens": t.output,
        "$ai_cache_read_input_tokens": t.cache_read,
        "$ai_cache_creation_input_tokens": t.cache_write,
        "agent": agent,
        "tools": t.tools,
        "mcp_servers": t.mcp_servers,
        "skills": t.skills,
        "token_saver": token_saver,
    });
    if let Some(model) = &t.model {
        p["$ai_model"] = json!(model);
    }
    if let Some(cost) = t.cost_usd.filter(|c| c.is_finite()) {
        p["$ai_total_cost_usd"] = json!(cost);
    }
    if let Some(ms) = t.latency_ms {
        p["$ai_latency"] = json!(ms as f64 / 1000.0);
    }
    if let Some(text) = input {
        p["$ai_input"] = json!([{ "role": "user", "content": text }]);
    }
    p
}

/// PostHog's `$ai_trace` for a finished run.
fn trace_props(trace: &str, agent: &str, seconds: u64, turns: u64) -> Value {
    json!({ "$ai_trace_id": trace, "$ai_span_name": agent, "$ai_latency": seconds, "turns": turns, "remote": false })
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where each running agent's reads stopped (`usage/cursors.json`, keyed by trace id), so a quit
/// and restart, or the next read, never sends a turn twice.
#[derive(Default, Serialize, Deserialize)]
struct Cursors {
    runs: HashMap<String, RunCursor>,
}

#[derive(Serialize, Deserialize)]
struct RunCursor {
    /// Unix seconds of the last read; old entries of runs that never ended are dropped.
    at: u64,
    cursor: TurnCursor,
}

impl Cursors {
    fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("cursors.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&mut self, dir: &Path) {
        let keep = unix_secs().saturating_sub(7 * 24 * 3600);
        self.runs.retain(|_, r| r.at >= keep);
        let (file, tmp) = (dir.join("cursors.json"), dir.join("cursors.json.tmp"));
        let _ = std::fs::create_dir_all(dir);
        if let Ok(text) = serde_json::to_string(self)
            && std::fs::write(&tmp, text).is_ok()
        {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Saved;

    #[test]
    fn gate_stats_reads_two_whole_numbers_and_nothing_else() {
        let ok = |v: Value| parse_gate_stats(&Ok(v));
        assert_eq!(ok(json!({"tokens_saved": 12, "outputs": 3, "bytes_in": 9})), Some(Saved { tokens: 12, outputs: 3 }));
        assert_eq!(ok(json!({"tokens_saved": 0, "outputs": 0})), Some(Saved { tokens: 0, outputs: 0 }));
        for bad in [json!({}), json!(null), json!("x"), json!([]), json!({"tokens_saved": 1}), json!({"outputs": 1}), json!({"tokens_saved": -5, "outputs": 1}), json!({"tokens_saved": "7", "outputs": 1}), json!({"tokens_saved": 1.5, "outputs": 1}), json!({"tokens_saved": 1, "outputs": null})] {
            assert_eq!(ok(bad.clone()), None, "{bad}");
        }
        // An old core has no `gate.stats`: the call fails.
        assert_eq!(parse_gate_stats(&Err(anyhow::anyhow!("unknown method"))), None);
    }

    #[test]
    fn the_daily_saver_event_has_the_change_and_sets_the_totals() {
        let props = token_saver_daily_props(Saved { tokens: 500, outputs: 2 }, Saved { tokens: 1500, outputs: 12 }, true);
        assert_eq!(props, json!({"tokens_saved_delta": 500, "outputs_delta": 2, "$set": {"tokens_saved_total": 1500, "token_saver_on": true}}));
        let reset = token_saver_daily_props(Saved::default(), Saved { tokens: 40, outputs: 1 }, false);
        assert_eq!((&reset["tokens_saved_delta"], &reset["$set"]["token_saver_on"]), (&json!(0), &json!(false)));
    }

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

    #[test]
    fn update_events_are_throttled_per_part_and_odd_text_uses_no_slot() {
        update_checked("app");
        assert!(
            !allow("update_checked:app", Duration::from_secs(600)),
            "a second app check within ten minutes is held back"
        );
        update_rolled_back("core", "0.1.1", "0.1.2");
        assert!(!allow("update_rolled_back:core", Duration::from_secs(60)));
        // The core's fallback target text is not a plain word: nothing is sent, no slot used.
        update_applied("runtime", "0.9.5", "the latest", 3);
        assert!(allow("update_applied:runtime", Duration::from_secs(60)));
        update_downloaded("C:\\Users\\me", "0.1.2");
        assert!(allow(
            "update_downloaded:C:\\Users\\me",
            Duration::from_secs(60)
        ));
        update_downloaded("", "");
        assert!(allow("update_downloaded:", Duration::from_secs(60)));
    }

    fn turn(prompt: Option<&str>) -> Turn {
        Turn {
            timestamp_ms: 1,
            model: Some("claude-sonnet".into()),
            provider: "anthropic",
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write: 40,
            cost_usd: Some(0.5),
            latency_ms: Some(2500),
            tools: vec!["Bash".into()],
            mcp_servers: vec!["github".into()],
            skills: vec!["review".into()],
            prompt: prompt.map(Into::into),
        }
    }

    fn job(trace: &str) -> Job {
        Job {
            kind: "claude",
            usage_kind: AgentKind::Claude,
            folder: PathBuf::from("/x/y"),
            started: SystemTime::now(),
            trace: trace.into(),
            turns: Arc::new(AtomicU64::new(0)),
            seconds: 0,
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("h0x-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    type Sent = std::cell::RefCell<Vec<(String, Value)>>;
    type Booked = std::cell::RefCell<Vec<(String, u64, f64)>>;

    /// One `read_with` with fakes: `redact` is the fake core, `sent` and `booked` record the outputs.
    #[allow(clippy::too_many_arguments)]
    fn run(d: &Path, sharing: bool, redact: &dyn Fn(&str) -> Option<String>, sent: &Sent, booked: &Booked, trace: &str, finish: bool, turns: Vec<Turn>) -> usize {
        let emit = |n: &str, p: Value| sent.borrow_mut().push((n.into(), p));
        let usage = |a: &str, t: u64, c: f64| booked.borrow_mut().push((a.into(), t, c));
        let ctx = Ctx { dir: d, sharing, token_saver: true, redact, emit: &emit, usage: &usage };
        read_with(&ctx, &job(trace), finish, |_| turns)
    }

    #[test]
    fn a_generation_has_the_posthog_names_and_the_redacted_prompt_as_a_message() {
        let (d, sent, booked) = (dir("shape"), Sent::default(), Booked::default());
        let fake_core = |t: &str| Some(t.replace("sk-secret", "[redacted]"));
        run(&d, true, &fake_core, &sent, &booked, "t1", false, vec![turn(Some("use sk-secret now"))]);
        let sent = sent.into_inner();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "$ai_generation");
        assert_eq!(
            sent[0].1,
            json!({
                "$ai_trace_id": "t1", "$ai_provider": "anthropic", "$ai_model": "claude-sonnet",
                "$ai_input_tokens": 10, "$ai_output_tokens": 20, "$ai_cache_read_input_tokens": 30,
                "$ai_cache_creation_input_tokens": 40, "$ai_total_cost_usd": 0.5, "$ai_latency": 2.5,
                "$ai_input": [{ "role": "user", "content": "use [redacted] now" }],
                "agent": "claude", "tools": ["Bash"], "mcp_servers": ["github"], "skills": ["review"], "token_saver": true,
            })
        );
        assert!(!sent[0].1.to_string().contains("sk-secret"));
    }

    #[test]
    fn without_a_redacting_core_the_numbers_go_and_no_text() {
        let (d, sent, booked) = (dir("noredact"), Sent::default(), Booked::default());
        run(&d, true, &|_| None, &sent, &booked, "t1", false, vec![turn(Some("a secret prompt")), turn(None), turn(Some("  \n"))]);
        let sent = sent.into_inner();
        assert_eq!(sent.len(), 3);
        for (_, p) in &sent {
            assert!(p.get("$ai_input").is_none());
            assert_eq!(p["$ai_input_tokens"], 10);
            assert!(!p.to_string().contains("secret"));
        }
        // An unknown model, cost or latency leaves those names out.
        let mut bare = turn(None);
        (bare.model, bare.cost_usd, bare.latency_ms) = (None, Some(f64::NAN), None);
        let p = generation_props("t", "codex", &bare, None, false);
        for k in ["$ai_model", "$ai_total_cost_usd", "$ai_latency", "$ai_input"] {
            assert!(p.get(k).is_none(), "{k}");
        }
        assert_eq!(p["token_saver"], false);
    }

    #[test]
    fn redact_replies_are_text_or_nothing() {
        assert_eq!(parse_redacted(&Ok(json!({ "text": "ok" }))), Some("ok".into()));
        assert_eq!(parse_redacted(&Ok(json!({ "text": "" }))), Some(String::new()));
        for bad in [json!({}), json!(null), json!("x"), json!({ "text": 5 })] {
            assert_eq!(parse_redacted(&Ok(bad.clone())), None, "{bad}");
        }
        assert_eq!(parse_redacted(&Err(anyhow::anyhow!("too_large"))), None);
    }

    #[test]
    fn sharing_off_books_locally_and_queues_nothing() {
        let (d, sent, booked) = (dir("off"), Sent::default(), Booked::default());
        let n = run(&d, false, &|t| Some(t.into()), &sent, &booked, "t1", false, vec![turn(Some("hi"))]);
        assert_eq!(n, 1);
        assert!(sent.borrow().is_empty());
        assert_eq!(booked.into_inner(), [("claude".to_string(), 30, 0.5)]);
    }

    #[test]
    fn usage_is_booked_once_per_turn_with_a_missing_cost_as_zero() {
        let (d, sent, booked) = (dir("book"), Sent::default(), Booked::default());
        let mut free = turn(None);
        free.cost_usd = None;
        run(&d, true, &|_| None, &sent, &booked, "t1", false, vec![turn(None), free]);
        assert_eq!(booked.into_inner(), [("claude".to_string(), 30, 0.5), ("claude".to_string(), 30, 0.0)]);
    }

    fn cursor_at(n: u64) -> TurnCursor {
        serde_json::from_value(json!({ "offsets": { "f.jsonl": n } })).unwrap()
    }

    #[test]
    fn the_cursor_is_stored_only_after_the_events_are_queued_and_dropped_at_the_end() {
        let d = dir("cursor");
        let seen_before = std::cell::Cell::new(None);
        let emit = |_: &str, _: Value| seen_before.set(Some(Cursors::load(&d).runs.contains_key("t1")));
        let usage = |_: &str, _: u64, _: f64| ();
        let ctx = Ctx { dir: &d, sharing: true, token_saver: false, redact: &|_| None, emit: &emit, usage: &usage };
        read_with(&ctx, &job("t1"), false, |c| {
            *c = cursor_at(99);
            vec![turn(None)]
        });
        assert_eq!(seen_before.get(), Some(false), "no cursor yet while the event is being queued");
        assert_eq!(Cursors::load(&d).runs["t1"].cursor, cursor_at(99));
        // The next read starts from the stored cursor; the final read drops it.
        let got = std::cell::RefCell::new(None);
        read_with(&ctx, &job("t1"), true, |c| {
            *got.borrow_mut() = Some(c.clone());
            Vec::new()
        });
        assert_eq!(got.into_inner(), Some(cursor_at(99)));
        assert!(Cursors::load(&d).runs.is_empty());
        // A broken file starts over instead of failing.
        std::fs::write(d.join("cursors.json"), "{ nope").unwrap();
        assert!(Cursors::load(&d).runs.is_empty());
    }

    #[test]
    fn a_turn_is_never_sent_twice_across_ten_minute_reads_and_the_final_read() {
        let (d, sent, booked) = (dir("dedup"), Sent::default(), Booked::default());
        let all = [turn(Some("one")), turn(Some("two")), turn(Some("three"))];
        let emit = |n: &str, p: Value| sent.borrow_mut().push((n.into(), p));
        let usage = |a: &str, t: u64, c: f64| booked.borrow_mut().push((a.into(), t, c));
        let ctx = Ctx { dir: &d, sharing: true, token_saver: false, redact: &|t| Some(t.into()), emit: &emit, usage: &usage };
        // The cursor offset stands in for the transcript position: how many turns were read.
        let step = |finish: bool, upto: u64| {
            read_with(&ctx, &job("t9"), finish, |c| {
                let done = (0..=3).find(|n| *c == cursor_at(*n)).unwrap_or(0) as usize;
                *c = cursor_at(upto);
                all[done..upto as usize].to_vec()
            })
        };
        assert_eq!([step(false, 1), step(false, 1), step(false, 2), step(true, 3)], [1, 0, 1, 1]);
        let sent = sent.into_inner();
        let texts: Vec<&str> = sent.iter().map(|(_, p)| p["$ai_input"][0]["content"].as_str().unwrap()).collect();
        assert_eq!(texts, ["one", "two", "three"]);
        // Only `$ai_generation` is sent for tokens: the old `agent_tokens` event is retired.
        assert!(sent.iter().all(|(n, _)| n == "$ai_generation"));
        assert_eq!(booked.borrow().len(), 3);
    }

    #[test]
    fn a_trace_names_the_run_and_each_run_has_its_own_id() {
        assert_eq!(
            trace_props("t1", "codex", 125, 4),
            json!({ "$ai_trace_id": "t1", "$ai_span_name": "codex", "$ai_latency": 125, "turns": 4, "remote": false })
        );
        assert_ne!(new_trace_id(), new_trace_id());
    }
}
