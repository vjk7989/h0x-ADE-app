//! A private record of how much agents worked, per day, for the person's own use. It lives in
//! `<app home>/usage/activity.json` and is never sent anywhere (not even with analytics on).
//!
//! `{"version":1,"days":{"YYYY-MM-DD":{"seconds":n,"sessions":n,"tokens":n}}}`, days are local dates.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A tick counts at most this much: a sleeping computer or a stalled window adds no hours.
const TICK_CAP: Duration = Duration::from_secs(10);
const FLUSH_EVERY: Duration = Duration::from_secs(60);
const KEEP_DAYS: i64 = 371;

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
struct Day {
    seconds: u64,
    sessions: u64,
    tokens: u64,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    days: BTreeMap<String, Day>,
}

/// What has happened since the last write; each flush adds it to the file.
struct Recorder {
    pending: BTreeMap<String, Day>,
    last_tick: Option<Instant>,
    last_flush: Option<Instant>,
    /// Working time not yet a whole second.
    carry_ms: u64,
    /// The latest day seen: a clock set back never moves time to an earlier day.
    latest: Option<String>,
}

impl Recorder {
    const fn new() -> Self {
        Recorder { pending: BTreeMap::new(), last_tick: None, last_flush: None, carry_ms: 0, latest: None }
    }

    fn day(&mut self, today: &str) -> &mut Day {
        if self.latest.as_deref().is_none_or(|l| today > l) {
            self.latest = Some(today.to_owned());
        }
        let key = self.latest.clone().unwrap_or_default();
        self.pending.entry(key).or_default()
    }

    fn tick(&mut self, now: Instant, today: &str, working: bool) {
        let elapsed = self.last_tick.map_or(Duration::ZERO, |t| now.saturating_duration_since(t)).min(TICK_CAP);
        self.last_tick = Some(now);
        self.last_flush.get_or_insert(now);
        if !working {
            return;
        }
        self.carry_ms += elapsed.as_millis() as u64;
        let whole = self.carry_ms / 1000;
        self.carry_ms %= 1000;
        let day = self.day(today);
        day.seconds = day.seconds.saturating_add(whole);
    }

    fn flush_due(&self, now: Instant) -> bool {
        !self.pending.is_empty() && self.last_flush.is_some_and(|t| now.saturating_duration_since(t) >= FLUSH_EVERY)
    }
}

static RECORDER: Mutex<Recorder> = Mutex::new(Recorder::new());

fn recorder() -> std::sync::MutexGuard<'static, Recorder> {
    RECORDER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Called every tick: whether any agent is working. Writes the file about once a minute.
pub fn tick(working: bool) {
    let (now, today) = (Instant::now(), local_date());
    let due = {
        let mut r = recorder();
        r.tick(now, &today, working);
        r.flush_due(now)
    };
    if due {
        flush(&today);
    }
}

/// An agent started.
pub fn session_started() {
    let today = local_date();
    let mut r = recorder();
    let day = r.day(&today);
    day.sessions = day.sessions.saturating_add(1);
}

/// An agent that ran here ended, having used this many tokens.
pub fn add_tokens(tokens: u64) {
    let today = local_date();
    let mut r = recorder();
    let day = r.day(&today);
    day.tokens = day.tokens.saturating_add(tokens);
}

/// Writes what is pending (the app is quitting).
pub fn finish() {
    flush(&local_date());
}

fn add_to(slot: &mut Day, d: &Day) {
    slot.seconds = slot.seconds.saturating_add(d.seconds);
    slot.sessions = slot.sessions.saturating_add(d.sessions);
    slot.tokens = slot.tokens.saturating_add(d.tokens);
}

fn flush(today: &str) {
    let taken = {
        let mut r = recorder();
        r.last_flush = Some(Instant::now());
        std::mem::take(&mut r.pending)
    };
    if taken.is_empty() {
        return;
    }
    let dir = crate::mods::app_dir().join("usage");
    if let Err(e) = write_merged(&dir, &taken, today) {
        tracing::warn!("cannot save the activity record: {e}");
        // Kept for the next flush.
        let mut r = recorder();
        for (k, d) in &taken {
            add_to(r.pending.entry(k.clone()).or_default(), d);
        }
    }
}

