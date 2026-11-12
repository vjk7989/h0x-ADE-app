//! Usage analytics and failure reports, sent to PostHog (docs/security.md, "What HyperOx sends").
//!
//! Only builds made with `H0X_ADE_POSTHOG_TOKEN` (the release workflow) send anything; every
//! other build is a no-op. Events wait in the app home (`analytics/queue.jsonl` for usage,
//! `reports/*.json` for failures, one file each) and go out every minute and when the app quits.
//! Usage events stop when `share_analytics` is off (what is queued is still sent); failure
//! reports always go. Every event is scrubbed of paths and long text first.

use serde_json::{Value, json};
mod crash;
pub use crash::{device_details, install_crash_hook, report};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TOKEN: Option<&str> = option_env!("H0X_ADE_POSTHOG_TOKEN");
const HOST: Option<&str> = option_env!("H0X_ADE_POSTHOG_HOST");
const DEFAULT_HOST: &str = "https://eu.i.posthog.com";
/// The usage queue's size limit: the oldest events are dropped beyond it.
const QUEUE_MAX: u64 = 1_000_000;
/// Events per request.
const BATCH: usize = 100;
const STRING_MAX: usize = 500;
const TICK: Duration = Duration::from_secs(60);

struct Analytics {
    dir: PathBuf,
    token: Option<String>,
    host: String,
    id: String,
}

static GLOBAL: OnceLock<Analytics> = OnceLock::new();
static SHARING: AtomicBool = AtomicBool::new(false);
static STARTED: OnceLock<Instant> = OnceLock::new();
/// Serializes the queue file and the reports folder inside this process.
static FILES: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Starts the once-a-minute flusher (a no-op in builds without a token). `share` is the
/// `share_analytics` setting; the flusher re-reads it every minute.
pub fn start(share: bool) {
    let _ = STARTED.set(Instant::now());
    let Some(token) = TOKEN.filter(|t| !t.is_empty()) else {
        return;
    };
    let dir = crate::mods::app_dir();
    let id = crate::settings::install_id(&dir);
    let host = HOST
        .filter(|h| !h.is_empty())
        .unwrap_or(DEFAULT_HOST)
        .trim_end_matches('/')
        .to_string();
    if GLOBAL
        .set(Analytics {
            dir,
            token: Some(token.to_string()),
            host,
            id,
        })
        .is_err()
    {
        return;
    }
    set_sharing(share);
    let spawned = std::thread::Builder::new()
        .name("analytics".into())
        .spawn(|| {
            loop {
                std::thread::sleep(TICK);
                if let Ok(person) = crate::settings::load_person() {
                    set_sharing(person.share_analytics);
                }
                flush();
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("analytics flusher not started: {e}");
    }
}

/// Whether usage events are kept from now on. Failure reports are not governed by this.
pub fn set_sharing(on: bool) {
    SHARING.store(on, Ordering::Relaxed);
}

/// Seconds since `start`.
pub fn session_seconds() -> u64 {
    STARTED.get().map_or(0, |t| t.elapsed().as_secs())
}

/// A usage event, dropped when sharing is off. `props` must hold no free text from a terminal.
pub fn event(name: &str, props: Value) {
    if SHARING.load(Ordering::Relaxed)
        && let Some(a) = GLOBAL.get()
    {
        a.record(false, name, props);
    }
}

/// A failure report: kept whatever the sharing setting says. `kind` is a short fixed word.
pub fn failure(kind: &str, props: Value) {
    if let Some(a) = GLOBAL.get() {
        a.record(true, kind, props);
    }
}

/// Sends what is queued now (the app quitting: short timeout, one try).
pub fn flush() {
    if let Some(a) = GLOBAL.get() {
        a.flush(&post);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `2026-11-13T09:30:00.123Z` for Unix milliseconds.
fn iso8601(ms: u64) -> String {
    let (secs, milli) = (ms / 1000, ms % 1000);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// True if `word` holds something that looks like an absolute path (a drive path, a UNC or
/// `\?\` path, or a Unix path with two or more segments).
fn path_like(word: &str) -> bool {
    word.split(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';' | '='))
        .any(|t| {
            let b = t.as_bytes();
            let drive = b.len() >= 3
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && matches!(b[2], b'\\' | b'/');
            let unix = t.starts_with('/') && t[1..].contains('/');
            drive || unix || t.starts_with("\\\\")
        })
}

/// `s` with every `pat` replaced by `~` (ASCII case-insensitive on Windows).
fn replace_home(s: &str, pat: &str) -> String {
    if !cfg!(windows) {
        return s.replace(pat, "~");
    }
    let (lower, lpat) = (s.to_ascii_lowercase(), pat.to_ascii_lowercase());
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (i, _) in lower.match_indices(&lpat) {
        out.push_str(&s[last..i]);
        out.push('~');
        last = i + pat.len();
    }
    out.push_str(&s[last..]);
    out
}

/// One line of `s` with the traces of this machine removed: the home folder (either separator
/// form) becomes `~`, every path-like word becomes `[path]`, and the result is cut to 500 characters.
fn scrub_text(s: &str, home: &str) -> String {
    let mut text = s.to_string();
    if !home.is_empty() && home != "/" {
        let backslashed = home.replace('/', "\\");
        let slashed = home.replace('\\', "/");
        text = replace_home(&text, &backslashed);
        if slashed != backslashed {
            text = replace_home(&text, &slashed);
        }
    }
    let line = text
        .split_whitespace()
        .map(|w| if path_like(w) { "[path]" } else { w })
        .collect::<Vec<_>>()
        .join(" ");
    line.chars().take(STRING_MAX).collect()
}

/// `v` with every string (and object key) passed through `scrub_text`.
fn scrub(v: Value, home: &str) -> Value {
    match v {
        Value::String(s) => Value::String(scrub_text(&s, home)),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| scrub(x, home)).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, x)| (scrub_text(&k, home), scrub(x, home)))
                .collect(),
        ),
        other => other,
    }
}

