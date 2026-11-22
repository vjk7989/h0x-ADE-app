//! Token totals and per-turn records of an agent session (Claude Code / Codex)
//! read from the agent's own on-disk session logs. `usage` keeps only counters,
//! model names, ids and timestamps; `turns` also returns the typed prompt text.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Upper bound on log bytes read by one `usage` call.
const MAX_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub model: Option<String>,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
    pub cost_usd: Option<f64>,
}

/// Token totals for the agent session that ran in `folder` between `start` and
/// `end`. `None` when nothing was found or the logs are unreadable.
pub fn usage(kind: AgentKind, folder: &Path, start: SystemTime, end: SystemTime) -> Option<Usage> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let home = PathBuf::from(env("USERPROFILE").or_else(|| env("HOME"))?);
    usage_in(&home, kind, folder, start, end)
}

fn usage_in(
    home: &Path,
    kind: AgentKind,
    folder: &Path,
    start: SystemTime,
    end: SystemTime,
) -> Option<Usage> {
    let (start_ms, end_ms) = (millis(start)?, millis(end)?);
    let mut budget = MAX_BYTES;
    let mut out = match kind {
        AgentKind::Claude => claude(home, folder, start, start_ms, end_ms, &mut budget)?,
        AgentKind::Codex => codex(home, folder, start, start_ms, end_ms, &mut budget)?,
    };
    out.cost_usd = out.model.as_deref().and_then(|m| cost(m, &out));
    Some(out)
}