/// Adds `delta` to what the file holds now (another HyperOx may have written meanwhile) and
/// drops the old days. A damaged file is renamed aside and a fresh one started.
fn write_merged(dir: &Path, delta: &BTreeMap<String, Day>, today: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("activity.json");
    let mut days = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<File>(&text) {
            Ok(file) if file.version == 1 => file.days,
            _ => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let _ = std::fs::rename(&path, dir.join(format!("activity.damaged-{secs}.json")));
                BTreeMap::new()
            }
        },
        Err(_) => BTreeMap::new(),
    };
    merge(&mut days, delta);
    prune(&mut days, today);
    let body = serde_json::to_string_pretty(&File { version: 1, days }).map_err(std::io::Error::other)?;
    crate::persist::write_atomic(&path, body)
}

fn merge(days: &mut BTreeMap<String, Day>, delta: &BTreeMap<String, Day>) {
    for (key, d) in delta {
        add_to(days.entry(key.clone()).or_default(), d);
    }
}

/// Drops days older than [`KEEP_DAYS`] before `today`, and keys that are not dates.
fn prune(days: &mut BTreeMap<String, Day>, today: &str) {
    let Some(now) = day_number(today) else { return };
    days.retain(|k, _| day_number(k).is_some_and(|n| n >= now - KEEP_DAYS));
}

/// Days since 1970-01-01 for `YYYY-MM-DD`.
fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let (y, m, d): (i64, i64, i64) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// `YYYY-MM-DD` of a Unix time shifted by the zone's offset.
fn date_string(utc_secs: i64, offset_secs: i64) -> String {
    let z = (utc_secs + offset_secs).div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// This computer's offset from UTC now, in seconds; 0 (UTC) when it can't be asked.
fn local_offset(utc_secs: i64) -> i64 {
    #[cfg(unix)]
    {
        let t = utc_secs as libc::time_t;
        // SAFETY: localtime_r fills the zeroed struct it is given and reads `t`.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i64 }
        }
    }
    #[cfg(windows)]
    {
        let _ = utc_secs;
        use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        // SAFETY: the call fills the zeroed struct.
        let (id, info) = unsafe {
            let mut info: TIME_ZONE_INFORMATION = std::mem::zeroed();
            (GetTimeZoneInformation(&mut info), info)
        };
        // UTC = local + bias (minutes); in daylight time the daylight bias adds to it.
        let bias = info.Bias + if id == 2 { info.DaylightBias } else { info.StandardBias };
        if id == 0xFFFF_FFFF { 0 } else { -i64::from(bias) * 60 }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = utc_secs;
        0
    }
}

