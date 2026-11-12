//! Crash reports and the device details of failure reports. A panic writes its report
//! synchronously (no lock, no network); the flusher sends it at the next start.

use super::{GLOBAL, STRING_MAX, now_ms, path_like, session_seconds};
use serde_json::{Value, json};
use std::io::Write;

const FRAMES: usize = 12;

/// Installs the panic hook: a crash report when analytics runs in this build, then the previous
/// hook. Call before anything that can panic.
pub fn install_crash_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_crash(info);
        previous(info);
    }));
}

/// Takes care not to panic: a panic inside the hook aborts the process.
fn write_crash(info: &std::panic::PanicHookInfo<'_>) {
    let Some(a) = GLOBAL.get() else { return };
    if a.token.is_none() {
        return;
    }
    let home = crate::mods::home().to_string_lossy().into_owned();
    let message = match info.payload().downcast_ref::<&str>() {
        Some(s) => (*s).to_string(),
        None => info.payload().downcast_ref::<String>().cloned().unwrap_or_default(),
    };
    let location = info
        .location()
        .map(|l| format!("{}:{}", file_name(l.file()), l.line()))
        .unwrap_or_default();
    let backtrace = frames(&std::backtrace::Backtrace::force_capture().to_string());
    let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
    let mut props = serde_json::Map::new();
    props.insert("message".into(), scrub_text(&message, &home).into());
    props.insert("location".into(), location.into());
    props.insert("backtrace".into(), backtrace.into());
    props.insert("thread".into(), thread.into());
    props.insert("uptime_seconds".into(), session_seconds().into());
    props.insert("device".into(), device_details());
    props.insert("app_version".into(), env!("CARGO_PKG_VERSION").into());
    props.insert("os".into(), std::env::consts::OS.into());
    props.insert("arch".into(), std::env::consts::ARCH.into());
    props.insert("$geoip_disable".into(), true.into());
    let ms = now_ms();
    let event = json!({ "event": "failure_crash", "distinct_id": a.id, "timestamp": super::iso8601(ms), "properties": props });
    let dir = a.reports();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join(format!("{ms}-{}-crash.json", std::process::id()));
    if let Ok(mut f) = std::fs::File::create(file) {
        let _ = f.write_all(event.to_string().as_bytes());
    }
}

/// A failure report of `kind` (a short fixed word) for the failed `step`, with the error text
/// and the device details; `failure` scrubs the text.
pub fn report(kind: &str, step: &str, error: &str) {
    super::failure(kind, json!({ "step": step, "error": error, "device": device_details() }));
}

/// The last component of a file path, either separator.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Free text with its path-like words (and anything holding `home`) replaced by `[path]`, on
/// one line and cut to 500 characters.
fn scrub_text(s: &str, home: &str) -> String {
    let slashed = home.replace('\\', "/");
    let text = s
        .split_whitespace()
        .map(|w| {
            let home_hit = !home.is_empty() && (w.contains(home) || w.contains(&slashed));
            if home_hit || path_like(w) { "[path]" } else { w }
        })
        .collect::<Vec<_>>()
        .join(" ");
    text.chars().take(STRING_MAX).collect()
}

/// The function names of a printed backtrace: its `at file:line` lines go, so do hashes and the
/// frames of the panic machinery. At most 12 are kept.
fn frames(printed: &str) -> Vec<String> {
    printed
        .lines()
        .filter_map(|l| {
            let (index, name) = l.trim().split_once(": ")?;
            if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let name = match name.trim().split_once(" - ") {
                Some((addr, n)) if addr.starts_with("0x") => n,
                _ => name.trim(),
            };
            let name = match name.rsplit_once("::h") {
                Some((head, hash)) if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => head,
                _ => name,
            };
            Some(name.to_string())
        })
        .filter(|n| {
            !["backtrace::", "panicking::", "panic_fmt", "analytics::crash", "rust_begin_unwind"]
                .iter()
                .any(|skip| n.contains(skip))
        })
        .take(FRAMES)
        .collect()
}

/// What the machine is, for failure reports: only fields that are cheap to read with the
/// standard library; the others are left out (never an empty or invented value). Not here:
/// GPU, screens and scale (gpui, at report time), install type (the app has no "default folder").
pub fn device_details() -> Value {
    let mut d = serde_json::Map::new();
    d.insert("os".into(), std::env::consts::OS.into());
    d.insert("arch".into(), std::env::consts::ARCH.into());
    d.insert("uptime_seconds".into(), session_seconds().into());
    if let Ok(n) = std::thread::available_parallelism() {
        d.insert("cpu_cores".into(), n.get().into());
    }
    let mut put = |key: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            d.insert(key.into(), v.chars().take(STRING_MAX).collect::<String>().into());
        }
    };
    put("os_version", os_version());
    put("cpu_model", cpu_model());
    put("language", language());
    put("time_zone", time_zone());
    if let Some(mb) = ram_mb() {
        d.insert("ram_mb".into(), mb.into());
    }
    Value::Object(d)
}