impl Analytics {
    fn queue(&self) -> PathBuf {
        self.dir.join("analytics").join("queue.jsonl")
    }

    fn reports(&self) -> PathBuf {
        self.dir.join("reports")
    }

    /// Queues one scrubbed event. Nothing is written without a token.
    fn record(&self, failure: bool, name: &str, props: Value) {
        if self.token.is_none() {
            return;
        }
        let home = crate::mods::home().to_string_lossy().into_owned();
        let mut props = match scrub(props, &home) {
            Value::Object(o) => o,
            _ => Default::default(),
        };
        props.insert("app_version".into(), env!("CARGO_PKG_VERSION").into());
        props.insert("os".into(), std::env::consts::OS.into());
        props.insert("arch".into(), std::env::consts::ARCH.into());
        let ms = now_ms();
        let name = if failure {
            format!("failure_{name}")
        } else {
            name.to_string()
        };
        let event = json!({ "event": name, "distinct_id": self.id, "timestamp": iso8601(ms), "properties": props });
        let _guard = FILES.lock().unwrap_or_else(|e| e.into_inner());
        let wrote = if failure {
            let file = self.reports().join(format!(
                "{ms}-{}-{}.json",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(self.reports()).and_then(|()| crate::persist::write_atomic(&file, event.to_string()))
        } else {
            self.append(&event.to_string())
        };
        if let Err(e) = wrote {
            tracing::warn!("analytics event not kept: {e}");
        }
    }

    /// Adds a line to the usage queue; past 1 MB the oldest lines go.
    fn append(&self, line: &str) -> std::io::Result<()> {
        let file = self.queue();
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?;
        writeln!(f, "{line}")?;
        drop(f);
        if std::fs::metadata(&file)?.len() > QUEUE_MAX {
            let text = std::fs::read_to_string(&file)?;
            let mut size = text.len() as u64;
            let kept: Vec<&str> = text
                .lines()
                .skip_while(|l| {
                    let over = size > QUEUE_MAX;
                    size -= (l.len() + 1) as u64;
                    over
                })
                .collect();
            std::fs::write(
                &file,
                kept.iter().map(|l| format!("{l}\n")).collect::<String>(),
            )?;
        }
        Ok(())
    }

    /// Sends every queued event through `send(url, body)`: failure reports first, then usage.
    /// What was sent (or refused by the server) is deleted; on any other error the rest stays
    /// for the next time.
    fn flush(&self, send: &dyn Fn(&str, &str) -> Result<(), String>) {
        let Some(token) = &self.token else { return };
        let url = format!("{}/batch/", self.host);
        let body = |events: &[Value]| json!({ "api_key": token, "batch": events }).to_string();

        let guard = FILES.lock().unwrap_or_else(|e| e.into_inner());
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.reports())
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let queued = std::fs::read_to_string(self.queue()).unwrap_or_default();
        let lines: Vec<&str> = queued.lines().filter(|l| !l.is_empty()).collect();
        drop(guard);

        for chunk in files.chunks(BATCH) {
            // A file that is not an event (damaged by a crash) is deleted, not retried for ever.
            let events: Vec<Value> = chunk
                .iter()
                .filter_map(|p| std::fs::read_to_string(p).ok())
                .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
                .filter(Value::is_object)
                .collect();
            if !events.is_empty() && !deliver(send, &url, &body(&events)) {
                return;
            }
            let _guard = FILES.lock().unwrap_or_else(|e| e.into_inner());
            for p in chunk {
                let _ = std::fs::remove_file(p);
            }
        }
        let mut sent = 0;
        for chunk in lines.chunks(BATCH) {
            let events: Vec<Value> = chunk
                .iter()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            if !events.is_empty() && !deliver(send, &url, &body(&events)) {
                break;
            }
            sent += chunk.len();
        }
        if sent > 0 {
            // Events added while sending are after the sent lines; the cap may have dropped some of those.
            let _guard = FILES.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::fs::read_to_string(self.queue()).unwrap_or_default();
            let rest: String = now
                .lines()
                .filter(|l| !l.is_empty())
                .skip(sent)
                .map(|l| format!("{l}\n"))
                .collect();
            let _ = std::fs::write(self.queue(), rest);
        }
    }
}

/// The start of the error `post` returns when the server answered with an HTTP 4xx.
const REFUSED: &str = "refused: ";

/// Sends one batch. True when it is done with: sent, or refused by the server (dropped, since
/// sending it again would be refused again). False when it should be tried again later.
fn deliver(send: &dyn Fn(&str, &str) -> Result<(), String>, url: &str, body: &str) -> bool {
    match send(url, body) {
        Ok(()) => true,
        Err(e) if e.starts_with(REFUSED) => {
            tracing::warn!("analytics batch dropped: {e}");
            true
        }
        Err(_) => false,
    }
}

/// POSTs `body` to `url` with curl: https only, no shell, a short timeout, the body on stdin.
fn post(url: &str, body: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("not https: {url}"));
    }
    let mut curl = crate::spawn::command(crate::update::system("curl"));
    curl.args([
        "-sS",
        "-o",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
        "-w",
        "%{http_code}",
        "--max-time",
        "5",
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
    ])
    .args(crate::update::CURL_HTTPS)
    .arg(url);
    let mut child = curl
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match code.chars().next() {
        Some('2') => Ok(()),
        // 408 and 429 mean "later", not "never": those are tried again.
        Some('4') if code != "408" && code != "429" => Err(format!("{REFUSED}HTTP {code}")),
        _ => Err(format!("HTTP {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn home_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h0x-analytics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make(dir: &Path, token: Option<&str>) -> Analytics {
        Analytics {
            dir: dir.to_path_buf(),
            token: token.map(Into::into),
            host: "https://example.test".into(),
            id: "ab".repeat(16),
        }
    }

    fn sent_batches(a: &Analytics, ok: bool) -> Vec<Value> {
        let seen = RefCell::new(Vec::new());
        a.flush(&|url, body| {
            seen.borrow_mut()
                .push(json!({ "url": url, "body": serde_json::from_str::<Value>(body).unwrap() }));
            if ok { Ok(()) } else { Err("offline".into()) }
        });
        seen.into_inner()
    }

    #[test]
    fn no_token_writes_and_sends_nothing() {
        let dir = home_dir("notoken");
        let a = make(&dir, None);
        a.record(false, "x", json!({}));
        a.record(true, "y", json!({}));
        assert!(!dir.join("analytics").exists() && !dir.join("reports").exists());
        assert!(sent_batches(&a, true).is_empty());
    }

    #[test]
    fn batch_body_has_key_events_and_the_install_id() {
        let dir = home_dir("shape");
        let a = make(&dir, Some("phc_test"));
        a.record(false, "app_started", json!({ "n": 1 }));
        let sent = sent_batches(&a, true);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["url"], "https://example.test/batch/");
        let body = &sent[0]["body"];
        assert_eq!(body["api_key"], "phc_test");
        let e = &body["batch"][0];
        assert_eq!(
            (&e["event"], &e["distinct_id"]),
            (&json!("app_started"), &json!("ab".repeat(16)))
        );
        assert_eq!(e["properties"]["n"], 1);
        assert_eq!(e["properties"]["app_version"], env!("CARGO_PKG_VERSION"));
        assert!(e["properties"]["os"].is_string() && e["properties"]["arch"].is_string());
        assert!(e["timestamp"].as_str().unwrap().ends_with('Z'));
        // Sent: gone. A second flush has nothing.
        assert!(sent_batches(&a, true).is_empty());
    }

    #[test]
    fn a_failed_send_keeps_everything_for_the_next_try() {
        let dir = home_dir("retry");
        let a = make(&dir, Some("t"));
        a.record(false, "one", json!({}));
        a.record(true, "bad", json!({}));
        assert_eq!(sent_batches(&a, false).len(), 1);
        let sent = sent_batches(&a, true);
        let names: Vec<_> = sent
            .iter()
            .flat_map(|s| {
                s["body"]["batch"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["event"].as_str().unwrap().to_string())
            })
            .collect();
        assert_eq!(names, ["failure_bad", "one"]);
        assert!(sent_batches(&a, true).is_empty());
    }

    #[test]
    fn failure_reports_survive_a_restart() {
        let dir = home_dir("restart");
        make(&dir, Some("t")).record(true, "crash", json!({ "where": "pty" }));
        assert_eq!(std::fs::read_dir(dir.join("reports")).unwrap().count(), 1);
        let sent = sent_batches(&make(&dir, Some("t")), true);
        assert_eq!(sent[0]["body"]["batch"][0]["event"], "failure_crash");
        assert_eq!(std::fs::read_dir(dir.join("reports")).unwrap().count(), 0);
    }

    #[test]
    fn a_damaged_report_is_deleted_not_retried() {
        let dir = home_dir("damaged");
        let a = make(&dir, Some("t"));
        std::fs::create_dir_all(a.reports()).unwrap();
        std::fs::write(a.reports().join("1.json"), "{ not json").unwrap();
        assert!(sent_batches(&a, true).is_empty());
        assert!(!a.reports().join("1.json").exists());
    }

    #[test]
    fn the_usage_queue_is_capped_dropping_the_oldest() {
        let dir = home_dir("cap");
        let a = make(&dir, Some("t"));
        let line = format!("{{\"n\":\"{}\"}}", "x".repeat(990));
        for i in 0..1100 {
            a.append(&line.replace('n', &format!("n{i}"))).unwrap();
        }
        let text = std::fs::read_to_string(a.queue()).unwrap();
        assert!(text.len() as u64 <= QUEUE_MAX, "{}", text.len());
        assert!(text.lines().last().unwrap().contains("\"n1099\""));
        assert!(!text.contains("\"n0\""));
        assert!(text.lines().count() > 900);
    }

    #[test]
    fn scrub_redacts_paths_and_caps_strings() {
        let home = "C:\\Users\\someone";
        let v = scrub(
            json!({
                "a": "C:\\Users\\someone\\repo",
                "b": "opened /home/x/project now",
                "c": "D:/work/x",
                "d": "\\\\server\\share",
                "e": "fine and/or ok https://github.com/x",
                "f": "y".repeat(900),
                "g": ["/usr/bin/ls", 3, true, null],
                "h": { "i": "c:/users/someone/x" },
            }),
            home,
        );
        // The home folder becomes `~`; other path words become `[path]`; the rest stays.
        assert_eq!(
            (&v["a"], &v["b"], &v["c"], &v["d"]),
            (
                &json!("~\\repo"),
                &json!("opened [path] now"),
                &json!("[path]"),
                &json!("[path]")
            )
        );
        assert_eq!(v["e"], "fine and/or ok https://github.com/x");
        assert_eq!(v["f"].as_str().unwrap().chars().count(), 500);
        assert_eq!(v["g"], json!(["[path]", 3, true, null]));
        // The home is matched without regard to case on Windows only.
        assert_eq!(v["h"]["i"], if cfg!(windows) { "~/x" } else { "[path]" });
        assert_eq!(scrub(json!("/tmp"), ""), json!("/tmp"));
        // Object keys are scrubbed too; a lone "/" home is not replaced.
        assert_eq!(
            scrub(json!({ "/a/b/c": 1 }), "/"),
            json!({ "[path]": 1 })
        );
        assert_eq!(scrub(json!("go /x/y"), "/"), json!("go [path]"));
    }

    #[test]
    fn recorded_events_are_scrubbed_and_non_object_props_become_empty() {
        let dir = home_dir("scrubbed");
        let a = make(&dir, Some("t"));
        a.record(false, "e", json!({ "cwd": "/home/someone/code/x" }));
        a.record(false, "f", json!("loose"));
        let sent = sent_batches(&a, true);
        let batch = sent[0]["body"]["batch"].as_array().unwrap();
        assert_eq!(batch[0]["properties"]["cwd"], "[path]");
        assert!(
            batch[1]["properties"].get("loose").is_none()
                && batch[1]["properties"]["os"].is_string()
        );
    }

    #[test]
    fn off_switch_drops_usage_events_but_keeps_failures() {
        // The module-level gate: nothing is recorded while sharing is off or before start.
        set_sharing(false);
        event("x", json!({}));
        failure("y", json!({}));
        assert!(GLOBAL.get().is_none() || !SHARING.load(Ordering::Relaxed));
        // What is already queued is still sent after the switch goes off.
        let dir = home_dir("off");
        let a = make(&dir, Some("t"));
        a.record(false, "queued_before", json!({}));
        set_sharing(false);
        let sent = sent_batches(&a, true);
        assert_eq!(sent[0]["body"]["batch"][0]["event"], "queued_before");
    }

    #[test]
    fn timestamps_are_iso8601() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_709_164_800_123), "2024-02-29T00:00:00.123Z");
        assert_eq!(iso8601(1_794_563_400_000), "2026-11-13T09:50:00.000Z");
    }

    #[test]
    fn post_refuses_plain_http() {
        assert!(post("http://example.test/batch/", "{}").is_err());
    }

    fn event_names(batch: &Value) -> Vec<String> {
        batch["body"]["batch"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["event"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn reports_go_first_and_everything_goes_in_batches_of_100() {
        let dir = home_dir("batches");
        let a = make(&dir, Some("t"));
        for i in 0..150 {
            a.record(true, &format!("r{i}"), json!({}));
            a.record(false, &format!("u{i}"), json!({}));
        }
        let sent = sent_batches(&a, true);
        let names: Vec<Vec<String>> = sent.iter().map(event_names).collect();
        assert_eq!(
            names.iter().map(Vec::len).collect::<Vec<_>>(),
            [100, 50, 100, 50]
        );
        assert!(
            names[..2]
                .iter()
                .flatten()
                .all(|n| n.starts_with("failure_r"))
        );
        assert!(names[2..].iter().flatten().all(|n| n.starts_with('u')));
        assert_eq!(
            (names[2][0].as_str(), names[3][49].as_str()),
            ("u0", "u149")
        );
        assert_eq!(std::fs::read_dir(a.reports()).unwrap().count(), 0);
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "");
    }

    #[test]
    fn a_send_failing_midway_keeps_only_the_unsent_usage_events() {
        let dir = home_dir("midway");
        let a = make(&dir, Some("t"));
        for i in 0..150 {
            a.record(false, &format!("u{i}"), json!({}));
        }
        let calls = std::cell::Cell::new(0);
        a.flush(&|_, _| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err("offline".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(calls.get(), 2);
        let left = std::fs::read_to_string(a.queue()).unwrap();
        assert_eq!(left.lines().count(), 50);
        assert!(left.lines().next().unwrap().contains("\"u100\""));
        let sent = sent_batches(&a, true);
        assert_eq!(sent.len(), 1);
        assert_eq!(event_names(&sent[0]).last().unwrap(), "u149");
    }

    #[test]
    fn an_empty_or_unparsable_queue_sends_nothing_and_its_junk_is_cleared() {
        let dir = home_dir("junk");
        let a = make(&dir, Some("t"));
        std::fs::create_dir_all(a.queue().parent().unwrap()).unwrap();
        // A 0-byte queue file.
        std::fs::write(a.queue(), "").unwrap();
        assert!(sent_batches(&a, true).is_empty());
        // Only lines that are not JSON: nothing is sent and they do not stay.
        std::fs::write(a.queue(), "not json\n\n{\"event\":\n").unwrap();
        assert!(sent_batches(&a, true).is_empty());
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "");
        // A line that is not JSON between two events: the events go, the junk goes with them.
        a.record(false, "first", json!({}));
        a.append("garbage {").unwrap();
        a.record(false, "second", json!({}));
        let sent = sent_batches(&a, true);
        assert_eq!(sent.len(), 1);
        assert_eq!(event_names(&sent[0]), ["first", "second"]);
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "");
    }

    #[test]
    fn the_cap_keeps_a_queue_of_exactly_1_mb_and_never_keeps_an_oversized_event() {
        let dir = home_dir("capedge");
        let a = make(&dir, Some("t"));
        // One line plus its newline is exactly the limit: kept.
        a.append(&"x".repeat(QUEUE_MAX as usize - 1)).unwrap();
        assert_eq!(std::fs::metadata(a.queue()).unwrap().len(), QUEUE_MAX);
        // One byte over: the oldest line goes.
        a.append("y").unwrap();
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "y\n");
        // A single event larger than the limit cannot be kept at all.
        a.append(&"z".repeat(QUEUE_MAX as usize)).unwrap();
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "");
    }

    #[test]
    fn concurrent_failure_reports_land_in_distinct_files() {
        let dir = home_dir("concurrent");
        let a = make(&dir, Some("t"));
        std::thread::scope(|s| {
            for t in 0..8 {
                let a = &a;
                s.spawn(move || {
                    for i in 0..5 {
                        a.record(true, &format!("t{t}_{i}"), json!({}));
                    }
                });
            }
        });
        assert_eq!(std::fs::read_dir(a.reports()).unwrap().count(), 40);
        let names: std::collections::HashSet<String> = sent_batches(&a, true)
            .iter()
            .flat_map(event_names)
            .collect();
        assert_eq!(names.len(), 40);
    }

    #[test]
    fn scrub_handles_windows_homes_with_spaces_and_non_ascii() {
        let home = "C:\\Users\\Jörg Müller";
        let v = scrub(
            json!({
                "a": "saved in C:\\Users\\Jörg Müller\\notes",
                "b": "C:/Users/Jörg Müller/x",
                "c": "C:\\Program Files\\HyperOx\\h0x-ade.exe",
                "d": "/é/ü",
                "e": "é".repeat(500),
                "f": "é".repeat(501),
            }),
            home,
        );
        assert_eq!(v["a"], "saved in ~\\notes");
        assert_eq!(v["b"], "~/x");
        // Only the word with the drive letter is a path word; no part of the home stays.
        assert_eq!(v["c"], "[path] Files\\HyperOx\\h0x-ade.exe");
        assert_eq!(v["d"], "[path]");
        assert!(!v.to_string().contains("Müller"));
        assert_eq!(v["e"], "é".repeat(500));
        assert_eq!(v["f"], "é".repeat(500));
    }

    #[test]
    fn scrub_with_an_empty_home_redacts_only_absolute_paths() {
        for kept in ["", "plain text", "/", "/tmp", "C:", "a/b/c", "x:y"] {
            assert_eq!(scrub(json!(kept), ""), json!(kept), "{kept}");
        }
        for path in ["C:\\", "c:/", "/a/b", "\\\\?\\C:\\x", "f(/a/b)"] {
            assert_eq!(scrub(json!(path), ""), json!("[path]"), "{path}");
        }
        assert_eq!(scrub(json!(null), ""), json!(null));
        assert_eq!(scrub(json!({}), ""), json!({}));
        assert_eq!(scrub(json!([]), ""), json!([]));
    }

    #[test]
    fn iso8601_handles_leap_days_and_century_boundaries() {
        assert_eq!(iso8601(946_684_799_000), "1999-12-31T23:59:59.000Z");
        assert_eq!(iso8601(951_868_799_999), "2000-02-29T23:59:59.999Z");
        assert_eq!(iso8601(4_107_542_399_999), "2100-02-28T23:59:59.999Z");
        assert_eq!(iso8601(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
        assert!(iso8601(u64::MAX).ends_with('Z'));
    }

    #[test]
    fn a_batch_the_server_refuses_is_dropped_but_other_errors_are_retried() {
        let dir = home_dir("refused");
        let a = make(&dir, Some("t"));
        a.record(false, "one", json!({}));
        a.flush(&|_, _| Err(format!("{REFUSED}HTTP 400")));
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap(), "");
        a.record(false, "two", json!({}));
        a.flush(&|_, _| Err("HTTP 503".into()));
        assert_eq!(std::fs::read_to_string(a.queue()).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_report_that_is_not_an_object_is_deleted_not_sent() {
        let dir = home_dir("notobject");
        let a = make(&dir, Some("t"));
        std::fs::create_dir_all(a.reports()).unwrap();
        std::fs::write(a.reports().join("1.json"), "[1]").unwrap();
        assert!(sent_batches(&a, true).is_empty());
        assert!(!a.reports().join("1.json").exists());
    }

    #[test]
    fn post_refuses_anything_but_https_without_running_curl() {
        for url in [
            "",
            "ftp://example.test/",
            "file:///etc/passwd",
            " https://example.test/",
            "HTTP://example.test/",
        ] {
            assert!(
                post(url, "{}").unwrap_err().starts_with("not https"),
                "{url}"
            );
        }
    }
}
