//! Per-turn records of an agent session HyperOx itself started, read from the
//! agent's own on-disk session log and chosen by the exact session id (never by
//! folder or time, so a session started in a plain terminal is never read).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Upper bound on log bytes read by one `turns` call.
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

fn millis(t: SystemTime) -> Option<i64> {
    let d = t.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
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
    /// Claude: the prompt message uuid, else the first reply's message id.
    /// `None` for Codex.
    pub id: Option<String>,
}

/// Where the last `turns` call stopped, per transcript file. Keep it between
/// calls so a turn is never returned twice.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TurnCursor {
    offsets: HashMap<PathBuf, u64>,
    /// Ids of the most recent turns returned, so a session copied into another
    /// file (resume) is not counted twice.
    seen: VecDeque<String>,
}

/// Recent turn ids a cursor remembers.
const MAX_SEEN: usize = 10_000;

/// Model name Claude Code logs for its own zero-usage error replies.
const SYNTHETIC: &str = "<synthetic>";

/// A session id is a canonical lower-case UUID (Claude names its logs so), not nil: no path
/// separator, `..` or anything else that could leave the log folder.
pub fn valid_session_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
        && s.bytes().any(|b| b != b'0' && b != b'-')
}

/// The Claude log `<session>.jsonl` under any project folder of `~/.claude/projects`.
fn claude_file(home: &Path, session: &str) -> Option<PathBuf> {
    if !valid_session_id(session) {
        return None;
    }
    let name = format!("{session}.jsonl");
    std::fs::read_dir(home.join(".claude").join("projects"))
        .ok()?
        .flatten()
        .map(|e| e.path().join(&name))
        .find(|p| p.is_file())
}

/// New completed turns of the session `session` (the id HyperOx gave the agent
/// at launch) with a prompt in `start..=end`, read from `cursor`. The turn still
/// being written stays unread (the cursor holds at its prompt) unless `finish`
/// is set, which closes it too (agent ended). No id, an invalid id or no log of
/// that id: no turns. Codex: no turns until it has a per-launch id (planned
/// per-launch notify, SA-3); it is never matched by folder or time.
pub fn turns(
    kind: AgentKind,
    session: &str,
    start: SystemTime,
    end: SystemTime,
    cursor: &mut TurnCursor,
    finish: bool,
) -> Vec<Turn> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let Some(home) = env("USERPROFILE").or_else(|| env("HOME")) else {
        return Vec::new();
    };
    turns_in(&PathBuf::from(home), kind, session, (start, end), cursor, finish, MAX_BYTES)
}

