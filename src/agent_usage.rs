//! Token totals of an agent session (Claude Code / Codex) read from the
//! agent's own on-disk session logs. Only token counters, model names, ids and
//! timestamps are deserialized; prompt and reply text is never kept.

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
}