#[cfg(target_os = "linux")]
fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The value of `key` (before the colon or equals sign) in a `key: value` or `key=value` file.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once([':', '='])?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn os_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    return field(&read("/etc/os-release"), "PRETTY_NAME");
    #[cfg(not(target_os = "linux"))]
    None
}

fn cpu_model() -> Option<String> {
    #[cfg(target_os = "linux")]
    return field(&read("/proc/cpuinfo"), "model name");
    #[cfg(windows)]
    return std::env::var("PROCESSOR_IDENTIFIER").ok();
    #[cfg(not(any(target_os = "linux", windows)))]
    None
}

fn ram_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    return field(&read("/proc/meminfo"), "MemTotal")?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kb| kb / 1024);
    #[cfg(not(target_os = "linux"))]
    None
}

/// `en_US` from `LC_ALL`, `LC_MESSAGES` or `LANG` (`en_US.UTF-8`).
fn language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .map(|v| v.split(['.', '@']).next().unwrap_or_default().to_string())
}

fn time_zone() -> Option<String> {
    std::env::var("TZ")
        .ok()
        .or_else(|| {
            #[cfg(target_os = "linux")]
            return Some(read("/etc/timezone"));
            #[cfg(not(target_os = "linux"))]
            None
        })
        .map(|v| v.trim().trim_start_matches(':').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_lose_their_folders_on_either_separator() {
        assert_eq!(file_name("C:\\Users\\me\\src\\main.rs"), "main.rs");
        assert_eq!(file_name("/home/me/src/main.rs"), "main.rs");
        assert_eq!(file_name("main.rs"), "main.rs");
        assert_eq!(file_name(""), "");
    }

    #[test]
    fn panic_messages_lose_paths_and_the_home_folder() {
        let m = scrub_text("cannot open C:\\Users\\me\\a.txt: gone", "C:\\Users\\me");
        assert_eq!(m, "cannot open [path] gone");
        assert_eq!(scrub_text("failed at /home/me/x/y", ""), "failed at [path]");
        assert_eq!(scrub_text("in /home/me.cfg", "/home/me"), "in [path]");
        assert_eq!(scrub_text("index out of bounds", "/home/me"), "index out of bounds");
        assert_eq!(scrub_text("", "/home/me"), "");
        assert_eq!(scrub_text("a\nb\tc", ""), "a b c");
        assert_eq!(scrub_text(&"x".repeat(900), "").chars().count(), STRING_MAX);
    }

    #[test]
    fn a_backtrace_keeps_function_names_only() {
        let printed = "   0: std::backtrace::Backtrace::force_capture\n             at /rustc/abc/library/std/src/backtrace.rs:1:1\n   1: h0x_ade::analytics::crash::write_crash\n   2: h0x_ade::update::swap::h0123456789abcdef\n             at C:\\Users\\me\\src\\update.rs:726:9\n   3: std::panicking::begin_panic\n   4:     0x7ff6e0 - h0x_ade::main\nnote: some hint";
        assert_eq!(frames(printed), vec!["h0x_ade::update::swap", "h0x_ade::main"]);
        let all = frames(&(0..30).map(|i| format!("  {i}: f{i}\n")).collect::<String>());
        assert_eq!(all.len(), FRAMES);
        assert!(frames("").is_empty());
    }

    #[test]
    fn device_details_hold_the_basics_and_never_an_empty_field() {
        let d = device_details();
        let o = d.as_object().unwrap();
        assert_eq!(o["os"], std::env::consts::OS);
        assert_eq!(o["arch"], std::env::consts::ARCH);
        assert!(o.contains_key("uptime_seconds"));
        for (k, v) in o {
            assert!(!v.is_null() && *v != "", "{k} is empty");
        }
    }

    #[test]
    fn key_value_files_parse_both_separators() {
        assert_eq!(field("model name\t: Fast CPU\nx: y", "model name").as_deref(), Some("Fast CPU"));
        assert_eq!(field("PRETTY_NAME=\"Debian 12\"", "PRETTY_NAME").as_deref(), Some("Debian 12"));
        assert_eq!(field("a: b", "c"), None);
        assert_eq!(field("", "c"), None);
    }
}