fn turns_in(
    home: &Path,
    kind: AgentKind,
    session: &str,
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
            if let Some(path) = claude_file(home, session) {
                out.extend(scan_turns(&path, cursor, &mut budget, finish, claude_line));
            }
        }
        AgentKind::Codex => {}
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
    id: Option<String>,
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
            id: None,
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
            id: self.id,
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
    let (end, clean) = lines_from(path, from, budget, |line, off| {
        match serde_json::from_slice::<serde_json::Value>(line) {
            Ok(v) => handle(&v, off, &mut open, &mut done),
            // Half-written last line: leave it for the next call.
            Err(_) if !line.ends_with(b"\n") => return false,
            Err(_) => {}
        }
        true
    });
    // An unfinished read (budget, half-written line) must not close the turn.
    if finish && clean {
        done.extend(open.take().and_then(Open::close));
    }
    cursor
        .offsets
        .insert(path.to_path_buf(), open.map_or(end, |o| o.start_off));
    done.retain(|t| match &t.id {
        Some(id) if cursor.seen.contains(id) => false,
        Some(id) => {
            cursor.seen.push_back(id.clone());
            if cursor.seen.len() > MAX_SEEN {
                cursor.seen.pop_front();
            }
            true
        }
        None => true,
    });
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
) -> (u64, bool) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = File::open(path) else {
        return (from, false);
    };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return (from, false);
    }
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut pos = from;
    loop {
        buf.clear();
        let n = match (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut buf) {
            Ok(0) => return (pos, true),
            Err(_) => return (pos, false),
            Ok(n) => n as u64,
        };
        if n > *budget {
            *budget = 0;
            return (pos, false);
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
            return (pos, false);
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
    "<user-prompt-submit-hook>",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
    "[Request interrupted",
];

/// What the person typed: the text before the first injected tag. Nothing
/// after an injected tag is kept, and a compact summary is not a prompt.
fn typed_text(s: &str) -> Option<&str> {
    let cut = INJECTED.iter().filter_map(|m| s.find(m)).min().unwrap_or(s.len());
    let s = s[..cut].trim();
    (!s.is_empty() && !s.starts_with(COMPACT_SUMMARY)).then_some(s)
}

const COMPACT_SUMMARY: &str = "This session is being continued from a previous conversation";

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
            let mut o = Open::new("anthropic", off, ts, prompt);
            o.id = v["uuid"].as_str().map(|u| format!("u:{u}"));
            done.extend(open.replace(o).and_then(Open::close));
        }
        Some("assistant") => {
            let Some(o) = open.as_mut() else { return };
            o.last_ts = o.last_ts.max(ts);
            if let Some(m) = msg["model"].as_str().filter(|m| *m != SYNTHETIC) {
                o.model = Some(m.to_string());
            }
            if o.id.is_none()
                && let Some(id) = msg["id"].as_str()
            {
                o.id = Some(format!("m:{id}"));
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

    const SID: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER: &str = "22222222-2222-4222-8222-222222222222";

    // 2026-01-01T00:00:00Z
    const T0: u64 = 1_767_225_600;

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
    fn unknown_model_has_no_cost() {
        let u = Usage {
            input: 1,
            ..Usage::default()
        };
        assert_eq!(cost("mystery-model", &u), None);
        assert!(cost("claude-sonnet-4-5", &u).is_some());
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

    fn claude_home(name: &str, folder: &Path, lines: &[String]) -> (PathBuf, PathBuf) {
        let home = tmp(name);
        let project: String = folder.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let dir = home.join(".claude/projects").join(project);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{SID}.jsonl"));
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

    fn run(home: &Path, session: &str, cur: &mut TurnCursor, finish: bool) -> Vec<Turn> {
        turns_in(home, AgentKind::Claude, session, (at(T0), at(T0 + 3600)), cur, finish, MAX_BYTES)
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
        let t = run(&home, SID, &mut cur, true);
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
        let t = run(&home, SID, &mut TurnCursor::default(), true);
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
        assert!(run(&home, SID, &mut cur, false).is_empty());
        std::fs::write(&file, all.join("\n") + "\n").unwrap();
        let t = run(&home, SID, &mut cur, false);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("first prompt"));
        assert_eq!(t[0].output, 12);
        assert!(run(&home, SID, &mut cur, false).is_empty());
        // agent ended: the open second turn closes, exactly once
        let t = run(&home, SID, &mut cur, true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("second prompt"));
        assert!(run(&home, SID, &mut cur, true).is_empty());
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
        assert!(run(&home, SID, &mut cur, false).is_empty());
        std::fs::write(&file, two_turns().join("\n") + "\n").unwrap();
        assert_eq!(run(&home, SID, &mut cur, false).len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn turns_empty_missing_and_oversized() {
        let folder = Path::new("/home/u/app");
        let mut cur = TurnCursor::default();
        let home = tmp("t-missing");
        assert!(run(&home, SID, &mut cur, true).is_empty());
        let (home, file) = claude_home("t-empty", folder, &[]);
        assert!(run(&home, SID, &mut cur, true).is_empty());
        // a line over MAX_LINE is skipped, the turn after it is still read
        let mut text = String::from("{\"x\":\"");
        text.push_str(&"a".repeat(MAX_LINE as usize + 10));
        text.push_str("\"}\n");
        text.push_str(&two_turns().join("\n"));
        text.push('\n');
        std::fs::write(&file, text).unwrap();
        assert_eq!(run(&home, SID, &mut TurnCursor::default(), true).len(), 2);
        // over the read budget: stop safely, nothing consumed
        let mut cur = TurnCursor::default();
        let t = turns_in(&home, AgentKind::Claude, SID, (at(T0), at(T0 + 3600)), &mut cur, true, 1000);
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
            SID,
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
    fn prompt_keeps_typed_text_blocks_and_drops_images_reminders_and_tool_results() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            // pasted file content in a text block was typed by the person: kept
            user(
                "2026-01-01T00:00:01Z",
                r#"[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"SECRET-IMG"}},{"type":"text","text":" \n\t<system-reminder>SECRET-REMINDER</system-reminder>"},{"type":"text","text":"look at this"},{"type":"text","text":"pasted file:\nfn main() {}"}]"#,
            ),
            asst("2026-01-01T00:00:02Z", "a", 1, r#"[{"type":"tool_use","name":"Read","input":{}}]"#),
            // a message that is only a tool_result continues the turn
            user(
                "2026-01-01T00:00:03Z",
                r#"[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"SECRET-RESULT"}]}]"#,
            ),
            asst("2026-01-01T00:00:04Z", "b", 2, "[]"),
            // image only: a new turn without a prompt
            user(
                "2026-01-01T00:00:05Z",
                r#"[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"SECRET-IMG2"}}]"#,
            ),
            asst("2026-01-01T00:00:06Z", "c", 4, "[]"),
        ];
        let (home, _) = claude_home("t-blocks", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].prompt.as_deref(), Some("look at this\npasted file:\nfn main() {}"));
        assert_eq!((t[0].output, t[0].latency_ms), (3, Some(3000)));
        assert_eq!(t[0].tools, ["Read"]);
        assert_eq!((t[1].prompt.as_deref(), t[1].output), (None, 4));
        assert!(!format!("{t:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn compact_summary_sidechain_hook_and_bash_lines_never_become_the_prompt() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"timestamp":"2026-01-01T00:00:01Z","message":{"role":"user","content":"This session is being continued from a previous conversation that ran out of context. SECRET-SUMMARY"}}"#.to_string(),
            r#"{"type":"user","isSidechain":true,"timestamp":"2026-01-01T00:00:02Z","message":{"role":"user","content":"SECRET-SIDECHAIN task prompt"}}"#.to_string(),
            r#"{"type":"user","sourceToolUseID":"toolu_1","timestamp":"2026-01-01T00:00:03Z","message":{"role":"user","content":[{"type":"text","text":"SECRET-SKILL-BODY"}]}}"#.to_string(),
            r#"{"type":"system","subtype":"stop_hook_summary","timestamp":"2026-01-01T00:00:04Z","content":"SECRET-HOOK"}"#.to_string(),
            r#"{"type":"attachment","timestamp":"2026-01-01T00:00:05Z","attachment":{"type":"hook_additional_context","content":["SECRET-HOOK-CONTEXT"]}}"#.to_string(),
            user("2026-01-01T00:00:06Z", r#""<bash-input>cat .env</bash-input>""#),
            user(
                "2026-01-01T00:00:07Z",
                r#""<bash-stdout>SECRET-STDOUT</bash-stdout><bash-stderr></bash-stderr>""#,
            ),
            asst("2026-01-01T00:00:08Z", "a", 1, "[]"),
            user("2026-01-01T00:00:09Z", r#""[Request interrupted by user for tool use]""#),
            asst("2026-01-01T00:00:10Z", "b", 1, "[]"),
            // whitespace only
            user("2026-01-01T00:00:11Z", r#""  \n  ""#),
            asst("2026-01-01T00:00:12Z", "c", 1, "[]"),
            user("2026-01-01T00:00:13Z", r#""real prompt""#),
            asst("2026-01-01T00:00:14Z", "d", 1, "[]"),
        ];
        let (home, _) = claude_home("t-meta", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        let prompts: Vec<_> = t.iter().map(|t| t.prompt.as_deref()).collect();
        assert_eq!(prompts, [None, None, None, Some("real prompt")]);
        assert!(!format!("{t:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn user_prompt_submit_hook_text_never_becomes_the_prompt() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user(
                "2026-01-01T00:00:01Z",
                r#"[{"type":"text","text":"fix the build"},{"type":"text","text":"<user-prompt-submit-hook>SECRET-HOOK-OUTPUT</user-prompt-submit-hook>"}]"#,
            ),
            asst("2026-01-01T00:00:02Z", "a", 1, "[]"),
        ];
        let (home, _) = claude_home("t-hook", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("fix the build"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn injected_command_output_after_typed_text_never_becomes_the_prompt() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user(
                "2026-01-01T00:00:01Z",
                r#""check this\n<local-command-stdout>SECRET-OUT</local-command-stdout>""#,
            ),
            asst("2026-01-01T00:00:02Z", "a", 1, "[]"),
        ];
        let (home, _) = claude_home("t-midtag", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 1);
        assert!(!format!("{t:?}").contains("SECRET"), "{:?}", t[0].prompt);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_plain_terminal_session_in_the_same_folder_and_time_window_is_not_read() {
        let folder = Path::new(r"G:\my proj\app");
        let ours = vec![
            user("2026-01-01T00:00:10Z", r#""a1""#),
            asst("2026-01-01T00:00:20Z", "x1", 1, "[]"),
        ];
        let (home, file) = claude_home("t-plain", folder, &ours);
        let plain = [
            user("2026-01-01T00:00:15Z", r#""SECRET-b1""#),
            asst("2026-01-01T00:00:30Z", "y1", 10, "[]"),
        ];
        std::fs::write(file.with_file_name(format!("{OTHER}.jsonl")), plain.join("\n") + "\n").unwrap();
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.iter().map(|t| (t.prompt.as_deref(), t.output)).collect::<Vec<_>>(), [(Some("a1"), 1)]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_missing_or_unsafe_session_id_reads_nothing() {
        let folder = Path::new("/home/u/app");
        let (home, file) = claude_home("t-ids", folder, &two_turns());
        std::fs::write(home.join(".claude").join("evil.jsonl"), two_turns().join("\n") + "\n").unwrap();
        let bad = [
            "".to_string(),
            "s".to_string(),
            "..".to_string(),
            "../evil".to_string(),
            r"..\evil".to_string(),
            format!("../{SID}"),
            format!("{SID}/x"),
            format!(r"{SID}\x"),
            "11111111-1111-4111-8111-11111111111".to_string(),
            "11111111-1111-4111-8111-1111111111111".to_string(),
            "1111111g-1111-4111-8111-111111111111".to_string(),
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".to_string(),
            "00000000-0000-0000-0000-000000000000".to_string(),
        ];
        for b in &bad {
            assert!(!valid_session_id(b), "{b:?}");
            assert!(run(&home, b, &mut TurnCursor::default(), true).is_empty(), "{b:?}");
        }
        assert!(valid_session_id(SID));
        // A valid id with no log: nothing.
        assert!(run(&home, OTHER, &mut TurnCursor::default(), true).is_empty());
        assert_eq!(run(&home, SID, &mut TurnCursor::default(), true).len(), 2);
        // Codex is never read, with or without an id.
        let codex = |s: &str| turns_in(&home, AgentKind::Codex, s, (at(T0), at(T0 + 3600)), &mut TurnCursor::default(), true, MAX_BYTES);
        assert!(codex(SID).is_empty() && codex("").is_empty());
        assert!(file.is_file());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn non_canonical_uuid_spellings_read_nothing_and_case_is_accepted() {
        let folder = Path::new("/home/u/app");
        let (home, _) = claude_home("t-noncanon", folder, &two_turns());
        let no_dash = SID.replace('-', "");
        for b in [
            format!("{{{SID}}}"),
            format!("urn:uuid:{SID}"),
            no_dash.clone(),
            format!("{no_dash}0000"),
            format!(" {SID}"),
            format!("{SID} "),
            format!("{SID}\0"),
            format!("{SID}.jsonl"),
            SID.replacen('-', "_", 1),
            SID.replacen('-', "1", 1).replacen('1', "-", 1),
            format!("{}-", &SID[..35]),
            "-".repeat(36),
            "é".repeat(18),
            format!("{}*", &SID[..35]),
            format!("{}?", &SID[..35]),
        ] {
            assert!(!valid_session_id(&b), "{b:?}");
            assert!(run(&home, &b, &mut TurnCursor::default(), true).is_empty(), "{b:?}");
        }
        // Lower-case hex in the 8-4-4-4-12 shape, v4 or not: valid. Upper/mixed case and the nil: not.
        for ok in ["aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "11111111-1111-1111-c111-111111111111"] {
            assert!(valid_session_id(ok), "{ok}");
        }
        for bad in ["AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE", "aAaAaAaA-bBbB-4cCc-8dDd-eEeEeEeEeEeE", "00000000-0000-0000-0000-000000000000"] {
            assert!(!valid_session_id(bad), "{bad}");
        }
        // An upper-case spelling of our id reads nothing.
        assert!(run(&home, "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA", &mut TurnCursor::default(), true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_same_session_id_in_two_project_folders_reads_exactly_one_log() {
        // Windows-like home with a space and non-ASCII letters.
        let home = tmp("t-two-dirs Jörg Müller");
        let projects = home.join(".claude").join("projects");
        let one = projects.join("C--Users-J-rg-M-ller-a");
        let two = projects.join("C--Users-J-rg-M-ller-b");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        let log = |p: &str, id: &str| [user("2026-01-01T00:00:10Z", &format!(r#""{p}""#)), asst("2026-01-01T00:00:20Z", id, 1, "[]")].join("\n") + "\n";
        std::fs::write(one.join(format!("{SID}.jsonl")), log("in-a", "ma")).unwrap();
        std::fs::write(two.join(format!("{SID}.jsonl")), log("in-b", "mb")).unwrap();
        let mut cur = TurnCursor::default();
        let t = run(&home, SID, &mut cur, true);
        // Which folder wins depends on directory order; never both, never merged.
        assert_eq!(t.len(), 1, "{t:?}");
        assert!(matches!(t[0].prompt.as_deref(), Some("in-a" | "in-b")), "{t:?}");
        assert_eq!(cur.offsets.len(), 1);
        // The same cursor does not then pick up the other folder's copy.
        assert!(run(&home, SID, &mut cur, true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_directory_named_like_the_log_or_a_projects_file_reads_nothing_wrong() {
        let home = tmp("t-dir-named-log");
        let projects = home.join(".claude").join("projects");
        // A directory called `<id>.jsonl` is skipped; the real log in another folder is read.
        std::fs::create_dir_all(projects.join("a").join(format!("{SID}.jsonl"))).unwrap();
        std::fs::create_dir_all(projects.join("b")).unwrap();
        std::fs::write(projects.join("b").join(format!("{SID}.jsonl")), two_turns().join("\n") + "\n").unwrap();
        assert_eq!(run(&home, SID, &mut TurnCursor::default(), true).len(), 2);
        // A log directly in `projects` (no project folder) is not one of ours.
        let flat = tmp("t-flat-log");
        std::fs::create_dir_all(flat.join(".claude").join("projects")).unwrap();
        std::fs::write(flat.join(".claude").join("projects").join(format!("{SID}.jsonl")), two_turns().join("\n") + "\n").unwrap();
        assert!(run(&flat, SID, &mut TurnCursor::default(), true).is_empty());
        // `projects` is a file, not a folder: nothing, no panic.
        let odd = tmp("t-projects-file");
        std::fs::create_dir_all(odd.join(".claude")).unwrap();
        std::fs::write(odd.join(".claude").join("projects"), "x").unwrap();
        assert!(run(&odd, SID, &mut TurnCursor::default(), true).is_empty());
        // An empty log and an empty home: nothing.
        let empty = tmp("t-empty-log");
        std::fs::create_dir_all(empty.join(".claude/projects/p")).unwrap();
        std::fs::write(empty.join(".claude/projects/p").join(format!("{SID}.jsonl")), "").unwrap();
        assert!(run(&empty, SID, &mut TurnCursor::default(), true).is_empty());
        assert!(run(&tmp("t-bare-home"), SID, &mut TurnCursor::default(), true).is_empty());
        for d in [home, flat, odd, empty, tmp("t-bare-home")] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_project_folder_is_followed_and_a_dangling_link_reads_nothing() {
        use std::os::unix::fs::symlink;
        let home = tmp("t-symlink");
        let elsewhere = tmp("t-symlink-target");
        std::fs::write(elsewhere.join(format!("{SID}.jsonl")), two_turns().join("\n") + "\n").unwrap();
        let projects = home.join(".claude").join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        symlink(&elsewhere, projects.join("linked")).unwrap();
        // The log is chosen by its file name (our id), so a linked folder holding it is read.
        assert_eq!(run(&home, SID, &mut TurnCursor::default(), true).len(), 2);
        // A dangling link and a link to a log of another id read nothing.
        let other = tmp("t-symlink-other");
        std::fs::create_dir_all(other.join(".claude/projects/p")).unwrap();
        symlink(elsewhere.join("gone.jsonl"), other.join(".claude/projects/p").join(format!("{SID}.jsonl"))).unwrap();
        assert!(run(&other, SID, &mut TurnCursor::default(), true).is_empty());
        assert!(run(&home, OTHER, &mut TurnCursor::default(), true).is_empty());
        for d in [home, elsewhere, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn a_large_log_is_read_within_the_budget_across_calls() {
        let folder = Path::new("/home/u/app");
        let ts = |s: u64| format!("2026-01-01T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60);
        let pad = "x".repeat(4096);
        let mut lines = Vec::new();
        for i in 0..200u64 {
            lines.push(user(&ts(2 * i), &format!(r#""p{i} {pad}""#)));
            lines.push(asst(&ts(2 * i + 1), &format!("m{i}"), 1, "[]"));
        }
        let (home, file) = claude_home("t-large", folder, &lines);
        let len = std::fs::metadata(&file).unwrap().len();
        assert!(len > 800_000);
        let window = (at(T0), at(T0 + 86_399));
        // A zero budget reads nothing and does not move the cursor.
        let mut cur = TurnCursor::default();
        assert!(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, 0).is_empty());
        assert_eq!(cur.offsets.get(&file).copied().unwrap_or(0), 0);
        // A budget one byte under the first line: still nothing.
        let first = lines[0].len() as u64 + 1;
        assert!(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, first - 1).is_empty());
        assert_eq!(cur.offsets.get(&file).copied().unwrap_or(0), 0);
        // 64 KiB per call: each call stays bounded, and all turns arrive exactly once.
        let mut all = Vec::new();
        let mut calls = 0;
        loop {
            let before = cur.offsets.get(&file).copied().unwrap_or(0);
            all.extend(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, 64 * 1024));
            let after = cur.offsets.get(&file).copied().unwrap_or(0);
            assert!(after - before <= 64 * 1024, "{before}..{after}");
            calls += 1;
            if after == len || calls > 100 {
                break;
            }
        }
        assert_eq!(all.len(), 200);
        assert!(all.windows(2).all(|w| w[0].timestamp_ms < w[1].timestamp_ms));
        assert!(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, MAX_BYTES).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn turn_copied_into_a_resumed_session_file_is_counted_once() {
        // A resumed Claude session copies earlier lines (same message ids) into a
        // new file of another id; only the file of our id is read.
        let folder = Path::new("/home/u/app");
        let (home, file) = claude_home("t-resume", folder, &two_turns());
        std::fs::write(file.with_file_name("resumed.jsonl"), two_turns().join("\n") + "\n").unwrap();
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 2, "{t:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_model_switch_between_turns_keeps_each_turns_model() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user("2026-01-01T00:00:01Z", r#""p1""#),
            asst("2026-01-01T00:00:02Z", "a", 5, "[]"),
            user("2026-01-01T00:00:10Z", r#""p2""#),
            asst("2026-01-01T00:00:11Z", "b", 5, "[]").replace("claude-sonnet-4-5", "claude-opus-4-1"),
            user("2026-01-01T00:00:20Z", r#""p3""#),
            asst("2026-01-01T00:00:21Z", "c", 5, "[]"),
        ];
        let (home, _) = claude_home("t-model", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        let models: Vec<_> = t.iter().map(|t| t.model.as_deref()).collect();
        assert_eq!(
            models,
            [Some("claude-sonnet-4-5"), Some("claude-opus-4-1"), Some("claude-sonnet-4-5")]
        );
        assert!(t[1].cost_usd.unwrap() > t[0].cost_usd.unwrap());
        assert_eq!(t[0].cost_usd, t[2].cost_usd);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn synthetic_error_reply_does_not_replace_the_turns_model() {
        // Claude Code logs API errors as an assistant message with model
        // "<synthetic>" and zero usage after the real reply.
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user("2026-01-01T00:00:01Z", r#""p1""#),
            asst("2026-01-01T00:00:02Z", "a", 5, "[]"),
            r#"{"type":"assistant","timestamp":"2026-01-01T00:00:03Z","message":{"id":"syn","model":"<synthetic>","content":[{"type":"text","text":"API Error"}],"usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#.to_string(),
        ];
        let (home, _) = claude_home("t-synthetic", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(t[0].cost_usd.is_some());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn window_by_prompt_time_is_inclusive_at_both_edges_across_midnight() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            // prompt before midnight, reply after: outside, the window is by prompt time
            user("2025-12-31T23:59:59.999Z", r#""before midnight""#),
            asst("2026-01-01T00:00:00.000Z", "a", 1, "[]"),
            user("2026-01-01T00:00:00.000Z", r#""at start""#),
            asst("2026-01-01T00:30:00Z", "b", 1, "[]"),
            user("2026-01-01T01:00:00.000Z", r#""at end""#),
            asst("2026-01-01T01:00:00.500Z", "c", 1, "[]"),
            user("2026-01-01T01:00:00.001Z", r#""after end""#),
            asst("2026-01-01T01:00:01Z", "d", 1, "[]"),
        ];
        let (home, _) = claude_home("t-edges", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        let got: Vec<_> = t.iter().map(|t| (t.prompt.as_deref(), t.latency_ms)).collect();
        assert_eq!(got, [(Some("at start"), Some(1_800_000)), (Some("at end"), Some(500))]);
        // inverted window: nothing
        let none = turns_in(
            &home,
            AgentKind::Claude,
            SID,
            (at(T0 + 3600), at(T0)),
            &mut TurnCursor::default(),
            true,
            MAX_BYTES,
        );
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_or_malformed_timestamps_and_content_are_skipped_without_leaking() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user("2026-01-01T00:00:01Z", r#""first""#),
            asst("2026-01-01T00:00:02Z", "a", 1, "[]"),
            r#"{"type":"user","message":{"role":"user","content":"SECRET-NO-TS"}}"#.to_string(),
            user("2026-13-01T00:00:00Z", r#""SECRET-BAD-MONTH""#),
            user("2026-01-01T00:00:03+05:30", r#""SECRET-OFFSET""#),
            user("not a time", r#""SECRET-GARBAGE""#),
            r#"{"type":"user","timestamp":42,"message":{"role":"user","content":"SECRET-NUMBER-TS"}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-01-01T00:00:04Z","message":{"role":"user","content":null}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-01-01T00:00:04Z"}"#.to_string(),
            "not json at all".to_string(),
            asst("2026-01-01T00:00:05Z", "b", 2, "[]"),
        ];
        let (home, _) = claude_home("t-badts", folder, &lines);
        let t = run(&home, SID, &mut TurnCursor::default(), true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("first"));
        assert!(!format!("{t:?}").contains("SECRET"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn deleted_then_shorter_rewritten_file_is_read_from_the_start() {
        let folder = Path::new("/home/u/app");
        let (home, file) = claude_home("t-rotate", folder, &two_turns());
        let mut cur = TurnCursor::default();
        assert_eq!(run(&home, SID, &mut cur, true).len(), 2);
        // the cursor still names the deleted file: harmless
        std::fs::remove_file(&file).unwrap();
        assert!(run(&home, SID, &mut cur, true).is_empty());
        let fresh = [
            user("2026-01-01T00:02:00Z", r#""after rotate""#),
            asst("2026-01-01T00:02:01Z", "n", 1, "[]"),
        ];
        std::fs::write(&file, fresh.join("\n") + "\n").unwrap();
        let t = run(&home, SID, &mut cur, true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("after rotate"));
        assert!(run(&home, SID, &mut cur, true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn finish_after_a_budget_stop_does_not_close_a_half_read_turn() {
        let folder = Path::new("/home/u/app");
        let lines = vec![
            user("2026-01-01T00:00:01Z", r#""p1""#),
            asst("2026-01-01T00:00:02Z", "m1", 5, "[]"),
            asst("2026-01-01T00:00:03Z", "m2", 7, "[]"),
            user("2026-01-01T00:00:04Z", r#""p2""#),
            asst("2026-01-01T00:00:05Z", "m3", 1, "[]"),
        ];
        let (home, file) = claude_home("t-budget", folder, &lines);
        let window = (at(T0), at(T0 + 3600));
        // a budget of exactly the file size reads everything
        let len = std::fs::metadata(&file).unwrap().len();
        let t = turns_in(&home, AgentKind::Claude, SID, window, &mut TurnCursor::default(), true, len);
        assert_eq!(t.len(), 2);
        // the budget runs out after p1's first reply; the rest comes next call
        let cut = (lines[0].len() + lines[1].len() + 2) as u64;
        let mut cur = TurnCursor::default();
        let mut all = turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, cut);
        all.extend(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, MAX_BYTES));
        let got: Vec<_> = all.iter().map(|t| (t.prompt.as_deref(), t.output)).collect();
        assert_eq!(got, [(Some("p1"), 12), (Some("p2"), 1)]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn ten_thousand_tiny_turns_parse_in_a_few_seconds() {
        let folder = Path::new("/home/u/app");
        let ts = |s: u64| format!("2026-01-01T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60);
        let mut lines = Vec::with_capacity(20_000);
        for i in 0..10_000u64 {
            lines.push(user(&ts(2 * i), r#""p""#));
            lines.push(asst(&ts(2 * i + 1), &format!("m{i}"), 1, "[]"));
        }
        let (home, _) = claude_home("t-many", folder, &lines);
        let window = (at(T0), at(T0 + 86_399));
        let started = std::time::Instant::now();
        let mut cur = TurnCursor::default();
        let t = turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, MAX_BYTES);
        assert_eq!(t.len(), 10_000);
        assert_eq!(t.iter().map(|t| t.output).sum::<u64>(), 10_000);
        assert!(turns_in(&home, AgentKind::Claude, SID, window, &mut cur, true, MAX_BYTES).is_empty());
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cursor_json_round_trip_with_windows_non_ascii_paths_resumes_exactly_once() {
        let folder = Path::new(r"C:\Users\Jörg Müller\my app");
        let (home, _) = claude_home("t-cursor-jörg müller", folder, &two_turns());
        let mut cur = TurnCursor::default();
        assert_eq!(run(&home, SID, &mut cur, false).len(), 1);
        let mut cur: TurnCursor = serde_json::from_str(&serde_json::to_string(&cur).unwrap()).unwrap();
        let t = run(&home, SID, &mut cur, true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].prompt.as_deref(), Some("second prompt"));
        assert!(run(&home, SID, &mut cur, true).is_empty());
        // a stale entry for a file that is gone is harmless
        let mut stale: TurnCursor =
            serde_json::from_str(r#"{"offsets":{"C:\\Users\\Jörg Müller\\gone.jsonl":42}}"#).unwrap();
        assert_eq!(stale.offsets.get(Path::new(r"C:\Users\Jörg Müller\gone.jsonl")), Some(&42));
        let empty = tmp("t-cursor-empty");
        assert!(run(&empty, SID, &mut stale, true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&empty);
    }
}
