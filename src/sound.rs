//! Attention sounds, as HyperOx plays them: one when an agent needs you, one
//! when it finished while you weren't looking. Bundled sounds (HyperOx's), or a
//! file from settings.json; played with whichever MP3-capable player exists
//! (Windows plays them itself).

use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Stdio;
#[cfg(not(windows))]
use std::time::{Duration, Instant};

const DONE: &[u8] = include_bytes!("../assets/sounds/done.mp3");
const BLOCKED: &[u8] = include_bytes!("../assets/sounds/request.mp3");

/// Players that decode MP3, tried in order (never bare `aplay`: it plays MP3 bytes as noise).
#[cfg(not(windows))]
const PLAYERS: &[(&str, &[&str])] = &[
    #[cfg(target_os = "macos")]
    ("afplay", &[]),
    ("paplay", &[]),
    ("pw-play", &[]),
    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ("mpg123", &["-q"]),
    ("mpv", &["--no-video", "--really-quiet"]),
];
#[cfg(not(windows))]
const PLAYER_LIMIT: Duration = Duration::from_secs(15);

/// `done`, `blocked`, or a file (absolute, or relative to ~/.config/h0x-ade). Off the UI thread.
pub fn play(sound: &str) {
    if std::env::var_os("H0X_ADE_NO_SOUND").is_some() {
        return;
    }
    let sound = sound.to_owned();
    let attention = matches!(sound.as_str(), "done" | "blocked");
    // Settings are local to the UI thread; resolve them before handing playback off.
    let chosen = if attention { custom(&sound) } else { Some(resolve(&sound)) };
    std::thread::spawn(move || {
        if let Some(path) = chosen {
            match play_file(&path) {
                Ok(()) => return,
                Err(e) => tracing::warn!("sound {}: {e}", path.display()),
            }
        }
        // A custom attention sound that can't play (moved, other OS) falls back to the bundled one.
        if attention && let Err(e) = bundled(&sound).and_then(|p| play_file(&p)) {
            tracing::warn!("sound {sound}: {e}");
        }
    });
}

fn resolve(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() { p } else { crate::mods::app_dir().join(p) }
}

/// The file settings.json names for `done` / `blocked`, if any.
fn custom(sound: &str) -> Option<PathBuf> {
    let s = crate::settings::get();
    let path = if sound == "done" { &s.sounds.done } else { &s.sounds.blocked };
    path.as_deref().filter(|p| !p.is_empty()).map(resolve)
}

/// The bundled sound, written once to a private temp file players can open.
fn bundled(sound: &str) -> Result<PathBuf, String> {
    static DIR: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    let (name, bytes) = if sound == "done" { ("done", DONE) } else { ("blocked", BLOCKED) };
    let dir = DIR.get_or_init(|| private_dir(&std::env::temp_dir())).as_ref().map_err(Clone::clone)?;
    write_new(dir, name, bytes)
}

/// A fresh directory only this user can enter. `create_dir` fails on anything that
/// already exists (a planted symlink included), so the name need not be secret.
fn private_dir(base: &Path) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = base.join(format!("h0x-ade-{}-{nanos}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// `NAME.mp3` in `dir`, created exclusively (an existing file or link is an error, never followed).
fn write_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    let path = dir.join(format!("{name}.mp3"));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => f.write_all(bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?,
        // Written by an earlier call this run.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && std::fs::read(&path).is_ok_and(|b| b == bytes) => {}
        Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_dir_is_fresh_and_files_are_exclusive() {
        let base = std::env::temp_dir().join(format!("h0x-sound-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = private_dir(&base).unwrap();
        assert!(dir.starts_with(&base) && dir.is_dir());
        let path = write_new(&dir, "done", b"abc").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        // Same content again is fine; different content (substituted file) is refused.
        assert_eq!(write_new(&dir, "done", b"abc").unwrap(), path);
        assert!(write_new(&dir, "done", b"xyz").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        std::fs::remove_dir_all(&base).unwrap();
    }
}

/// Windows plays MP3 itself (MCI): no player to find.
#[cfg(windows)]
fn play_file(path: &Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Media::Multimedia::mciSendStringW;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // Its own alias, so two sounds can overlap.
    let alias = format!("h0x{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let mci = |cmd: String| {
        let wide: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
        match unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) } {
            0 => Ok(()),
            code => Err(format!("MCI error {code}")),
        }
    };
    mci(format!("open \"{}\" type mpegvideo alias {alias}", path.display()))?;
    let played = mci(format!("play {alias} wait"));
    let _ = mci(format!("close {alias}"));
    played
}

#[cfg(not(windows))]
fn play_file(path: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for (program, args) in PLAYERS {
        match run(program, args, path) {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("{program}: {e}")),
        }
    }
    Err(format!("no MP3 player worked ({})", errors.join("; ")))
}

#[cfg(not(windows))]
fn run(program: &str, args: &[&str], path: &Path) -> Result<(), String> {
    let mut child = crate::spawn::command(program)
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("exited with {status}")),
            Ok(None) if start.elapsed() > PLAYER_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(e.to_string()),
        }
    }
}