fn local_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    date_string(secs, local_offset(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("h0x-activity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(dir: &Path) -> BTreeMap<String, Day> {
        serde_json::from_str::<File>(&std::fs::read_to_string(dir.join("activity.json")).unwrap()).unwrap().days
    }

    fn day(seconds: u64, sessions: u64, tokens: u64) -> Day {
        Day { seconds, sessions, tokens }
    }

    #[test]
    fn dates_are_computed_from_the_epoch_and_the_zone_offset() {
        assert_eq!(date_string(0, 0), "1970-01-01");
        assert_eq!(date_string(951_782_400, 0), "2000-02-29");
        assert_eq!(date_string(1_794_614_400, 0), "2026-11-14");
        // 23:30 UTC is already tomorrow in UTC+1 and still today in UTC-5.
        let late = 1_794_614_400 - 1800;
        assert_eq!(date_string(late, 0), "2026-11-13");
        assert_eq!(date_string(late, 3600), "2026-11-14");
        assert_eq!(date_string(late, -5 * 3600), "2026-11-13");
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2026-11-14"), Some(1_794_614_400 / 86400));
        for bad in ["", "2026", "2026-13-01", "2026-00-10", "2026-01-32", "a-b-c", "2026-01-01-01"] {
            assert_eq!(day_number(bad), None, "{bad:?}");
        }
        assert!(day_number(&local_date()).is_some());
    }

    #[test]
    fn a_tick_counts_at_most_ten_seconds_and_only_while_working() {
        let mut r = Recorder::new();
        let t0 = Instant::now();
        r.tick(t0, "2026-11-14", true);
        r.tick(t0 + Duration::from_secs(3), "2026-11-14", true);
        r.tick(t0 + Duration::from_secs(4), "2026-11-14", false);
        // A long stall (sleep) is capped.
        r.tick(t0 + Duration::from_secs(4 + 3600), "2026-11-14", true);
        assert_eq!(r.pending["2026-11-14"].seconds, 3 + 10);
        // Half seconds are carried, not lost.
        let mut r = Recorder::new();
        for i in 0..=4 {
            r.tick(t0 + Duration::from_millis(500 * i), "2026-11-14", true);
        }
        assert_eq!(r.pending["2026-11-14"].seconds, 2);
    }

    #[test]
    fn work_across_midnight_goes_to_each_day_and_a_clock_set_back_is_ignored() {
        let mut r = Recorder::new();
        let t0 = Instant::now();
        r.tick(t0, "2026-11-14", true);
        r.tick(t0 + Duration::from_secs(2), "2026-11-14", true);
        r.tick(t0 + Duration::from_secs(4), "2026-11-15", true);
        assert_eq!((r.pending["2026-11-14"].seconds, r.pending["2026-11-15"].seconds), (2, 2));
        // The clock goes back a day: it counts on the latest day seen.
        r.tick(t0 + Duration::from_secs(6), "2026-11-14", true);
        assert_eq!((r.pending["2026-11-14"].seconds, r.pending["2026-11-15"].seconds), (2, 4));
        assert_eq!(r.pending.len(), 2);
    }

    #[test]
    fn days_older_than_371_are_dropped_with_keys_that_are_not_dates() {
        let mut days: BTreeMap<String, Day> = ["2025-11-08", "2025-11-09", "2026-11-14", "junk", "2026-02-30x"].iter().map(|k| (k.to_string(), day(1, 0, 0))).collect();
        prune(&mut days, "2026-11-14");
        // 2026-11-14 minus 371 days is 2025-11-08.
        assert_eq!(days.keys().map(String::as_str).collect::<Vec<_>>(), ["2025-11-08", "2025-11-09", "2026-11-14"]);
        let mut days: BTreeMap<String, Day> = [("2025-11-07".to_string(), day(1, 0, 0)), ("2026-11-14".to_string(), day(1, 0, 0))].into();
        prune(&mut days, "2026-11-14");
        assert_eq!(days.len(), 1);
    }

    #[test]
    fn a_flush_adds_to_what_another_app_wrote_and_keeps_the_file_valid() {
        let dir = scratch("merge");
        let delta: BTreeMap<String, Day> = [("2026-11-14".to_string(), day(5, 1, 100))].into();
        write_merged(&dir, &delta, "2026-11-14").unwrap();
        // A second process wrote in between: its numbers stay, ours add.
        let other: BTreeMap<String, Day> = [("2026-11-14".to_string(), day(7, 2, 50)), ("2026-11-13".to_string(), day(9, 0, 0))].into();
        write_merged(&dir, &other, "2026-11-14").unwrap();
        write_merged(&dir, &delta, "2026-11-14").unwrap();
        let days = read(&dir);
        assert_eq!(days["2026-11-14"], day(17, 4, 250));
        assert_eq!(days["2026-11-13"], day(9, 0, 0));
        let text = std::fs::read_to_string(dir.join("activity.json")).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary files are left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_renamed_aside_and_a_fresh_one_started() {
        let dir = scratch("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        for (i, junk) in ["{not json", "", "null", r#"{"version":2,"days":{}}"#].iter().enumerate() {
            std::fs::write(dir.join("activity.json"), junk).unwrap();
            let delta: BTreeMap<String, Day> = [("2026-11-14".to_string(), day(3, 1, 0))].into();
            write_merged(&dir, &delta, "2026-11-14").unwrap();
            assert_eq!(read(&dir)["2026-11-14"], day(3, 1, 0), "{junk:?}");
            let aside = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("activity.damaged-")).count();
            assert!(aside >= 1, "round {i}: the old file is kept");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_is_written_about_once_a_minute() {
        let mut r = Recorder::new();
        let t0 = Instant::now();
        r.tick(t0, "2026-11-14", true);
        assert!(!r.flush_due(t0), "nothing pending");
        r.tick(t0 + Duration::from_secs(2), "2026-11-14", true);
        assert!(!r.flush_due(t0 + Duration::from_secs(59)));
        assert!(r.flush_due(t0 + Duration::from_secs(60)));
    }
}