fn millis(t: SystemTime) -> Option<i64> {
    let d = t.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// Claude Code project directory name: every non-alphanumeric character of
/// the folder path becomes '-' (`G:\pavii-ade` -> `G--pavii-ade`).
fn slug(folder: &Path) -> String {
    folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff]Z` (UTC) into unix milliseconds.
fn parse_ts(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let mut frac = 0;
    let mut rest = &s[19..];
    if let Some(f) = rest.strip_prefix('.') {
        let digits: String = f.chars().take_while(|c| c.is_ascii_digit()).collect();
        let take: String = digits.chars().take(3).collect();
        frac = format!("{take:0<3}").parse::<i64>().ok()?;
        rest = &f[digits.len()..];
    }
    if rest != "Z" {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + frac)
}

/// Visit the lines of a file, charging `budget`; stops when it is exhausted.
fn for_each_line(path: &Path, budget: &mut u64, mut f: impl FnMut(&[u8])) {
    let Ok(file) = File::open(path) else { return };
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let n = n as u64;
                if n > *budget {
                    *budget = 0;
                    return;
                }
                *budget -= n;
                f(&buf);
            }
        }
    }
}

fn modified_since(path: &Path, start: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|m| m >= start)
}

#[derive(serde::Deserialize)]
struct ClaudeLine {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<ClaudeMsg>,
}

#[derive(serde::Deserialize)]
struct ClaudeMsg {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(serde::Deserialize, Default, Clone, Copy)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

fn claude(
    home: &Path,
    folder: &Path,
    start: SystemTime,
    start_ms: i64,
    end_ms: i64,
    budget: &mut u64,
) -> Option<Usage> {
    let dir = home.join(".claude").join("projects").join(slug(folder));
    let mut by_id: HashMap<String, ClaudeUsage> = HashMap::new();
    let mut anonymous: Vec<ClaudeUsage> = Vec::new();
    let mut model: Option<String> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl")
            || !modified_since(&path, start)
        {
            continue;
        }
        for_each_line(&path, budget, |line| {
            let Ok(l) = serde_json::from_slice::<ClaudeLine>(line) else {
                return;
            };
            let Some(ts) = l.timestamp.as_deref().and_then(parse_ts) else {
                return;
            };
            if ts < start_ms || ts > end_ms {
                return;
            }
            let Some(msg) = l.message else { return };
            let Some(u) = msg.usage else { return };
            if msg.model.is_some() {
                model = msg.model;
            }
            let key = match (msg.id, l.request_id) {
                (Some(id), _) => Some(format!("m:{id}")),
                (None, Some(id)) => Some(format!("r:{id}")),
                (None, None) => None,
            };
            match key {
                // A streamed response repeats its id with growing counters.
                Some(id) => {
                    let e = by_id.entry(id).or_default();
                    e.input_tokens = e.input_tokens.max(u.input_tokens);
                    e.output_tokens = e.output_tokens.max(u.output_tokens);
                    e.cache_read_input_tokens =
                        e.cache_read_input_tokens.max(u.cache_read_input_tokens);
                    e.cache_creation_input_tokens = e
                        .cache_creation_input_tokens
                        .max(u.cache_creation_input_tokens);
                }
                None => anonymous.push(u),
            }
        });
    }
    if by_id.is_empty() && anonymous.is_empty() {
        return None;
    }
    let mut out = Usage {
        model,
        ..Usage::default()
    };
    for u in by_id.values().chain(anonymous.iter()) {
        out.input = out.input.saturating_add(u.input_tokens.unwrap_or(0));
        out.output = out.output.saturating_add(u.output_tokens.unwrap_or(0));
        out.cache_read = out.cache_read.saturating_add(u.cache_read_input_tokens.unwrap_or(0));
        out.cache_write = out.cache_write.saturating_add(u.cache_creation_input_tokens.unwrap_or(0));
    }
    Some(out)
}

#[derive(serde::Deserialize)]
struct CodexLine {
    timestamp: Option<String>,
    payload: Option<CodexPayload>,
}

#[derive(serde::Deserialize)]
struct CodexPayload {
    cwd: Option<String>,
    model: Option<String>,
    info: Option<CodexInfo>,
}

#[derive(serde::Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexTotals>,
}

#[derive(serde::Deserialize, Default, Clone, Copy)]
struct CodexTotals {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
}

impl CodexTotals {
    fn weight(&self) -> u64 {
        self.input_tokens
            .unwrap_or(0)
            .saturating_add(self.output_tokens.unwrap_or(0))
    }
}

fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

fn numeric_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| e.path())
        .collect()
}

fn codex(
    home: &Path,
    folder: &Path,
    start: SystemTime,
    start_ms: i64,
    end_ms: i64,
    budget: &mut u64,
) -> Option<Usage> {
    let root = home.join(".codex").join("sessions");
    let want = folder.to_string_lossy();
    let mut out = Usage::default();
    let mut found = false;
    for year in numeric_subdirs(&root) {
        for month in numeric_subdirs(&year) {
            for day in numeric_subdirs(&month) {
                let Ok(rd) = std::fs::read_dir(&day) else {
                    continue;
                };
                for entry in rd.flatten() {
                    let path = entry.path();
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !name.starts_with("rollout-")
                        || !name.ends_with(".jsonl")
                        || !modified_since(&path, start)
                    {
                        continue;
                    }
                    let mut best: Option<CodexTotals> = None;
                    let mut model = None;
                    let mut cwd_ok = false;
                    let mut cwd_seen = false;
                    for_each_line(&path, budget, |line| {
                        if cwd_seen && !cwd_ok {
                            return;
                        }
                        let Ok(l) = serde_json::from_slice::<CodexLine>(line) else {
                            return;
                        };
                        let Some(p) = l.payload else { return };
                        if !cwd_seen {
                            if let Some(c) = p.cwd.as_deref() {
                                cwd_seen = true;
                                cwd_ok = same_dir(c, &want);
                                if !cwd_ok {
                                    return;
                                }
                            }
                        }
                        if p.model.is_some() {
                            model = p.model;
                        }
                        let Some(t) = p.info.and_then(|i| i.total_token_usage) else {
                            return;
                        };
                        let in_window = l
                            .timestamp
                            .as_deref()
                            .and_then(parse_ts)
                            .is_some_and(|ts| ts >= start_ms && ts <= end_ms);
                        if in_window && best.is_none_or(|b| t.weight() >= b.weight()) {
                            best = Some(t);
                        }
                    });
                    if !cwd_ok {
                        continue;
                    }
                    let Some(t) = best else { continue };
                    found = true;
                    // Codex input counts include cached tokens; split them.
                    let cached = t.cached_input_tokens.unwrap_or(0);
                    out.input = out
                        .input
                        .saturating_add(t.input_tokens.unwrap_or(0).saturating_sub(cached));
                    out.cache_read = out.cache_read.saturating_add(cached);
                    out.output = out.output.saturating_add(t.output_tokens.unwrap_or(0));
                    out.reasoning = out
                        .reasoning
                        .saturating_add(t.reasoning_output_tokens.unwrap_or(0));
                    if model.is_some() {
                        out.model = model;
                    }
                }
            }
        }
    }
    found.then_some(out)
}

/// Hard cap for one transcript line; longer lines are skipped unparsed.
const MAX_LINE: u64 = 8 * 1024 * 1024;

/// One agent turn: a typed prompt and everything the agent did until the next one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    /// Unix milliseconds of the prompt.
    pub timestamp_ms: i64,
    pub model: Option<String>,
    /// `"anthropic"` or `"openai"`.
    pub provider: &'static str,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: Option<f64>,
    /// Prompt to last transcript line of the turn; `None` when not derivable.
    pub latency_ms: Option<u64>,
    pub tools: Vec<String>,
    /// Server part of `mcp__<server>__<tool>`.
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    /// What the person typed (Claude user text blocks / Codex `user_message`).
    /// Never tool results or text the agent injected. Not redacted here.
    pub prompt: Option<String>,
}

/// Where the last `turns` call stopped, per transcript file. Keep it between
/// calls so a turn is never returned twice.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TurnCursor {
    offsets: HashMap<PathBuf, u64>,
}

/// New completed turns of the session that ran in `folder` with a prompt in
/// `start..=end`, read from `cursor`. The turn still being written stays
/// unread (the cursor holds at its prompt) unless `finish` is set, which
/// closes it too (agent ended).
pub fn turns(
    kind: AgentKind,
    folder: &Path,
    start: SystemTime,
    end: SystemTime,
    cursor: &mut TurnCursor,
    finish: bool,
) -> Vec<Turn> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let Some(home) = env("USERPROFILE").or_else(|| env("HOME")) else {
        return Vec::new();
    };
    turns_in(&PathBuf::from(home), kind, folder, (start, end), cursor, finish, MAX_BYTES)
}

fn turns_in(
    home: &Path,
    kind: AgentKind,
    folder: &Path,
    (start, end): (SystemTime, SystemTime),
    cursor: &mut TurnCursor,
    finish: bool,
    mut budget: u64,
) -> Vec<Turn> {
    let (Some(start_ms), Some(end_ms)) = (millis(start), millis(end)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match kind {
        AgentKind::Claude => {
            let dir = home.join(".claude").join("projects").join(slug(folder));
            let Ok(rd) = std::fs::read_dir(dir) else {
                return out;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()) == Some("jsonl")
                        && modified_since(p, start)
                })
                .collect();
            files.sort();
            for path in files {
                out.extend(scan_turns(&path, cursor, &mut budget, finish, claude_line));
            }
        }
        AgentKind::Codex => {
            let want = folder.to_string_lossy();
            for path in codex_files(&home.join(".codex").join("sessions"), start) {
                if !codex_cwd(&path).is_some_and(|c| same_dir(&c, &want)) {
                    continue;
                }
                let mut model: Option<String> = None;
                out.extend(scan_turns(&path, cursor, &mut budget, finish, |v, off, open, done| {
                    codex_line(&mut model, v, off, open, done)
                }));
            }
        }
    }
    out.retain(|t| t.timestamp_ms >= start_ms && t.timestamp_ms <= end_ms);
    out.sort_by_key(|t| t.timestamp_ms);
    out
}

/// A turn being built.
struct Open {
    start_off: u64,
    ts: i64,
    last_ts: i64,
    provider: &'static str,
    prompt: Option<String>,
    model: Option<String>,
    /// [input, output, cache read, cache write], max-merged per message id
    /// because a streamed response repeats its id with growing counters.
    by_id: HashMap<String, [u64; 4]>,
    anonymous: Vec<[u64; 4]>,
    last_total: Option<serde_json::Value>,
    tools: Vec<String>,
    mcp_servers: Vec<String>,
    skills: Vec<String>,
}

impl Open {
    fn new(provider: &'static str, off: u64, ts: i64, prompt: Option<String>) -> Self {
        Open {
            start_off: off,
            ts,
            last_ts: ts,
            provider,
            prompt,
            model: None,
            by_id: HashMap::new(),
            anonymous: Vec::new(),
            last_total: None,
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            skills: Vec::new(),
        }
    }

    fn tool(&mut self, name: &str) {
        push_unique(&mut self.tools, name);
        if let Some(server) = name.strip_prefix("mcp__").and_then(|r| r.split("__").next())
            && !server.is_empty()
        {
            push_unique(&mut self.mcp_servers, server);
        }
    }

    /// `None` when the agent produced nothing (interrupted before replying).
    fn close(self) -> Option<Turn> {
        if self.by_id.is_empty() && self.anonymous.is_empty() {
            return None;
        }
        let mut t = Turn {
            timestamp_ms: self.ts,
            provider: self.provider,
            model: self.model,
            latency_ms: u64::try_from(self.last_ts - self.ts).ok().filter(|d| *d > 0),
            tools: self.tools,
            mcp_servers: self.mcp_servers,
            skills: self.skills,
            prompt: self.prompt,
            ..Turn::default()
        };
        for u in self.by_id.values().chain(self.anonymous.iter()) {
            t.input = t.input.saturating_add(u[0]);
            t.output = t.output.saturating_add(u[1]);
            t.cache_read = t.cache_read.saturating_add(u[2]);
            t.cache_write = t.cache_write.saturating_add(u[3]);
        }
        let totals = Usage {
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            ..Usage::default()
        };
        t.cost_usd = t.model.as_deref().and_then(|m| cost(m, &totals));
        Some(t)
    }
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// Read one transcript from its cursor offset. The offset moves to the end of
/// the last whole line, or back to the open turn's prompt when it is unfinished.
fn scan_turns(
    path: &Path,
    cursor: &mut TurnCursor,
    budget: &mut u64,
    finish: bool,
    mut handle: impl FnMut(&serde_json::Value, u64, &mut Option<Open>, &mut Vec<Turn>),
) -> Vec<Turn> {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut from = cursor.offsets.get(path).copied().unwrap_or(0);
    if from > len {
        from = 0; // file was replaced
    }
    let mut open: Option<Open> = None;
    let mut done = Vec::new();
    let end = lines_from(path, from, budget, |line, off| {
        match serde_json::from_slice::<serde_json::Value>(line) {
            Ok(v) => handle(&v, off, &mut open, &mut done),
            // Half-written last line: leave it for the next call.
            Err(_) if !line.ends_with(b"\n") => return false,
            Err(_) => {}
        }
        true
    });
    if finish {
        done.extend(open.take().and_then(Open::close));
    }
    cursor
        .offsets
        .insert(path.to_path_buf(), open.map_or(end, |o| o.start_off));
    done
}

/// Visit whole lines from byte `from`, charging `budget`. `f` returns whether
/// it consumed the line. Returns the offset after the last consumed line.
/// Lines over `MAX_LINE` are skipped without being parsed.
fn lines_from(
    path: &Path,
    from: u64,
    budget: &mut u64,
    mut f: impl FnMut(&[u8], u64) -> bool,
) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = File::open(path) else {
        return from;
    };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return from;
    }
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut pos = from;
    loop {
        buf.clear();
        let n = match (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return pos,
            Ok(n) => n as u64,
        };
        if n > *budget {
            *budget = 0;
            return pos;
        }
        *budget -= n;
        if n > MAX_LINE && !buf.ends_with(b"\n") {
            let mut skipped = n;
            let mut scratch = Vec::new();
            loop {
                scratch.clear();
                match (&mut reader).take(64 * 1024).read_until(b'\n', &mut scratch) {
                    Ok(0) | Err(_) => break,
                    Ok(m) => {
                        skipped += m as u64;
                        *budget = budget.saturating_sub(m as u64);
                        if scratch.ends_with(b"\n") {
                            break;
                        }
                    }
                }
            }
            pos += skipped;
            continue;
        }
        if !f(&buf, pos) {
            return pos;
        }
        pos += n;
    }
}

/// Markers Claude Code puts at the start of text it injects into a user message.
const INJECTED: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<local-command-caveat>",
    "<system-reminder>",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
    "[Request interrupted",
];

fn typed_text(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty() && !INJECTED.iter().any(|m| s.starts_with(m))).then_some(s)
}

fn claude_line(
    v: &serde_json::Value,
    off: u64,
    open: &mut Option<Open>,
    done: &mut Vec<Turn>,
) {
    let Some(ts) = v["timestamp"].as_str().and_then(parse_ts) else {
        return;
    };
    let msg = &v["message"];
    match v["type"].as_str() {
        Some("user") => {
            let flag = |k: &str| v[k].as_bool() == Some(true);
            if flag("isMeta")
                || flag("isSidechain")
                || flag("isCompactSummary")
                || v.get("sourceToolUseID").is_some()
                || v.get("toolUseResult").is_some()
            {
                return;
            }
            let prompt = match &msg["content"] {
                serde_json::Value::String(s) => typed_text(s).map(str::to_string),
                serde_json::Value::Array(blocks) => {
                    if blocks.iter().any(|b| b["type"].as_str() == Some("tool_result")) {
                        return;
                    }
                    let texts: Vec<&str> = blocks
                        .iter()
                        .filter(|b| b["type"].as_str() == Some("text"))
                        .filter_map(|b| b["text"].as_str().and_then(typed_text))
                        .collect();
                    (!texts.is_empty()).then(|| texts.join("\n"))
                }
                _ => return,
            };
            done.extend(
                open.replace(Open::new("anthropic", off, ts, prompt))
                    .and_then(Open::close),
            );
        }
        Some("assistant") => {
            let Some(o) = open.as_mut() else { return };
            o.last_ts = o.last_ts.max(ts);
            if let Some(m) = msg["model"].as_str() {
                o.model = Some(m.to_string());
            }
            if let Some(blocks) = msg["content"].as_array() {
                for b in blocks.iter().filter(|b| b["type"].as_str() == Some("tool_use")) {
                    let Some(name) = b["name"].as_str() else { continue };
                    o.tool(name);
                    if name == "Skill"
                        && let Some(s) = b["input"]["skill"]
                            .as_str()
                            .or_else(|| b["input"]["name"].as_str())
                    {
                        push_unique(&mut o.skills, s);
                    }
                }
            }
            let u = &msg["usage"];
            if !u.is_object() {
                return;
            }
            let n = |k: &str| u[k].as_u64().unwrap_or(0);
            let now = [
                n("input_tokens"),
                n("output_tokens"),
                n("cache_read_input_tokens"),
                n("cache_creation_input_tokens"),
            ];
            let key = match (msg["id"].as_str(), v["requestId"].as_str()) {
                (Some(id), _) => Some(format!("m:{id}")),
                (None, Some(id)) => Some(format!("r:{id}")),
                (None, None) => None,
            };
            match key {
                Some(k) => {
                    let e = o.by_id.entry(k).or_default();
                    for (e, n) in e.iter_mut().zip(now) {
                        *e = (*e).max(n);
                    }
                }
                None => o.anonymous.push(now),
            }
        }
        _ => {}
    }
}

fn codex_line(
    model: &mut Option<String>,
    v: &serde_json::Value,
    off: u64,
    open: &mut Option<Open>,
    done: &mut Vec<Turn>,
) {
    let Some(ts) = v["timestamp"].as_str().and_then(parse_ts) else {
        return;
    };
    let p = &v["payload"];
    if let Some(m) = p["model"].as_str() {
        *model = Some(m.to_string());
        if let Some(o) = open.as_mut() {
            o.model = Some(m.to_string());
        }
    }
    if v["type"].as_str() == Some("event_msg") && p["type"].as_str() == Some("user_message") {
        let prompt = p["message"].as_str().and_then(typed_text).map(str::to_string);
        let mut o = Open::new("openai", off, ts, prompt);
        o.model = model.clone();
        done.extend(open.replace(o).and_then(Open::close));
        return;
    }
    let Some(o) = open.as_mut() else { return };
    o.last_ts = o.last_ts.max(ts);
    match (v["type"].as_str(), p["type"].as_str()) {
        (Some("response_item"), Some("function_call" | "custom_tool_call")) => {
            if let Some(name) = p["name"].as_str() {
                o.tool(name);
            }
        }
        (Some("event_msg"), Some("token_count")) => {
            let info = &p["info"];
            let (total, last) = (&info["total_token_usage"], &info["last_token_usage"]);
            // Codex repeats a token_count event with unchanged totals.
            if last.is_object() && o.last_total.as_ref() != Some(total) {
                o.last_total = Some(total.clone());
                let n = |k: &str| last[k].as_u64().unwrap_or(0);
                let cached = n("cached_input_tokens");
                // Codex input counts include cached tokens; split them.
                o.anonymous
                    .push([n("input_tokens").saturating_sub(cached), n("output_tokens"), cached, 0]);
            }
        }
        _ => {}
    }
}

fn codex_files(root: &Path, start: SystemTime) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for year in numeric_subdirs(root) {
        for month in numeric_subdirs(&year) {
            for day in numeric_subdirs(&month) {
                let Ok(rd) = std::fs::read_dir(&day) else {
                    continue;
                };
                for e in rd.flatten() {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("rollout-")
                        && name.ends_with(".jsonl")
                        && modified_since(&e.path(), start)
                    {
                        out.push(e.path());
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Working directory a Codex rollout ran in (from its first lines).
fn codex_cwd(path: &Path) -> Option<String> {
    let mut budget = MAX_LINE;
    let mut cwd = None;
    let mut seen = 0;
    lines_from(path, 0, &mut budget, |line, _| {
        seen += 1;
        if cwd.is_none() && seen <= 20 {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
                cwd = v["payload"]["cwd"].as_str().map(str::to_string);
            }
        }
        cwd.is_none() && seen < 20
    });
    cwd
}

/// USD per 1M tokens: (model name contains, input, output, cache read, cache write).
/// Update when prices change. First match wins, so specific names come first.
const PRICES: &[(&str, f64, f64, f64, f64)] = &[
    ("claude-3-opus", 15.0, 75.0, 1.5, 18.75),
    ("claude-opus-4-0", 15.0, 75.0, 1.5, 18.75),
    ("claude-opus-4-1", 15.0, 75.0, 1.5, 18.75),
    ("opus", 5.0, 25.0, 0.5, 6.25),
    ("sonnet", 3.0, 15.0, 0.3, 3.75),
    ("haiku", 1.0, 5.0, 0.1, 1.25),
    ("codex-mini", 0.25, 2.0, 0.025, 0.0),
    ("gpt-5-mini", 0.25, 2.0, 0.025, 0.0),
    ("gpt-5-nano", 0.05, 0.4, 0.005, 0.0),
    ("gpt-5", 1.25, 10.0, 0.125, 0.0),
    ("codex", 1.25, 10.0, 0.125, 0.0),
];

fn cost(model: &str, u: &Usage) -> Option<f64> {
    let model = model.to_lowercase();
    let (_, pin, pout, pcr, pcw) = PRICES.iter().find(|(k, ..)| model.contains(k))?;
    let per = |n: u64, p: f64| n as f64 * p / 1_000_000.0;
    Some(
        per(u.input, *pin)
            + per(u.output, *pout)
            + per(u.cache_read, *pcr)
            + per(u.cache_write, *pcw),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("h0x-agent-usage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    // 2026-01-01T00:00:00Z
    const T0: u64 = 1_767_225_600;

    #[test]
    fn slug_rule() {
        assert_eq!(slug(Path::new(r"G:\pavii-ade")), "G--pavii-ade");
        assert_eq!(
            slug(Path::new(r"G:\pavii-ade\.cache\my proj")),
            "G--pavii-ade--cache-my-proj"
        );
        assert_eq!(slug(Path::new("/home/u/app")), "-home-u-app");
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("2026-01-01T00:00:00Z"), Some(T0 as i64 * 1000));
        assert_eq!(
            parse_ts("2026-01-01T00:00:01.5Z"),
            Some(T0 as i64 * 1000 + 1500)
        );
        assert_eq!(parse_ts("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_ts("garbage"), None);
        assert_eq!(parse_ts(""), None);
    }

    #[test]
    fn claude_sums_dedupes_and_hides_content() {
        let home = tmp("claude");
        let folder = Path::new(r"G:\my proj\app");
        let dir = home.join(".claude/projects").join(slug(folder));
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            r#"{"timestamp":"2026-01-01T00:00:10Z","requestId":"r1","message":{"id":"m1","model":"claude-opus-4-5","content":"SECRET-PROMPT","usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":100,"cache_creation_input_tokens":5}}}"#,
            r#"{"timestamp":"2026-01-01T00:00:11Z","requestId":"r1","message":{"id":"m1","model":"claude-opus-4-5","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":100,"cache_creation_input_tokens":5}}}"#,
            r#"{"timestamp":"2026-01-01T00:00:12Z","requestId":"r2","message":{"model":"claude-opus-4-5","usage":{"input_tokens":1,"output_tokens":2}}}"#,
            r#"{"timestamp":"2025-12-31T00:00:00Z","requestId":"r3","message":{"usage":{"input_tokens":999,"output_tokens":999}}}"#,
            r#"{"timestamp":"2026-02-01T00:00:00Z","requestId":"r4","message":{"usage":{"input_tokens":999,"output_tokens":999}}}"#,
            "not json at all",
            r#"{"timestamp":"2026-01-01T00:00:13Z","message":{"content":"SECRET-REPLY"}}"#,
            "",
        ];
        std::fs::write(dir.join("a.jsonl"), lines.join("\n")).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let u = usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 3600)).unwrap();
        assert_eq!(u.input, 11);
        assert_eq!(u.output, 22);
        assert_eq!(u.cache_read, 100);
        assert_eq!(u.cache_write, 5);
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-5"));
        assert!(u.cost_usd.unwrap() > 0.0);
        assert!(!format!("{u:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_missing_or_empty_is_none() {
        let home = tmp("claude-none");
        let folder = Path::new(r"G:\nowhere");
        assert!(usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 10)).is_none());
        let dir = home.join(".claude/projects").join(slug(folder));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "").unwrap();
        assert!(usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 10)).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_takes_max_cumulative_and_matches_cwd() {
        let home = tmp("codex");
        let day = home.join(".codex/sessions/2026/01/01");
        std::fs::create_dir_all(&day).unwrap();
        let tc = |ts: &str, i: u64, c: u64, o: u64, r: u64| {
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{i},"cached_input_tokens":{c},"output_tokens":{o},"reasoning_output_tokens":{r}}}}}}}}}"#
            )
        };
        let mine = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"session_meta","payload":{"cwd":"G:\\My Proj\\","base_instructions":"SECRET-SYS"}}"#.to_string(),
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}"#.to_string(),
            tc("2026-01-01T00:00:05Z", 100, 40, 10, 3),
            tc("2026-01-01T00:00:09Z", 300, 100, 50, 7),
            tc("2026-03-01T00:00:09Z", 9000, 0, 9000, 0),
            "{broken".to_string(),
        ];
        std::fs::write(day.join("rollout-a.jsonl"), mine.join("\n")).unwrap();
        let other = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"session_meta","payload":{"cwd":"G:\\else"}}"#.to_string(),
            tc("2026-01-01T00:00:05Z", 5000, 0, 5000, 0),
        ];
        std::fs::write(day.join("rollout-b.jsonl"), other.join("\n")).unwrap();
        let u = usage_in(
            &home,
            AgentKind::Codex,
            Path::new(r"G:\my proj"),
            at(T0),
            at(T0 + 3600),
        )
        .unwrap();
        assert_eq!(
            (u.input, u.cache_read, u.output, u.reasoning),
            (200, 100, 50, 7)
        );
        assert_eq!(u.model.as_deref(), Some("gpt-5-codex"));
        assert!(u.cost_usd.is_some());
        assert!(!format!("{u:?}").contains("SECRET"));
        let none = usage_in(
            &home,
            AgentKind::Codex,
            Path::new(r"G:\zzz"),
            at(T0),
            at(T0 + 3600),
        );
        assert!(none.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unknown_model_has_no_cost() {
        let u = Usage {
            input: 1,
            ..Usage::default()
        };
        assert_eq!(cost("mystery-model", &u), None);
        assert!(cost("claude-sonnet-4-5", &u).is_some());
    }

    #[test]
    fn slug_rule_handles_non_ascii_spaces_and_unc_paths() {
        assert_eq!(
            slug(Path::new(r"C:\Users\Jörg Müller\my app")),
            "C--Users-J-rg-M-ller-my-app"
        );
        assert_eq!(slug(Path::new(r"\\server\share\x")), "--server-share-x");
        assert_eq!(slug(Path::new("")), "");
    }

    #[test]
    fn timestamps_reject_offsets_and_keep_millisecond_boundaries() {
        // Only UTC `Z` timestamps are read; an offset is rejected, not converted.
        assert_eq!(parse_ts("2026-01-01T05:30:00+05:30"), None);
        assert_eq!(parse_ts("2026-01-01T00:00:00+00:00"), None);
        assert_eq!(parse_ts("2026-01-01T00:00:00z"), None);
        assert_eq!(parse_ts("2026-01-01 00:00:00Z"), None);
        assert_eq!(parse_ts("2026-01-01T00:00:00ZZ"), None);
        assert_eq!(parse_ts("2026-01-01T24:00:00Z"), None);
        assert_eq!(parse_ts("2026-01-01T00:00:0éZ"), None);
        assert_eq!(
            parse_ts("2026-01-01T00:00:00.123456789Z"),
            Some(T0 as i64 * 1000 + 123)
        );
        assert_eq!(parse_ts("2000-02-29T23:59:59.999Z"), Some(951_868_799_999));
        assert_eq!(parse_ts("2100-03-01T00:00:00Z"), Some(4_107_542_400_000));
    }

    fn claude_dir(home: &Path, folder: &Path) -> PathBuf {
        let dir = home.join(".claude").join("projects").join(slug(folder));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn claude_window_is_inclusive_and_a_backwards_window_finds_nothing() {
        let home = tmp("claude-window");
        let folder = Path::new(r"C:\Users\Jörg Müller\my app");
        let line = |ts: &str, id: &str, n: u64| {
            format!(
                r#"{{"timestamp":"{ts}","requestId":"{id}","message":{{"usage":{{"input_tokens":{n}}}}}}}"#
            )
        };
        let lines = [
            line("2026-01-01T00:00:00Z", "a", 1),
            line("2026-01-01T00:00:10Z", "b", 2),
            line("2026-01-01T00:00:10.001Z", "c", 4),
            line("2025-12-31T23:59:59.999Z", "d", 8),
            line("2026-01-01T05:30:00+05:30", "e", 16),
        ];
        std::fs::write(claude_dir(&home, folder).join("s.jsonl"), lines.join("\n")).unwrap();
        let u = usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 10)).unwrap();
        assert_eq!(u.input, 3);
        // The clock went backwards: end before start.
        assert!(usage_in(&home, AgentKind::Claude, folder, at(T0 + 10), at(T0)).is_none());
        assert!(usage_in(&home, AgentKind::Codex, folder, at(T0 + 10), at(T0)).is_none());
        // A start before 1970 is not a window.
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert!(usage_in(&home, AgentKind::Claude, folder, before, at(T0 + 10)).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_keeps_the_max_of_each_field_and_sums_lines_without_ids() {
        let home = tmp("claude-max");
        let folder = Path::new(r"G:\max");
        let ts = "2026-01-01T00:00:05Z";
        let mut text = Vec::new();
        for l in [
            // Same requestId: the larger value of each field, even when a later line is smaller.
            format!(
                r#"{{"timestamp":"{ts}","requestId":"r1","message":{{"model":"mystery-1","usage":{{"input_tokens":10,"output_tokens":1}}}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","requestId":"r1","message":{{"usage":{{"input_tokens":5,"output_tokens":20}}}}}}"#
            ),
            // No requestId: message.id dedupes.
            format!(
                r#"{{"timestamp":"{ts}","message":{{"id":"m9","usage":{{"input_tokens":3}}}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","message":{{"id":"m9","usage":{{"input_tokens":4}}}}}}"#
            ),
            // No id at all: every line counts.
            format!(
                r#"{{"timestamp":"{ts}","message":{{"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","message":{{"usage":{{"input_tokens":2,"output_tokens":2}}}}}}"#
            ),
        ] {
            // Windows line endings.
            text.extend_from_slice(l.as_bytes());
            text.extend_from_slice(b"\r\n");
        }
        // A line that is not UTF-8 is skipped.
        text.extend_from_slice(b"\xff\xfe{\"timestamp\"\r\n");
        std::fs::write(claude_dir(&home, folder).join("s.jsonl"), text).unwrap();
        let u = usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 60)).unwrap();
        assert_eq!((u.input, u.output), (10 + 4 + 1 + 2, 20 + 1 + 2));
        assert_eq!(u.model.as_deref(), Some("mystery-1"));
        assert_eq!(u.cost_usd, None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_huge_line_is_counted_without_keeping_its_text() {
        let home = tmp("claude-huge");
        let folder = Path::new(r"G:\huge");
        let secret = format!("SECRET{}", "x".repeat(4 * 1024 * 1024));
        let line = format!(
            r#"{{"timestamp":"2026-01-01T00:00:05Z","requestId":"r1","message":{{"content":"{secret}","usage":{{"input_tokens":7}}}}}}"#
        );
        std::fs::write(claude_dir(&home, folder).join("s.jsonl"), line).unwrap();
        let u = usage_in(&home, AgentKind::Claude, folder, at(T0), at(T0 + 60)).unwrap();
        assert_eq!(u.input, 7);
        assert!(!format!("{u:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_budget_admits_a_line_of_exactly_its_size_and_a_missing_file_costs_nothing() {
        let home = tmp("budget-edge");
        let p = home.join("x.jsonl");
        std::fs::write(&p, "aaaa\nbbbb\n").unwrap();
        let (mut budget, mut n) = (5, 0);
        for_each_line(&p, &mut budget, |_| n += 1);
        assert_eq!((n, budget), (1, 0));
        let (mut budget, mut n) = (10, 0);
        for_each_line(&p, &mut budget, |_| n += 1);
        assert_eq!((n, budget), (2, 0));
        let (mut budget, mut n) = (0, 0);
        for_each_line(&p, &mut budget, |_| n += 1);
        assert_eq!((n, budget), (0, 0));
        let (mut budget, mut n) = (10, 0);
        for_each_line(&home.join("missing.jsonl"), &mut budget, |_| n += 1);
        assert_eq!((n, budget), (0, 10));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_sums_matching_rollouts_and_ignores_other_names_and_folders() {
        let home = tmp("codex-sum");
        let sessions = home.join(".codex").join("sessions");
        let tc = |ts: &str, i: u64, c: u64, o: u64, r: u64| {
            format!(
                r#"{{"timestamp":"{ts}","payload":{{"info":{{"total_token_usage":{{"input_tokens":{i},"cached_input_tokens":{c},"output_tokens":{o},"reasoning_output_tokens":{r}}}}}}}}}"#
            )
        };
        let write = |rel: &str, cwd: &str, rest: &[String]| {
            let p = sessions.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let meta =
                format!(r#"{{"timestamp":"2026-01-01T00:00:01Z","payload":{{"cwd":"{cwd}"}}}}"#);
            let mut lines = vec![meta];
            lines.extend_from_slice(rest);
            std::fs::write(p, lines.join("\n")).unwrap();
        };
        // The same folder written two ways (case, separators, trailing slash, non-ASCII).
        write(
            "2026/01/01/rollout-a.jsonl",
            "c:/users/jörg müller/app/",
            &[
                tc("2026-01-01T00:00:05Z", 100, 40, 10, 1),
                // No timestamp: not in the window.
                r#"{"payload":{"info":{"total_token_usage":{"input_tokens":9999,"output_tokens":9999}}}}"#.to_string(),
            ],
        );
        // More cached than input: input saturates at 0.
        write(
            "2026/01/02/rollout-b.jsonl",
            r"C:\\Users\\JÖRG Müller\\app",
            &[tc("2026-01-01T00:00:06Z", 50, 60, 5, 0)],
        );
        // Not a rollout file, and not under a numeric year folder.
        write(
            "2026/01/01/notes.jsonl",
            "c:/users/jörg müller/app",
            &[tc("2026-01-01T00:00:05Z", 7000, 0, 7000, 0)],
        );
        write(
            "archive/01/01/rollout-c.jsonl",
            "c:/users/jörg müller/app",
            &[tc("2026-01-01T00:00:05Z", 8000, 0, 8000, 0)],
        );
        let u = usage_in(
            &home,
            AgentKind::Codex,
            Path::new(r"C:\Users\JÖRG Müller\app"),
            at(T0),
            at(T0 + 3600),
        )
        .unwrap();
        assert_eq!(
            (u.input, u.cache_read, u.output, u.reasoning),
            (60, 100, 15, 1)
        );
        // No model line: no model, so no cost.
        assert_eq!((u.model, u.cost_usd), (None, None));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_empty_home_finds_nothing() {
        let folder = Path::new(r"Z:\h0x-agent-usage-no-such-folder");
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            assert!(usage_in(Path::new(""), kind, folder, at(T0), at(T0 + 10)).is_none());
        }
    }

    #[test]
    fn prices_pick_the_specific_model_first_and_ignore_case() {
        let million = |input, output, cache_read, cache_write| Usage {
            input,
            output,
            cache_read,
            cache_write,
            ..Usage::default()
        };
        let one_in = million(1_000_000, 0, 0, 0);
        assert_eq!(cost("gpt-5-mini", &one_in), Some(0.25));
        assert_eq!(cost("GPT-5", &one_in), Some(1.25));
        assert_eq!(cost("gpt-5-nano-2025", &one_in), Some(0.05));
        assert_eq!(
            cost(
                "claude-opus-4-5",
                &million(1_000_000, 1_000_000, 1_000_000, 1_000_000)
            ),
            Some(5.0 + 25.0 + 0.5 + 6.25)
        );
        assert_eq!(cost("claude-haiku-4-5", &Usage::default()), Some(0.0));
        assert_eq!(cost("", &one_in), None);
    }

    #[test]
    fn budget_stops_reading() {
        let home = tmp("budget");
        let p = home.join("x.jsonl");
        std::fs::write(&p, "aaaa\nbbbb\n").unwrap();
        let mut budget = 6;
        let mut n = 0;
        for_each_line(&p, &mut budget, |_| n += 1);
        assert_eq!(n, 1);
        assert_eq!(budget, 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    fn claude_home(name: &str, folder: &Path, lines: &[String]) -> (PathBuf, PathBuf) {
        let home = tmp(name);
        let dir = home.join(".claude/projects").join(slug(folder));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.jsonl");
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
        (home, file)
    }

    fn user(ts: &str, content: &str) -> String {
        format!(r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":{content}}}}}"#)
    }

    fn asst(ts: &str, id: &str, out: u64, content: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"req-{id}","message":{{"id":"{id}","model":"claude-sonnet-4-5","content":{content},"usage":{{"input_tokens":10,"output_tokens":{out},"cache_read_input_tokens":100,"cache_creation_input_tokens":5}}}}}}"#
        )
    }

    fn run(home: &Path, folder: &Path, cur: &mut TurnCursor, finish: bool) -> Vec<Turn> {
        turns_in(home, AgentKind::Claude, folder, (at(T0), at(T0 + 3600)), cur, finish, MAX_BYTES)
    }

    fn two_turns() -> Vec<String> {
        vec![
            user("2026-01-01T00:00:10Z", r#""first prompt""#),
            asst(
                "2026-01-01T00:00:12Z",
                "m1",
                5,
                r#"[{"type":"tool_use","name":"mcp__github__create_issue","input":{}},{"type":"tool_use","name":"Skill","input":{"skill":"h0x-pipeline"}},{"type":"tool_use","name":"Bash","input":{}}]"#,
            ),
            // streamed repeat of m1 with a larger counter
            asst("2026-01-01T00:00:13Z", "m1", 9, r#"[]"#),
            user(
                "2026-01-01T00:00:14Z",
                r#"[{"type":"tool_result","tool_use_id":"t","content":"SECRET-FILE"}]"#,
            ),
            asst("2026-01-01T00:00:16Z", "m2", 3, r#"[{"type":"tool_use","name":"Bash","input":{}}]"#),
            user("2026-01-01T00:01:00Z", r#"[{"type":"text","text":"second prompt"}]"#),
            asst("2026-01-01T00:01:05Z", "m3", 7, r#"[]"#),
        ]
    }

    #[test]
    fn turns_multi_turn_dedup_names_and_latency() {
        let folder = Path::new(r"G:\my proj\app");
        let (home, _) = claude_home("t-multi", folder, &two_turns());
        let mut cur = TurnCursor::default();
        let t = run(&home, folder, &mut cur, true);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].prompt.as_deref(), Some("first prompt"));
        assert_eq!(t[0].provider, "anthropic");
        assert_eq!((t[0].input, t[0].output, t[0].cache_read, t[0].cache_write), (20, 12, 200, 10));
        assert_eq!(t[0].tools, ["mcp__github__create_issue", "Skill", "Bash"]);
        assert_eq!(t[0].mcp_servers, ["github"]);
        assert_eq!(t[0].skills, ["h0x-pipeline"]);
        assert_eq!(t[0].latency_ms, Some(6000));
        assert_eq!(t[0].model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(t[0].cost_usd.unwrap() > 0.0);
        assert_eq!(t[0].timestamp_ms, (T0 as i64 + 10) * 1000);
        assert_eq!(t[1].prompt.as_deref(), Some("second prompt"));
        assert_eq!(t[1].output, 7);
        assert_eq!(t[1].latency_ms, Some(5000));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tool_result_and_injected_text_never_become_the_prompt() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user("2026-01-01T00:00:01Z", r#""<command-name>/clear</command-name>""#),
            asst("2026-01-01T00:00:02Z", "a", 1, "[]"),
            user("2026-01-01T00:00:03Z", r#"[{"type":"tool_result","content":"SECRET-FILE"},{"type":"text","text":"SECRET-TEXT"}]"#),
            r#"{"type":"user","isMeta":true,"timestamp":"2026-01-01T00:00:04Z","message":{"role":"user","content":"<local-command-caveat>x</local-command-caveat>"}}"#.to_string(),
            user("2026-01-01T00:00:05Z", r#""<local-command-stdout>SECRET-OUT</local-command-stdout>""#),
            asst("2026-01-01T00:00:06Z", "b", 1, "[]"),
        ];
        let (home, _) = claude_home("t-inject", folder, &lines);
        let t = run(&home, folder, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|t| t.prompt.is_none()));
        assert!(!format!("{t:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cursor_resume_returns_only_new_turns() {
        let folder = Path::new(r"G:\my proj\app");
        let all = two_turns();
        let (home, file) = claude_home("t-cursor", folder, &all[..5]);
        let mut cur = TurnCursor::default();
        // turn 1 is still open: nothing is final yet
        assert!(run(&home, folder, &mut cur, false).is_empty());
        std::fs::write(&file, all.join("\n") + "\n").unwrap();
        let t = run(&home, folder, &mut cur, false);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("first prompt"));
        assert_eq!(t[0].output, 12);
        assert!(run(&home, folder, &mut cur, false).is_empty());
        // agent ended: the open second turn closes, exactly once
        let t = run(&home, folder, &mut cur, true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("second prompt"));
        assert!(run(&home, folder, &mut cur, true).is_empty());
        // cursor survives a round trip through JSON
        let json = serde_json::to_string(&cur).unwrap();
        assert_eq!(serde_json::from_str::<TurnCursor>(&json).unwrap(), cur);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn half_written_line_is_retried_not_lost() {
        let folder = Path::new("/home/u/app");
        let (home, file) = claude_home("t-half", folder, &two_turns()[..2]);
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str(&user("2026-01-01T00:01:00Z", r#""second""#)[..20]);
        std::fs::write(&file, &text).unwrap();
        let mut cur = TurnCursor::default();
        assert!(run(&home, folder, &mut cur, false).is_empty());
        std::fs::write(&file, two_turns().join("\n") + "\n").unwrap();
        assert_eq!(run(&home, folder, &mut cur, false).len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn turns_empty_missing_and_oversized() {
        let folder = Path::new("/home/u/app");
        let mut cur = TurnCursor::default();
        let home = tmp("t-missing");
        assert!(run(&home, folder, &mut cur, true).is_empty());
        let (home, file) = claude_home("t-empty", folder, &[]);
        assert!(run(&home, folder, &mut cur, true).is_empty());
        // a line over MAX_LINE is skipped, the turn after it is still read
        let mut text = String::from("{\"x\":\"");
        text.push_str(&"a".repeat(MAX_LINE as usize + 10));
        text.push_str("\"}\n");
        text.push_str(&two_turns().join("\n"));
        text.push('\n');
        std::fs::write(&file, text).unwrap();
        assert_eq!(run(&home, folder, &mut TurnCursor::default(), true).len(), 2);
        // over the read budget: stop safely, nothing consumed
        let mut cur = TurnCursor::default();
        let t = turns_in(&home, AgentKind::Claude, folder, (at(T0), at(T0 + 3600)), &mut cur, true, 1000);
        assert!(t.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn turns_respect_the_time_window() {
        let folder = Path::new("/home/u/app");
        let (home, _) = claude_home("t-window", folder, &two_turns());
        let t = turns_in(
            &home,
            AgentKind::Claude,
            folder,
            (at(T0 + 30), at(T0 + 3600)),
            &mut TurnCursor::default(),
            true,
            MAX_BYTES,
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("second prompt"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_turns_split_per_user_message() {
        let home = tmp("t-codex");
        let day = home.join(".codex/sessions/2026/01/01");
        std::fs::create_dir_all(&day).unwrap();
        let tc = |ts: &str, ti: u64, li: u64, lc: u64, lo: u64| {
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{ti}}},"last_token_usage":{{"input_tokens":{li},"cached_input_tokens":{lc},"output_tokens":{lo}}}}}}}}}"#
            )
        };
        let um = |ts: &str, m: &str| {
            format!(r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"user_message","message":"{m}"}}}}"#)
        };
        let lines = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"session_meta","payload":{"cwd":"G:\\My Proj\\"}}"#.to_string(),
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}"#.to_string(),
            um("2026-01-01T00:00:10Z", "do a thing"),
            r#"{"timestamp":"2026-01-01T00:00:11Z","type":"response_item","payload":{"type":"function_call","name":"shell"}}"#.to_string(),
            r#"{"timestamp":"2026-01-01T00:00:11Z","type":"response_item","payload":{"type":"function_call","name":"mcp__docs__search"}}"#.to_string(),
            tc("2026-01-01T00:00:12Z", 100, 100, 40, 10),
            tc("2026-01-01T00:00:12Z", 100, 100, 40, 10),
            tc("2026-01-01T00:00:15Z", 250, 150, 50, 20),
            um("2026-01-01T00:01:00Z", "again"),
            tc("2026-01-01T00:01:02Z", 300, 50, 0, 5),
        ];
        std::fs::write(day.join("rollout-a.jsonl"), lines.join("\n") + "\n").unwrap();
        let other = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"session_meta","payload":{"cwd":"G:\\else"}}"#;
        std::fs::write(day.join("rollout-b.jsonl"), other).unwrap();
        let mut cur = TurnCursor::default();
        let go = |cur: &mut TurnCursor, finish| {
            turns_in(&home, AgentKind::Codex, Path::new(r"G:\my proj"), (at(T0), at(T0 + 3600)), cur, finish, MAX_BYTES)
        };
        let t = go(&mut cur, false);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].provider, "openai");
        assert_eq!(t[0].prompt.as_deref(), Some("do a thing"));
        assert_eq!((t[0].input, t[0].cache_read, t[0].output), (160, 90, 30));
        assert_eq!(t[0].tools, ["shell", "mcp__docs__search"]);
        assert_eq!(t[0].mcp_servers, ["docs"]);
        assert_eq!(t[0].model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(t[0].latency_ms, Some(5000));
        let t = go(&mut cur, true);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].prompt.as_deref(), t[0].input, t[0].output), (Some("again"), 50, 5));
        assert!(go(&mut cur, true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
