//! Live settings and the resolved layout look, including persisted app zoom.
//!
//! A theme is a palette plus optional color overrides: a built-in preset
//! (graphite, ink, moss, paper) or `themes/<name>.json`. Chrome colors are
//! derived from the palette (dark or light by its background), so every
//! theme is consistent and overrides only need what they change.

use crate::vt::Rgb;
use gpui::{Hsla, Pixels, Rgba, px};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

/// A terminal palette, as h0x-ade shipped them.
#[derive(Clone)]
struct Palette {
    background: u32,
    foreground: u32,
    selection: u32,
    cursor: Option<u32>,
    ansi: [u32; 16],
    /// working, blocked, done: the presets pick their own; theme files take
    /// them from ansi (yellow, red, blue). Idle is grey: it needs nobody.
    status: Option<[u32; 3]>,
}

const DARK_STATUS: Option<[u32; 3]> = Some([0xe5c07b, 0xf0705f, 0x76a8ff]);

const GRAPHITE: Palette = Palette {
    background: 0x17191d,
    foreground: 0xd2d4d9,
    selection: 0x394454,
    cursor: None,
    status: DARK_STATUS,
    ansi: [
        0x45484f, 0xdc8c95, 0xa6bd91, 0xd8be87, 0x96b2d7, 0xb5a2ce, 0x8cbdc3, 0xd2d4d9, 0x818793, 0xf0a5ad, 0xbad5a2, 0xead2a1, 0xb0cbef,
        0xcbb7e6, 0xa4d5db, 0xf0f0f2,
    ],
};
const INK: Palette = Palette {
    background: 0x111720,
    foreground: 0xcdd6e4,
    selection: 0x344861,
    cursor: None,
    status: DARK_STATUS,
    ansi: [
        0x414b5a, 0xe58c9a, 0x89c5aa, 0xd9be82, 0x91b6ee, 0xb9a0e2, 0x85c4d0, 0xcdd6e4, 0x7f91a7, 0xf5a5b2, 0xa5dec3, 0xedd39c, 0xaccbfa,
        0xd0b7f4, 0xa0dce5, 0xf0f5fc,
    ],
};
const MOSS: Palette = Palette {
    background: 0x151c19,
    foreground: 0xcdd7ce,
    selection: 0x354a40,
    cursor: None,
    status: DARK_STATUS,
    ansi: [
        0x435248, 0xdf978d, 0xa0c4a2, 0xd4bc8b, 0x99b9cd, 0xbba4c8, 0x8bbdb5, 0xcdd7ce, 0x82968a, 0xf1afa3, 0xb8dab9, 0xe9d2a5, 0xb1d0e2,
        0xd1bddc, 0xa4d6ca, 0xeff5ee,
    ],
};

/// Light: white paper, dark ink. Every ANSI color stays readable on white
/// (bright white is ink, not white: programs write it as "strong" text).
const PAPER: Palette = Palette {
    background: 0xffffff,
    foreground: 0x25272c,
    selection: 0xcfdcf5,
    cursor: Some(0x25272c),
    status: Some([0x9a6700, 0xc93c2c, 0x2f6fe4]),
    ansi: [
        0x25272c, 0xc62a2a, 0x1a7f37, 0x8f5d00, 0x1f62d0, 0x7a45c8, 0x0f6f74, 0x5f6068, 0x7b7e87, 0xd1242f, 0x1f883d, 0x9a6700, 0x0969da,
        0x8250df, 0x1b7c83, 0x111214,
    ],
};

pub const PRESETS: &[&str] = &["graphite", "ink", "moss", "paper"];

/// The theme a layout without one shows, by appearance.
pub const DEFAULT_DARK: &str = "graphite";
pub const DEFAULT_LIGHT: &str = "paper";

fn preset(name: &str) -> Option<&'static Palette> {
    match name {
        "graphite" => Some(&GRAPHITE),
        "ink" => Some(&INK),
        "moss" => Some(&MOSS),
        "paper" => Some(&PAPER),
        _ => None,
    }
}

/// The shipped theme files (defaults/themes), written to `themes/` when missing.
const THEMES: &[(&str, &str)] = &[
    ("atom-one-dark", include_str!("../defaults/themes/atom-one-dark.json")),
    ("win98", include_str!("../defaults/themes/win98.json")),
    ("amber-minimal", include_str!("../defaults/themes/amber-minimal.json")),
    ("candyland", include_str!("../defaults/themes/candyland.json")),
    ("claymorphism", include_str!("../defaults/themes/claymorphism.json")),
    ("darkmatter", include_str!("../defaults/themes/darkmatter.json")),
    ("soft-pop", include_str!("../defaults/themes/soft-pop.json")),
    ("neo-brutalism", include_str!("../defaults/themes/neo-brutalism.json")),
];

/// `themes/<name>.json`: a palette, plus `colors` overrides like settings.json's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    background: String,
    foreground: String,
    selection: String,
    cursor: Option<String>,
    ansi: [String; 16],
    #[serde(default)]
    colors: BTreeMap<String, String>,
}

fn theme_path(name: &str) -> std::path::PathBuf {
    crate::mods::app_dir().join("themes").join(format!("{name}.json"))
}

// Only these exact shipped versions may be replaced; personal themes stay intact.
const PREVIOUS_THEMES: &[(&str, &str)] = &[
    ("atom-one-dark", r##"{
  "background": "#21252b",
  "foreground": "#abb2bf",
  "selection": "#323844",
  "cursor": "#abb2bf",
  "ansi": [
    "#21252b", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
    "#767676", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf"
  ]
}
"##),
    ("win98", r##"{
  "background": "#ffffff",
  "foreground": "#c0c0c0",
  "selection": "#000080",
  "cursor": "#c0c0c0",
  "ansi": [
    "#000000", "#aa0000", "#00aa00", "#aa5500", "#0000aa", "#aa00aa", "#00aaaa", "#aaaaaa",
    "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff"
  ],
  "colors": {
    "foreground": "#000000",
    "surface": "#c0c0c0",
    "popover": "#c0c0c0",
    "raised": "#dfdfdf",
    "hover": "#dfdfdf",
    "border": "#808080",
    "input": "#ffffff",
    "muted": "#404040",
    "dim": "#808080",
    "accent": "#000080",
    "accent_foreground": "#ffffff",
    "ring": "#000000",
    "working": "#808000",
    "blocked": "#ff0000",
    "done": "#0000ff",
    "idle": "#808080",
    "unknown": "#808080"
  }
}
"##),
];

pub fn install_themes() {
    let _ = crate::mods::ensure_app_dir().and_then(|home| std::fs::create_dir_all(home.join("themes")));
    for (name, text) in THEMES {
        install_theme(&theme_path(name), name, text);
    }
}

fn install_theme(path: &std::path::Path, name: &str, text: &str) {
    let previous = PREVIOUS_THEMES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);
    if !path.exists() || std::fs::read_to_string(path).ok().as_deref().is_some_and(|s| Some(s) == previous) {
        let _ = crate::persist::write_atomic(path, text.as_bytes());
    }
}

/// A theme by name: a preset, or `themes/<name>.json`.
fn palette(name: &str) -> Result<(Palette, BTreeMap<String, String>), String> {
    if let Some(p) = preset(name) {
        return Ok((p.clone(), BTreeMap::new()));
    }
    let path = theme_path(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("theme {name:?} is not one of {} and there is no themes/{name}.json", PRESETS.join(", ")))?;
    parse_theme_file(name, &text)
}

fn parse_theme_file(name: &str, text: &str) -> Result<(Palette, BTreeMap<String, String>), String> {
    let f: ThemeFile = serde_json::from_str(text).map_err(|e| format!("themes/{name}.json: {e}"))?;
    let c = |k: &str, v: &str| {
        let h = v.trim().trim_start_matches('#');
        (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().ok_or(format!("themes/{name}.json: {k} {v:?} is not #rrggbb"))
    };
    let mut ansi = [0; 16];
    for (i, v) in f.ansi.iter().enumerate() {
        ansi[i] = c(&format!("ansi[{i}]"), v)?;
    }
    for (k, v) in &f.colors {
        if parse_hex(v).is_none() {
            return Err(format!("themes/{name}.json: colors.{k}: {v:?} is not #rrggbb or #rrggbbaa"));
        }
    }
    let cursor = f.cursor.as_deref().map(|v| c("cursor", v)).transpose()?;
    let p = Palette {
        background: c("background", &f.background)?,
        foreground: c("foreground", &f.foreground)?,
        selection: c("selection", &f.selection)?,
        cursor,
        ansi,
        status: None,
    };
    Ok((p, f.colors))
}

#[test]
fn shipped_theme_upgrades_preserve_personal_edits() {
    let root = std::env::temp_dir().join(format!("h0x-ade-theme-upgrades-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for (name, previous) in PREVIOUS_THEMES {
        let path = root.join(format!("{name}.json"));
        let current = THEMES.iter().find(|(n, _)| n == name).unwrap().1;
        install_theme(&path, name, current);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), current);
        std::fs::write(&path, previous).unwrap();
        install_theme(&path, name, current);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), current);
        let edited = previous.replace("#abb2bf", "#abcdef").replace("#c0c0c0", "#abcdef");
        std::fs::write(&path, &edited).unwrap();
        install_theme(&path, name, current);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn shipped_theme_files() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults/themes");
    let mut found = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        parse_theme_file(name, &text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(THEMES.iter().find(|(n, _)| *n == name).unwrap().1, text);
        found.insert(name.to_owned());
    }
    assert_eq!(found, THEMES.iter().map(|(n, _)| n.to_string()).collect());
}

/// Whether `name` is a theme that can be shown.
pub fn theme_exists(name: &str) -> bool {
    palette(name).is_ok()
}

/// Whether a theme is light (by its background), for picking its half of a pair.
pub fn theme_is_light(name: &str) -> bool {
    palette(name).map(|(p, _)| is_light(p.background)).unwrap_or(false)
}

fn is_light(c: u32) -> bool {
    let ch = |s: u32| ((c >> s) & 255) as f32 / 255.0;
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0) > 0.5
}

/// Every theme that can be shown, presets first, each with its colors
/// (the settings screen draws them as swatches).
pub fn themes() -> Vec<(String, Theme)> {
    let mut files: Vec<String> = std::fs::read_dir(crate::mods::app_dir().join("themes"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
        .filter(|n| !PRESETS.contains(&n.as_str()))
        .collect();
    files.sort();
    PRESETS
        .iter()
        .map(|n| n.to_string())
        .chain(files)
        .filter_map(|n| palette(&n).ok().map(|(p, o)| (n, resolve(&p, &o))))
        .collect()
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |acc, s| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        acc | (((x * (1.0 - t) + y * t).round() as u32) << s)
    })
}

/// Every color the app and mods use, resolved. Values are `#rrggbb` or `#rrggbbaa`.
pub type Theme = BTreeMap<String, String>;

/// The panel colors around a palette. `chrome` (sidebars, bars) and
/// `canvas` (behind cards) step away from the terminal's background:
/// darker on a dark theme, toward warm grey on a light one.
fn resolve(p: &Palette, overrides: &BTreeMap<String, String>) -> Theme {
    let bg = p.background;
    let light = is_light(bg);
    let fg = if light { 0x1b1b1e } else { 0xeeeeef };
    let [working, blocked, done] = p.status.unwrap_or([p.ansi[3], p.ansi[1], p.ansi[4]]);
    let (popover, raised, hover, selected, border, input, canvas, chrome) = if light {
        let warm = 0xe8e8e2;
        (hex(bg), hex(mix(bg, 0, 0.035)), "#0000000c".into(), "#00000012".into(), "#00000017".into(), "#00000026".into(), mix(bg, warm, 0.2), mix(bg, warm, 0.6))
    } else {
        (
            hex(mix(bg, 0xffffff, 0.06)),
            hex(mix(bg, 0xffffff, 0.09)),
            "#ffffff0e".into(),
            "#ffffff17".into(),
            "#ffffff18".into(),
            "#ffffff24".into(),
            mix(bg, 0, 0.16),
            mix(bg, 0, 0.32),
        )
    };
    let mut t: Theme = [
        ("background", hex(bg)),
        ("terminal_foreground", hex(p.foreground)),
        ("foreground", hex(fg)),
        ("surface", hex(bg)),
        ("canvas", hex(canvas)),
        ("chrome", hex(chrome)),
        ("popover", popover),
        ("raised", raised),
        ("hover", hover),
        ("selected", selected),
        ("border", border),
        ("input", input),
        ("muted", hex(mix(bg, fg, if light { 0.66 } else { 0.6 }))),
        ("dim", hex(mix(bg, fg, if light { 0.46 } else { 0.38 }))),
        ("accent", hex(p.ansi[4])),
        ("accent_foreground", hex(bg)),
        ("solid", hex(fg)),
        ("solid_foreground", hex(if light { 0xffffff } else { mix(bg, 0, 0.28) })),
        ("ring", hex(mix(bg, fg, 0.7))),
        ("selection", hex(p.selection)),
        ("cursor", hex(p.cursor.unwrap_or(p.foreground))),
        ("working", hex(working)),
        ("blocked", hex(blocked)),
        ("done", hex(done)),
        ("idle", hex(mix(bg, fg, 0.38))),
        ("unknown", hex(mix(bg, fg, 0.38))),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    for (i, c) in p.ansi.iter().enumerate() {
        t.insert(format!("ansi{i}"), hex(*c));
    }
    for (k, v) in overrides {
        t.insert(k.clone(), v.clone());
    }
    t
}

/// settings.json: preferences used where a layout leaves its look unset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    /// How big the whole app is drawn (1 = as designed). The zoom keys change it.
    pub zoom: f32,
    pub sounds: Sounds,
    /// The h0x-core to use, when it isn't where HyperOx looks (core_bin.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core: Option<String>,
    /// Fonts for layouts that don't set their own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Enable programming ligatures in terminal fonts.
    pub font_ligatures: bool,
    /// The panels' font (their sizes follow zoom).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<String>,
    /// Allow terminal programs (including remote ones) to copy text locally
    /// (OSC 52). Off unless set: any output could replace the clipboard.
    #[serde(skip_serializing_if = "is_false")]
    pub clipboard_write: bool,
    /// Look for HyperOx and h0x-core updates in the background; "Check now" always works.
    #[serde(skip_serializing_if = "is_true")]
    pub check_updates: bool,
    /// Send usage analytics (Settings > Info). Failure reports are not governed by this.
    #[serde(skip_serializing_if = "is_true", deserialize_with = "bool_or_true")]
    pub share_analytics: bool,
    /// The welcome's one-time "How did you find HyperOx?": the choice, or "skip". Set means asked.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "string_or_none")]
    pub found_via: Option<String>,
    /// A draggable scroll position thumb at a terminal pane's right edge.
    #[serde(skip_serializing_if = "is_true")]
    pub pane_scrollbars: bool,
    /// Minimum terminal text contrast ratio; 1 leaves program colors unchanged.
    pub minimum_contrast: f32,
    /// Scrollback lines kept per terminal pane (new panes). Anything not a whole
    /// number in SCROLLBACK_MIN..=SCROLLBACK_MAX falls back to the default.
    #[serde(skip_serializing_if = "is_default_scrollback", deserialize_with = "scrollback_or_default")]
    pub scrollback_lines: u32,
    /// macOS Option sends Alt shortcuts, or follows the keyboard layout.
    #[serde(skip_serializing_if = "OptionAsAlt::is_auto")]
    pub option_as_alt: OptionAsAlt,
    /// Light or dark: the system's, or one always.
    #[serde(skip_serializing_if = "Appearance::is_system")]
    pub appearance: Appearance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<crate::layout::ThemeChoice>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
    /// Mods that did not ship with HyperOx: whether the person let them run,
    /// for the code they had then (ADR-048).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub mods: BTreeMap<String, ModChoice>,
}

/// The person's answer for one mod's code (`mods::trust`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModChoice {
    pub hash: String,
    pub allow: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    fn is_system(&self) -> bool {
        *self == Appearance::System
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsAlt {
    #[default]
    Auto,
    Off,
    On,
    Left,
    Right,
}

impl OptionAsAlt {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// Scrollback per terminal pane. The default keeps the long-standing 10,000
/// lines; the range keeps a pane between ~1 MB and ~100 MB (see `Vt::new`).
pub const SCROLLBACK_DEFAULT: u32 = 10_000;
pub const SCROLLBACK_MIN: u32 = 1_000;
pub const SCROLLBACK_MAX: u32 = 100_000;

fn is_default_scrollback(n: &u32) -> bool {
    *n == SCROLLBACK_DEFAULT
}

/// A whole number in range (2000.0 and 2e3 count), else the default (never an error: a typo here must not discard the file).
fn scrollback_or_default<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|n| n.fract() == 0.0 && (SCROLLBACK_MIN as f64..=SCROLLBACK_MAX as f64).contains(n)).map_or(SCROLLBACK_DEFAULT, |n| n as u32))
}

/// This install's random ID (128 bits, hex): `install-id` in `dir`, the app home. Made on first
/// need and reused; made again only if the file is missing or not 32 hex digits.
#[allow(dead_code)] // sent by the analytics step
pub fn install_id(dir: &std::path::Path) -> String {
    let file = dir.join("install-id");
    if let Ok(text) = std::fs::read_to_string(&file) {
        let id = text.trim();
        if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            let lower = id.to_ascii_lowercase();
            if lower != id {
                let _ = std::fs::write(&file, &lower);
            }
            return lower;
        }
    }
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded by the OS (as Model::fresh_id does).
    let random = || std::collections::hash_map::RandomState::new().build_hasher().finish();
    let id = format!("{:016x}{:016x}", random(), random());
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, &id)) {
        tracing::warn!("install-id not saved: {e}");
        // Not saved: keep one id for this process (one app home per process) so each call isn't a new install.
        static UNSAVED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        return UNSAVED.get_or_init(|| id).clone();
    }
    id
}

/// A bool; any other type is the default (true), so one bad value can't discard the file.
fn bool_or_true<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_bool().unwrap_or(true))
}

/// A string; any other type is None (not yet asked).
fn string_or_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
    })
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for Person {
    fn default() -> Self {
        Person {
            zoom: 1.0,
            sounds: Sounds::default(),
            core: None,
            font: None,
            font_size: None,
            font_ligatures: false,
            ui_font: None,
            clipboard_write: false,
            check_updates: true,
            share_analytics: true,
            found_via: None,
            pane_scrollbars: true,
            minimum_contrast: 1.0,
            scrollback_lines: SCROLLBACK_DEFAULT,
            option_as_alt: OptionAsAlt::Auto,
            appearance: Appearance::System,
            theme: None,
            colors: BTreeMap::new(),
            mods: BTreeMap::new(),
        }
    }
}

/// What the app runs with: the shown layout's look over the defaults, at
/// the person's zoom. Everything reads this one value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_family: String,
    pub font_size: f32,
    pub font_ligatures: bool,
    pub ui_font_family: String,
    pub ui_font_size: f32,
    /// How big the whole app is drawn (1 = as designed). The zoom keys change it.
    pub zoom: f32,
    /// The theme showing: a preset or a file in themes/.
    pub theme: String,
    /// The layout's theme for each appearance (the same one twice when it names one).
    pub theme_light: String,
    pub theme_dark: String,
    /// Whether the window is dark now (the person's `appearance`, else the system's).
    pub dark: bool,
    /// Any theme color by name (see `theme()` in h0x.d.ts), `#rrggbb` or `#rrggbbaa`.
    pub colors: BTreeMap<String, String>,
    /// A draggable scroll position thumb at a terminal pane's right edge.
    pub pane_scrollbars: bool,
    /// Space between a terminal's edge and its text, in px: [x, y].
    pub terminal_padding: [f32; 2],
    /// 0..1: below 1, what's behind a terminal shows through.
    pub terminal_opacity: f32,
    pub clipboard_write: bool,
    pub check_updates: bool,
    pub share_analytics: bool,
    pub minimum_contrast: f32,
    /// Scrollback lines per new terminal pane.
    pub scrollback_lines: u32,
    pub option_as_alt: OptionAsAlt,
    pub sounds: Sounds,
}

/// Attention sounds: when an agent needs you, and when one finished while you
/// weren't looking. `done` / `blocked`: an MP3 file instead of the bundled one
/// (absolute, or relative to ~/.config/h0x-ade).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sounds {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

impl Default for Sounds {
    fn default() -> Self {
        Sounds { enabled: true, done: None, blocked: None }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font_family: DEFAULT_FONT.into(),
            font_size: 14.0,
            font_ligatures: false,
            ui_font_family: "Geist".into(),
            ui_font_size: 13.0,
            zoom: 1.0,
            theme: DEFAULT_DARK.into(),
            theme_light: DEFAULT_LIGHT.into(),
            theme_dark: DEFAULT_DARK.into(),
            dark: true,
            colors: BTreeMap::new(),
            pane_scrollbars: true,
            terminal_padding: [10.0, 6.0],
            terminal_opacity: 1.0,
            clipboard_write: false,
            check_updates: true,
            share_analytics: true,
            minimum_contrast: 1.0,
            scrollback_lines: SCROLLBACK_DEFAULT,
            option_as_alt: OptionAsAlt::Auto,
            sounds: Sounds::default(),
        }
    }
}

impl Settings {
    pub fn compose(person: &Person, look: &crate::layout::Look, system_dark: bool) -> Settings {
        let d = Settings::default();
        let mut colors = person.colors.clone();
        colors.extend(look.colors.clone().unwrap_or_default());
        let dark = match person.appearance {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        let (theme_light, theme_dark) = match look.theme.as_ref().or(person.theme.as_ref()) {
            Some(c) => (c.pick(false).to_owned(), c.pick(true).to_owned()),
            None => (DEFAULT_LIGHT.to_owned(), DEFAULT_DARK.to_owned()),
        };
        Settings {
            // The layout's own, else the person's, else HyperOx's.
            font_family: look.font.clone().or(person.font.clone()).unwrap_or(d.font_family),
            font_size: look.font_size.or(person.font_size).unwrap_or(d.font_size),
            font_ligatures: person.font_ligatures,
            ui_font_family: look.ui_font.clone().or(person.ui_font.clone()).unwrap_or(d.ui_font_family),
            ui_font_size: d.ui_font_size,
            zoom: person.zoom,
            theme: if dark { theme_dark.clone() } else { theme_light.clone() },
            theme_light,
            theme_dark,
            dark,
            colors,
            pane_scrollbars: person.pane_scrollbars,
            terminal_padding: look.terminal_padding.unwrap_or(d.terminal_padding),
            terminal_opacity: look.terminal_opacity.unwrap_or(1.0).clamp(0.0, 1.0),
            sounds: person.sounds.clone(),
            clipboard_write: person.clipboard_write,
            check_updates: person.check_updates,
            share_analytics: person.share_analytics,
            minimum_contrast: person.minimum_contrast,
            scrollback_lines: person.scrollback_lines,
            option_as_alt: person.option_as_alt,
        }
    }
}

pub fn parse_hex(s: &str) -> Option<Hsla> {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok()?;
    let (rgb, a) = match h.len() {
        6 => (v, 255),
        8 => (v >> 8, v & 255),
        _ => return None,
    };
    Some(
        Rgba {
            r: ((rgb >> 16) & 255) as f32 / 255.0,
            g: ((rgb >> 8) & 255) as f32 / 255.0,
            b: (rgb & 255) as f32 / 255.0,
            a: a as f32 / 255.0,
        }
        .into(),
    )
}

pub fn rgb(s: &str) -> Rgb {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(&h[..h.len().min(6)], 16).unwrap_or(0xff00ff);
    Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Settings {
    /// Problems an agent should hear about before the file is applied.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for name in [&self.theme_light, &self.theme_dark] {
            if let Err(e) = palette(name) {
                out.push(e);
            }
        }
        out.dedup();
        for (k, v) in &self.colors {
            if parse_hex(v).is_none() {
                out.push(format!("colors.{k}: {v:?} is not #rrggbb or #rrggbbaa"));
            }
        }
        if !(1.0..=21.0).contains(&self.minimum_contrast) {
            out.push(format!("minimum_contrast {} is outside 1..21", self.minimum_contrast));
        }
        if !(6.0..=72.0).contains(&self.font_size) {
            out.push(format!("font_size {} is outside 6..72", self.font_size));
        }
        out
    }
}

pub fn path() -> std::path::PathBuf {
    crate::mods::app_dir().join("settings.json")
}

thread_local! {
    static CURRENT: RefCell<(Rc<Settings>, Rc<Theme>)> =
        RefCell::new((Rc::new(Settings::default()), Rc::new(resolve(&GRAPHITE, &BTreeMap::new()))));
}

const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;

pub fn get() -> Rc<Settings> {
    CURRENT.with(|c| c.borrow().0.clone())
}

/// The resolved theme: every color by name.
pub fn theme() -> Rc<Theme> {
    CURRENT.with(|c| c.borrow().1.clone())
}

/// A theme color as GPUI paint (magenta if missing, so mistakes show).
pub fn color(name: &str) -> Hsla {
    theme().get(name).and_then(|v| parse_hex(v)).unwrap_or(gpui::hsla(0.83, 1.0, 0.5, 1.0))
}

/// A theme color name, or a literal `#rrggbb[aa]`.
pub fn color_or_literal(v: &str) -> Hsla {
    parse_hex(v).unwrap_or_else(|| color(v))
}

/// Reads settings.json (writing the defaults if it's missing), or says why it can't be used.
pub fn load_person() -> Result<Person, String> {
    let p = path();
    if !p.exists() {
        let _ = crate::persist::write_atomic(&p, serde_json::to_string_pretty(&Person::default()).unwrap() + "\n");
    }
    parse_person(&std::fs::read_to_string(&p).map_err(|e| e.to_string())?)
}

/// settings.json's text as the person's settings; a leading UTF-8 BOM (some editors add one) is ignored.
fn parse_person(text: &str) -> Result<Person, String> {
    let person: Person = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if !(ZOOM_MIN..=ZOOM_MAX).contains(&person.zoom) {
        return Err(format!("zoom {} is outside {ZOOM_MIN}..{ZOOM_MAX}", person.zoom));
    }
    Ok(person)
}

pub fn write_person(person: &Person) -> std::io::Result<()> {
    crate::persist::write_atomic(&path(), serde_json::to_string_pretty(person).unwrap_or_default() + "\n")
}

/// Installs new settings; returns whether anything changed.
pub fn set(s: Settings) -> bool {
    crate::update::set_auto(s.check_updates);
    #[cfg(target_os = "macos")]
    gpui_macos::set_option_as_alt(match s.option_as_alt {
        OptionAsAlt::Auto => None,
        OptionAsAlt::Off => Some(gpui_macos::OptionAsAlt::None),
        OptionAsAlt::On => Some(gpui_macos::OptionAsAlt::Both),
        OptionAsAlt::Left => Some(gpui_macos::OptionAsAlt::Left),
        OptionAsAlt::Right => Some(gpui_macos::OptionAsAlt::Right),
    });
    CURRENT.with(|c| {
        // Read the theme even when the settings are the same: its file may
        // have new colors under the same name.
        let (p, mut colors) = palette(&s.theme).unwrap_or_else(|_| (GRAPHITE.clone(), BTreeMap::new()));
        colors.extend(s.colors.clone());
        let theme = resolve(&p, &colors);
        if *c.borrow().0 == s && *c.borrow().1 == theme {
            return false;
        }
        gpui_shell::set_pixel_scale(s.zoom);
        *c.borrow_mut() = (Rc::new(s), Rc::new(theme));
        true
    })
}

/// The zoom a first start picks: the system's text size (GNOME's large text,
/// Windows' "make text bigger"), a little more on a big screen at 100%
/// (logical px: a scaled screen already draws bigger), in tenths.
pub fn first_zoom(display_width: Option<f32>) -> f32 {
    let text = system_text_scale().clamp(1.0, 2.0);
    let screen = if display_width.is_some_and(|w| w >= 2400.0) { 1.15 } else { 1.0 };
    ((text * screen * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX)
}

fn system_text_scale() -> f32 {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = crate::spawn::command(cmd).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    if cfg!(windows) {
        // Percent, 100..225, as REG_DWORD hex.
        return run("reg", &["query", r"HKCU\Software\Microsoft\Accessibility", "/v", "TextScaleFactor"])
            .and_then(|o| o.split_whitespace().last().and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok()))
            .map_or(1.0, |p| p as f32 / 100.0);
    }
    if cfg!(target_os = "linux") {
        return run("gsettings", &["get", "org.gnome.desktop.interface", "text-scaling-factor"]).and_then(|o| o.trim().parse().ok()).unwrap_or(1.0);
    }
    1.0
}

/// The next zoom level in `steps` tenths (0 resets).
pub fn next_zoom(steps: i32) -> f32 {
    if steps == 0 {
        return 1.0;
    }
    ((zoom() * 10.0).round() + steps as f32).clamp(ZOOM_MIN * 10.0, ZOOM_MAX * 10.0) / 10.0
}

pub fn zoom() -> f32 {
    get().zoom
}

/// A designed size in px, at the current zoom.
pub fn z(v: f32) -> Pixels {
    px(v * zoom())
}

pub fn font_size() -> f32 {
    get().font_size * zoom()
}

pub fn ui_font_size() -> f32 {
    get().ui_font_size * zoom()
}

/// Font families on this machine (the bundled ones and the home's included).
static FONTS: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

pub fn note_fonts(names: Vec<String>) {
    *FONTS.lock().unwrap() = Some(names.into_iter().map(|n| n.to_lowercase()).collect());
}

/// Where fonts load from: `home/fonts` and the `fonts/` of each mod that may
/// run (HyperOx's own, or allowed by the person for its current code). A mod
/// not allowed yet, or denied, loads none: font files are parsed natively.
fn font_dirs(home: &Path, mods: &[crate::mods::Found], choices: &BTreeMap<String, ModChoice>) -> Vec<PathBuf> {
    let mut dirs = vec![home.join("fonts")];
    dirs.extend(mods.iter().filter(|m| crate::mods::trust(m, choices) == crate::mods::Trust::Allowed).map(|m| m.dir.join("fonts")));
    dirs
}

/// Font files the home brings: `fonts/` and each allowed mod's `fonts/` (a
/// layout's look travels with its mod, as HyperOx 98's does). Loads the ones
/// not loaded yet; true when any was.
pub fn load_home_fonts(cx: &mut gpui::App) -> bool {
    static LOADED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let home = crate::mods::app_dir();
    let person = load_person().unwrap_or_default();
    let dirs = font_dirs(&home, &crate::mods::scan(), &person.mods);
    let mut new = Vec::new();
    let mut loaded = LOADED.lock().unwrap();
    for dir in dirs {
        for path in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            let font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "ttf" | "otf" | "ttc"));
            if font && !loaded.contains(&path) && let Ok(bytes) = std::fs::read(&path) {
                new.push(std::borrow::Cow::Owned(bytes));
                loaded.push(path);
            }
        }
    }
    if new.is_empty() {
        return false;
    }
    if let Err(e) = cx.text_system().add_fonts(new) {
        tracing::warn!("fonts in {} not loaded: {e:#}", home.display());
    }
    FONTS_ADDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    note_fonts(cx.text_system().all_font_names());
    true
}

static FONTS_ADDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times fonts were added while running: text shaped before may now
/// fall back to other glyphs.
pub fn fonts_added() -> u64 {
    FONTS_ADDED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The terminal font, or the bundled default when this machine lacks it:
/// the platform's own fallback is proportional and breaks the grid.
pub fn terminal_font() -> String {
    let family = get().font_family.clone();
    match FONTS.lock().unwrap().as_ref() {
        Some(have) if !have.contains(&family.to_lowercase()) => DEFAULT_FONT.into(),
        _ => family,
    }
}

/// Fonts HyperOx ships, so the defaults look the same everywhere.
pub const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/GeistMono-Variable.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Italic-Variable.ttf"),
    include_bytes!("../assets/fonts/Geist.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Variable.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic-Variable.ttf"),
    SYMBOLS_FONT_FILE,
];

/// The symbols font itself: terminals read icons' glyphs from it directly.
pub const SYMBOLS_FONT_FILE: &[u8] = include_bytes!("../assets/fonts/SymbolsNerdFont-Regular.ttf");

/// Nerd Font icons for terminals, under any font (MIT, nerd-fonts).
pub const SYMBOLS_FONT: &str = "Symbols Nerd Font";

/// Terminals' font unless a layout or settings.json picks another.
pub const DEFAULT_FONT: &str = "JetBrains Mono";

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_ligatures_default_off_and_can_be_enabled() {
        use super::*;
        let person: Person = serde_json::from_str("{}").unwrap();
        assert!(!Settings::compose(&person, &crate::layout::Look::default(), true).font_ligatures);
        assert!(!Settings::default().font_ligatures);
        let person: Person = serde_json::from_str(r#"{"font_ligatures":true}"#).unwrap();
        let saved = serde_json::to_string(&person).unwrap();
        let restored: Person = serde_json::from_str(&saved).unwrap();
        assert!(Settings::compose(&restored, &crate::layout::Look::default(), true).font_ligatures);
        assert!(serde_json::from_str::<Person>(r#"{"font_ligatures":"yes"}"#).is_err());
    }

    #[test]
    fn an_old_settings_file_with_telemetry_still_loads_and_keeps_its_other_keys() {
        for telemetry in ["true", "false", "\"garbage\"", "null", "{}"] {
            let text = format!(r#"{{"telemetry": {telemetry}, "font_ligatures": true, "zoom": 1.25, "clipboard_write": false}}"#);
            let person: Person = serde_json::from_str(&text).unwrap_or_else(|e| panic!("telemetry {telemetry}: {e}"));
            assert!(person.font_ligatures, "telemetry {telemetry}");
            assert_eq!(person.zoom, 1.25, "telemetry {telemetry}");
            assert!(!person.clipboard_write, "telemetry {telemetry}");
            // Saving it again drops the retired key.
            assert!(serde_json::to_value(&person).unwrap().get("telemetry").is_none(), "telemetry {telemetry}");
        }
    }

    use super::*;

    #[test]
    fn layout_look_wins_and_settings_fill_what_it_leaves_unset() {
        let person = Person {
            font: Some("person".into()), font_size: Some(19.0), ui_font: Some("person-ui".into()),
            theme: Some(crate::layout::ThemeChoice::One("paper".into())),
            colors: [("accent".into(), "#123456".into())].into(),
            ..Person::default()
        };
        let look = crate::layout::Look {
            font: Some("layout".into()), font_size: Some(14.0), ui_font: Some("layout-ui".into()),
            theme: Some(crate::layout::ThemeChoice::One("ink".into())),
            colors: Some([("accent".into(), "#abcdef".into()), ("text".into(), "#ffffff".into())].into()),
            ..Default::default()
        };
        let inherited = Settings::compose(&person, &crate::layout::Look::default(), true);
        assert_eq!((inherited.font_family.as_str(), inherited.font_size, inherited.ui_font_family.as_str(), inherited.theme.as_str()), ("person", 19.0, "person-ui", "paper"));
        assert_eq!(inherited.colors["accent"], "#123456");
        let own = Settings::compose(&person, &look, true);
        assert_eq!((own.font_family.as_str(), own.font_size, own.ui_font_family.as_str(), own.theme.as_str()), ("layout", 14.0, "layout-ui", "ink"));
        assert_eq!(own.colors["accent"], "#abcdef");
        assert_eq!(own.colors["text"], "#ffffff");
    }

    #[test]
    fn option_as_alt_defaults_to_auto_and_round_trips() {
        let default: Person = serde_json::from_str("{}").unwrap();
        assert_eq!(default.option_as_alt, OptionAsAlt::Auto);
        for (value, expected) in [("auto", OptionAsAlt::Auto), ("off", OptionAsAlt::Off), ("on", OptionAsAlt::On), ("left", OptionAsAlt::Left), ("right", OptionAsAlt::Right)] {
            let person: Person = serde_json::from_value(serde_json::json!({"option_as_alt": value})).unwrap();
            let settings = Settings::compose(&person, &crate::layout::Look::default(), true);
            assert_eq!(settings.option_as_alt, expected);
            assert_eq!(serde_json::to_value(settings).unwrap()["option_as_alt"], value);
        }
        assert!(serde_json::from_value::<Person>(serde_json::json!({"option_as_alt": "invalid"})).is_err());
    }

    #[test]
    fn minimum_contrast_defaults_off_and_validates_the_ratio() {
        let default: Person = serde_json::from_str("{}").unwrap();
        assert_eq!(default.minimum_contrast, 1.0);
        assert_eq!(Settings::default().minimum_contrast, 1.0);
        for ratio in [1.0, 1.6, 21.0, 0.0, 21.1] {
            let person: Person = serde_json::from_value(serde_json::json!({"minimum_contrast": ratio})).unwrap();
            let settings = Settings::compose(&person, &crate::layout::Look::default(), true);
            assert_eq!(settings.minimum_contrast, ratio);
            assert_eq!(serde_json::to_value(person).unwrap()["minimum_contrast"], serde_json::json!(ratio));
            assert_eq!(settings.problems().is_empty(), (1.0..=21.0).contains(&ratio));
        }
    }

    #[test]
    fn scrollback_lines_defaults_to_ten_thousand_and_bad_values_fall_back() {
        assert_eq!(SCROLLBACK_DEFAULT, 10_000);
        assert_eq!(Settings::default().scrollback_lines, 10_000);
        let lines = |text: &str| serde_json::from_str::<Person>(text).unwrap().scrollback_lines;
        assert_eq!(lines("{}"), 10_000);
        assert_eq!(lines(r#"{"scrollback_lines": 1000}"#), 1_000);
        assert_eq!(lines(r#"{"scrollback_lines": 100000}"#), 100_000);
        assert_eq!(lines(r#"{"scrollback_lines": 50000}"#), 50_000);
        // A whole number written with a fraction part or an exponent is still that number.
        for whole in ["2000.0", "2e3", "2.0E3", "1000.0", "100000.0", "1e5"] {
            let want = whole.parse::<f64>().unwrap() as u32;
            assert_eq!(lines(&format!(r#"{{"scrollback_lines": {whole}}}"#)), want, "{whole}");
        }
        for bad in ["2000.5", "999.0", "100001.0", "1e6", "1e-3"] {
            assert_eq!(lines(&format!(r#"{{"scrollback_lines": {bad}}}"#)), 10_000, "{bad}");
        }
        for bad in ["999", "100001", "0", "-5", "5000.5", "\"5000\"", "null", "true", "[]", "99999999999"] {
            assert_eq!(lines(&format!(r#"{{"scrollback_lines": {bad}}}"#)), 10_000, "{bad}");
        }
        let person: Person = serde_json::from_str(r#"{"scrollback_lines": 2000}"#).unwrap();
        assert_eq!(Settings::compose(&person, &crate::layout::Look::default(), true).scrollback_lines, 2_000);
        assert_eq!(serde_json::to_value(&person).unwrap()["scrollback_lines"], serde_json::json!(2000));
        assert!(serde_json::to_value(Person::default()).unwrap().get("scrollback_lines").is_none());
    }

    #[test]
    fn a_settings_file_with_a_byte_order_mark_loads() {
        let person = parse_person("\u{feff}{\"scrollback_lines\": 2000, \"font_ligatures\": true}").unwrap();
        assert_eq!((person.scrollback_lines, person.font_ligatures), (2_000, true));
        assert_eq!(parse_person("\u{feff}{}").unwrap(), Person::default());
        assert!(parse_person("\u{feff}").is_err(), "an empty file is still reported");
        assert!(parse_person("{\"zoom\": 9}").is_err());
    }

    #[test]
    fn a_hand_edited_scrollback_value_with_whitespace_and_crlf_still_applies() {
        let lines = |text: &str| serde_json::from_str::<Person>(text).unwrap_or_else(|e| panic!("{text:?}: {e}")).scrollback_lines;
        assert_eq!(lines("{\r\n  \"scrollback_lines\" :\t 7500 \r\n}\r\n"), 7_500);
        assert_eq!(lines("\n\n{\"scrollback_lines\":1000}\n\n"), 1_000);
        // Whitespace inside a quoted number is still a string, not a whole number.
        for padded in ["\" 7500\"", "\"7500 \"", "\"\"", "\"1e3\""] {
            assert_eq!(lines(&format!("{{\"scrollback_lines\": {padded}}}")), SCROLLBACK_DEFAULT, "{padded}");
        }
    }

    #[test]
    fn a_bad_scrollback_value_keeps_the_rest_of_the_settings_and_a_good_one_survives_other_out_of_range_values() {
        let bad = serde_json::from_str::<Person>(
            r#"{"scrollback_lines": 999, "font_ligatures": true, "clipboard_write": true, "minimum_contrast": 4.5, "zoom": 1.5, "font": "Mono"}"#,
        )
        .unwrap();
        assert_eq!(bad.scrollback_lines, SCROLLBACK_DEFAULT);
        assert!(bad.font_ligatures && bad.clipboard_write);
        assert_eq!((bad.minimum_contrast, bad.zoom, bad.font.as_deref()), (4.5, 1.5, Some("Mono")));
        let s = Settings::compose(&bad, &crate::layout::Look::default(), true);
        assert_eq!((s.scrollback_lines, s.font_family.as_str()), (SCROLLBACK_DEFAULT, "Mono"));
        assert!(s.problems().is_empty(), "a fallen-back scrollback value is not a problem: {:?}", s.problems());

        // Other values out of range are reported, and the good scrollback value stays.
        let good = serde_json::from_str::<Person>(r#"{"scrollback_lines": 2000, "minimum_contrast": 50, "font_size": 100}"#).unwrap();
        let s = Settings::compose(&good, &crate::layout::Look::default(), true);
        assert_eq!(s.scrollback_lines, 2_000);
        let problems = s.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().all(|p| !p.contains("scrollback")), "{problems:?}");

        // A wrongly typed other key still rejects the whole file (the app keeps what it had).
        assert!(serde_json::from_str::<Person>(r#"{"scrollback_lines": 2000, "check_updates": "yes"}"#).is_err());
    }

    #[test]
    fn scrollback_lines_written_by_the_stepper_round_trip_through_the_settings_file() {
        // The stepper's stops, plus a value set by hand between two of them.
        for n in [1_000, 2_000, 5_000, 7_500, 10_000, 20_000, 50_000, 100_000] {
            let person = Person { scrollback_lines: n, font_ligatures: true, ..Person::default() };
            let file = serde_json::to_string_pretty(&person).unwrap() + "\n";
            let back: Person = serde_json::from_str(&file).unwrap_or_else(|e| panic!("{n}: {e}\n{file}"));
            assert_eq!(back.scrollback_lines, n, "{file}");
            assert!(back.font_ligatures, "{file}");
            assert_eq!(file.contains("scrollback_lines"), n != SCROLLBACK_DEFAULT, "only a non-default value is written: {file}");
            assert_eq!(Settings::compose(&back, &crate::layout::Look::default(), false).scrollback_lines, n);
        }
    }

    #[test]
    fn an_empty_settings_file_is_an_error_and_an_empty_object_means_the_default() {
        // load_person reports these (settings.error) and the app keeps the settings it had.
        for empty in ["", "\n", "  \r\n"] {
            assert!(serde_json::from_str::<Person>(empty).is_err(), "{empty:?}");
        }
        let person: Person = serde_json::from_str("{}").unwrap();
        assert_eq!(person.scrollback_lines, SCROLLBACK_DEFAULT);
        assert!(!serde_json::to_string_pretty(&person).unwrap().contains("scrollback_lines"));
    }

    #[test]
    fn a_layout_cannot_change_the_scrollback() {
        // Layouts (local, a mod's, or a remote machine's) carry only a Look;
        // a scrollback key in one is ignored and the person's value is used.
        let look: crate::layout::Look = serde_json::from_str(r#"{"scrollback_lines": 1000, "font_size": 20}"#).unwrap();
        assert_eq!(look.font_size, Some(20.0));
        let person = Person { scrollback_lines: 50_000, ..Person::default() };
        for dark in [true, false] {
            assert_eq!(Settings::compose(&person, &look, dark).scrollback_lines, 50_000);
            assert_eq!(Settings::compose(&Person::default(), &look, dark).scrollback_lines, SCROLLBACK_DEFAULT);
        }
    }

    #[test]
    fn program_clipboard_writes_default_off_and_can_be_enabled() {
        let default: Person = serde_json::from_str("{}").unwrap();
        assert!(!default.clipboard_write);
        assert!(!Settings::compose(&default, &crate::layout::Look::default(), true).clipboard_write);
        assert!(serde_json::to_value(&default).unwrap().get("clipboard_write").is_none());
        let on: Person = serde_json::from_str(r#"{"clipboard_write":true}"#).unwrap();
        assert!(Settings::compose(&on, &crate::layout::Look::default(), true).clipboard_write);
        assert_eq!(serde_json::to_value(on).unwrap()["clipboard_write"], true);
        let off: Person = serde_json::from_str(r#"{"clipboard_write":false}"#).unwrap();
        assert!(!off.clipboard_write);
    }

    #[test]
    fn update_checks_default_on_and_can_be_turned_off() {
        let default: Person = serde_json::from_str("{}").unwrap();
        assert!(default.check_updates);
        assert!(serde_json::to_value(&default).unwrap().get("check_updates").is_none());
        let off: Person = serde_json::from_str(r#"{"check_updates":false}"#).unwrap();
        assert!(!Settings::compose(&off, &crate::layout::Look::default(), true).check_updates);
        assert_eq!(serde_json::to_value(off).unwrap()["check_updates"], false);
    }

    #[test]
    fn fonts_load_only_from_mods_that_may_run() {
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let found = |id: &str| {
            let dir = root.join("mods").join(id);
            std::fs::create_dir_all(dir.join("fonts")).unwrap();
            std::fs::write(dir.join("main.js"), id).unwrap();
            crate::mods::Found { id: id.into(), dir, hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH }
        };
        let (allowed, denied, unasked, stale) = (found("allowed"), found("denied"), found("unasked"), found("stale"));
        let choice = |m: &crate::mods::Found, allow, hash: Option<String>| ModChoice { hash: hash.unwrap_or_else(|| crate::mods::content_hash(&m.dir)), allow };
        let choices: BTreeMap<String, ModChoice> = [
            ("allowed".to_owned(), choice(&allowed, true, None)),
            ("denied".to_owned(), choice(&denied, false, None)),
            ("stale".to_owned(), choice(&stale, true, Some("old code".into()))),
        ]
        .into();
        let mods = [allowed.clone(), denied, unasked, stale];
        assert_eq!(font_dirs(&root, &mods, &choices), vec![root.join("fonts"), allowed.dir.join("fonts")]);
        assert_eq!(font_dirs(&root, &mods, &BTreeMap::new()), vec![root.join("fonts")], "no consent, no mod fonts");
        assert_eq!(font_dirs(&root, &[], &choices), vec![root.join("fonts")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_whose_fonts_change_after_it_was_allowed_loads_none() {
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-changed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("mods").join("O\u{2019}Neil fonts");
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("main.js"), "export default 1").unwrap();
        let found = crate::mods::Found { id: "O\u{2019}Neil fonts".into(), dir: dir.clone(), hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH };
        let choices: BTreeMap<String, ModChoice> = [(found.id.clone(), ModChoice { hash: crate::mods::content_hash(&dir), allow: true })].into();
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &choices), vec![root.join("fonts"), dir.join("fonts")]);
        // A font file added after the person said yes is new code: ask again.
        std::fs::write(dir.join("fonts").join("planted.ttf"), b"not a font").unwrap();
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &choices), vec![root.join("fonts")], "stale answer");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_mod_fonts_link_to_elsewhere_counts_toward_its_hash() {
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let dir = root.join("mods").join("linked");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.js"), "export default 1").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("fonts")).unwrap();
        let found = crate::mods::Found { id: "linked".into(), dir: dir.clone(), hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH };
        let choices: BTreeMap<String, ModChoice> = [("linked".to_owned(), ModChoice { hash: crate::mods::content_hash(&dir), allow: true })].into();
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &choices).len(), 2);
        // Changing what the link points at changes the mod's code.
        std::fs::write(elsewhere.join("planted.ttf"), b"not a font").unwrap();
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &choices), vec![root.join("fonts")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shipped_mod_brings_its_fonts_without_asking_until_it_is_changed() {
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let name = "toast";
        let dir = root.join("mods").join(name);
        let mut files = 0;
        for (path, bytes) in crate::mods::DEFAULT_MODS {
            let Some(rel) = path.strip_prefix(&format!("{name}/")) else { continue };
            let to = dir.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, bytes).unwrap();
            files += 1;
        }
        assert!(files > 0, "HyperOx ships a {name} mod");
        let found = crate::mods::Found { id: name.into(), dir: dir.clone(), hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH };
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &BTreeMap::new()), vec![root.join("fonts"), dir.join("fonts")]);
        // A font added to a shipped mod makes it someone else's code.
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("fonts").join("planted.ttf"), b"not a font").unwrap();
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &BTreeMap::new()), vec![root.join("fonts")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_todo_mod_ships_the_handwriting_font_its_panel_asks_for() {
        let file = |path: &str| crate::mods::DEFAULT_MODS.iter().find(|(p, _)| *p == path).map(|(_, b)| *b).unwrap_or_else(|| panic!("{path} is not shipped"));
        let font = file("todo/fonts/PatrickHand-Regular.ttf");
        assert!(font.len() > 1024 && font[..4] == [0, 1, 0, 0], "a TrueType file, not {:?}", &font[..font.len().min(4)]);
        // The family the panel names is the one in the font's (UTF-16BE) name table.
        let family: Vec<u8> = "Patrick Hand".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert!(font.windows(family.len()).any(|w| w == family.as_slice()), "the font's family is Patrick Hand");
        assert!(String::from_utf8_lossy(file("todo/main.js")).contains("\"Patrick Hand\""));
        assert!(String::from_utf8_lossy(file("todo/fonts/OFL-PatrickHand.txt")).contains("SIL OPEN FONT LICENSE Version 1.1"));
        // As shipped, the todo mod's fonts load without a question.
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-todo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("mods").join("todo");
        for (path, bytes) in crate::mods::DEFAULT_MODS {
            let Some(rel) = path.strip_prefix("todo/") else { continue };
            std::fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
            std::fs::write(dir.join(rel), bytes).unwrap();
        }
        let found = crate::mods::Found { id: "todo".into(), dir: dir.clone(), hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH };
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &BTreeMap::new()), vec![root.join("fonts"), dir.join("fonts")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_or_malformed_settings_allow_no_mod_fonts() {
        let root = std::env::temp_dir().join(format!("h0x-ade-fonts-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("mods").join("mine");
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("main.js"), "export default 1").unwrap();
        let found = crate::mods::Found { id: "mine".into(), dir: dir.clone(), hint: Default::default(), stamp: std::time::SystemTime::UNIX_EPOCH };
        let mods = format!(r#""mods":{{"mine":{{"hash":"{}","allow":true}}}}"#, crate::mods::content_hash(&dir));
        // As load_home_fonts reads settings.json: unreadable means the defaults.
        let dirs = |text: &str| font_dirs(&root, std::slice::from_ref(&found), &serde_json::from_str::<Person>(text).unwrap_or_default().mods);
        assert_eq!(dirs(&format!("{{{mods}}}")), vec![root.join("fonts"), dir.join("fonts")], "well-formed control");
        for bad in [
            format!(r#"{{{mods},"check_updates":"no"}}"#),
            format!(r#"{{{mods},"clipboard_write":null}}"#),
            format!("{{{mods}"),
            String::new(),
        ] {
            assert_eq!(dirs(&bad), vec![root.join("fonts")], "{bad:?}");
        }
        assert_eq!(font_dirs(&root, std::slice::from_ref(&found), &Person::default().mods), vec![root.join("fonts")], "missing settings.json");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clipboard_and_update_switches_round_trip_and_reject_wrong_types() {
        let look = crate::layout::Look::default();
        assert!(!Settings::default().clipboard_write && Settings::default().check_updates);
        // The settings toggle: on, saved, reloaded, off again, saved without the default.
        let p = Person { clipboard_write: true, check_updates: false, ..Person::default() };
        let saved = serde_json::to_string(&p).unwrap();
        let mut back: Person = serde_json::from_str(&saved).unwrap();
        let s = Settings::compose(&back, &look, true);
        assert!(s.clipboard_write && !s.check_updates);
        back.clipboard_write = false;
        back.check_updates = true;
        let value = serde_json::to_value(&back).unwrap();
        assert!(value.get("clipboard_write").is_none() && value.get("check_updates").is_none(), "{value}");
        for bad in [r#"{"check_updates":"no"}"#, r#"{"check_updates":0}"#, r#"{"check_updates":null}"#, r#"{"clipboard_write":null}"#, r#"{"clipboard_write":"true"}"#, r#"{"clipboard_write":1}"#] {
            assert!(serde_json::from_str::<Person>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn shipped_looks_and_theme_clicks_have_the_same_precedence() {
        use crate::layout::{Node, ThemeChoice};
        let person = Person { font_size: Some(19.0), theme: Some(ThemeChoice::One("moss".into())), ..Person::default() };
        let main: Node = serde_json::from_str(include_str!("../defaults/layouts/main.json")).unwrap();
        assert!(main.look.theme.is_none() && main.look.colors.is_none() && main.look.font.is_none() && main.look.font_size.is_none() && main.look.ui_font.is_none());
        assert_eq!(Settings::compose(&Person::default(), &main.look, true).font_size, 14.0);
        assert_eq!(main.look.terminal_padding, Some([14.0, 12.0]));
        let terminals: Node = serde_json::from_str(include_str!("../defaults/layouts/terminals.json")).unwrap();
        assert_eq!(Settings::compose(&person, &terminals.look, true).font_size, 15.0);
        let main_settings = Settings::compose(&person, &main.look, true);
        assert_eq!((main_settings.font_size, main_settings.theme.as_str()), (19.0, "moss"));
        let mut win98: Node = serde_json::from_str(include_str!("../defaults/layouts/win98.json")).unwrap();
        let win_settings = Settings::compose(&person, &win98.look, true);
        assert_eq!(win_settings.theme, "win98");
        assert_eq!(win_settings.font_family, "PxPlus IBM VGA 8x16");
        let clicks = crate::layout_sync::State { theme: Some(ThemeChoice::One("ink".into())), ..Default::default() };
        clicks.apply(&mut win98);
        assert_eq!(Settings::compose(&person, &win98.look, true).theme, "ink");
    }

    /// The guide agents read (written to the home folder) explains every
    /// settings.json key.
    #[test]
    fn the_guide_explains_every_setting() {
        let all = Person { core: Some(String::new()), font: Some(String::new()), font_size: Some(1.0), ui_font: Some(String::new()), clipboard_write: false, appearance: Appearance::Light, theme: Some(crate::layout::ThemeChoice::One(String::new())), colors: [("text".into(), String::new())].into(), mods: [("m".into(), ModChoice::default())].into(), ..Person::default() };
        let keys: Vec<String> = serde_json::to_value(&all).unwrap().as_object().unwrap().keys().cloned().collect();
        let guide = include_str!("../defaults/AGENTS.md");
        let section = guide.split("## The person").nth(1).and_then(|s| s.split("\n## ").next()).unwrap_or_default();
        let missing: Vec<&String> = keys.iter().filter(|k| !section.contains(&format!("`{k}`"))).collect();
        assert!(missing.is_empty(), "defaults/AGENTS.md (## The person) doesn't explain: {missing:?}");
    }

    /// The guide's stated defaults are what an empty settings.json gives, and
    /// every key it documents is one settings.json keeps.
    #[test]
    fn share_analytics_defaults_on_and_round_trips() {
        let old: Person = serde_json::from_str(r#"{"telemetry":false}"#).unwrap();
        assert!(old.share_analytics && old.found_via.is_none());
        assert!(serde_json::to_value(&old).unwrap().get("share_analytics").is_none());
        let off: Person = serde_json::from_str(r#"{"share_analytics":false,"found_via":"skip"}"#).unwrap();
        assert!(!Settings::compose(&off, &crate::layout::Look::default(), true).share_analytics);
        let v = serde_json::to_value(&off).unwrap();
        assert_eq!((&v["share_analytics"], &v["found_via"]), (&serde_json::json!(false), &serde_json::json!("skip")));
        // A wrong type is the default, not a reason to reject the whole file.
        let bad: Person = serde_json::from_str(r#"{"share_analytics":"no","zoom":1.5}"#).unwrap();
        assert!(bad.share_analytics && bad.zoom == 1.5);
    }

    #[test]
    fn install_id_is_made_once_and_remade_only_when_invalid() {
        let dir = std::env::temp_dir().join(format!("h0x-install-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let id = install_id(&dir);
        assert!(id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
        assert_eq!(install_id(&dir), id);
        assert_eq!(std::fs::read_to_string(dir.join("install-id")).unwrap(), id);
        std::fs::write(dir.join("install-id"), "not an id").unwrap();
        let again = install_id(&dir);
        assert!(again != id && again.len() == 32);
        assert_eq!(install_id(&dir), again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn found_via_keeps_any_answer_and_null_or_missing_means_not_yet_asked() {
        for text in ["{}", r#"{"found_via":null}"#] {
            let person: Person = serde_json::from_str(text).unwrap();
            assert!(person.found_via.is_none() && person.share_analytics, "{text}");
            assert!(serde_json::to_value(&person).unwrap().get("found_via").is_none(), "{text}");
        }
        // The app doesn't check the answer: one it doesn't know (or an empty one) is kept as written.
        for answer in ["friend", "x", "github", "search", "other", "skip", "carrier-pigeon", ""] {
            let person: Person = serde_json::from_value(serde_json::json!({"found_via": answer})).unwrap();
            assert_eq!(person.found_via.as_deref(), Some(answer));
            assert_eq!(serde_json::to_value(&person).unwrap()["found_via"], serde_json::json!(answer));
        }
        for wrong in [r#"{"found_via":1}"#, r#"{"found_via":true}"#, r#"{"found_via":["x"]}"#] {
            let person: Person = serde_json::from_str(wrong).unwrap();
            assert!(person.found_via.is_none(), "{wrong}: a wrong type means not yet asked");
        }
        // share_analytics: true is the default, so it isn't written back; null is the default too.
        let on: Person = serde_json::from_str(r#"{"share_analytics":true}"#).unwrap();
        assert!(on.share_analytics && serde_json::to_value(&on).unwrap().get("share_analytics").is_none());
        assert!(serde_json::from_str::<Person>(r#"{"share_analytics":null}"#).unwrap().share_analytics);
        assert!(Person::default().share_analytics && Settings::default().share_analytics);
        for dark in [false, true] {
            assert!(Settings::compose(&Person::default(), &crate::layout::Look::default(), dark).share_analytics);
        }
    }

    #[test]
    fn install_id_keeps_a_valid_id_read_past_whitespace_and_remakes_any_other_file() {
        let dir = std::env::temp_dir().join(format!("h0x-install-id-edges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("install-id");
        let lower_hex = |id: &str| id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        let kept = "0123456789abcdef0123456789abcdef";
        // Kept: whitespace around it (an editor's newline, CRLF) and upper case, read back in lower case.
        // The file is left as it is, except that an upper-case id is rewritten in lower case.
        for text in [
            "0123456789abcdef0123456789abcdef\n",
            "  0123456789abcdef0123456789abcdef \r\n",
            "\t0123456789ABCDEF0123456789abcdef",
            "0123456789ABCDEF0123456789ABCDEF",
        ] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(install_id(&dir), kept, "{text:?}");
            let want = if text.bytes().any(|b| b.is_ascii_uppercase()) { kept } else { text };
            assert_eq!(std::fs::read_to_string(&file).unwrap(), want, "{text:?}");
        }
        // Made again, saved and then reused: empty, one digit short or over, a non-hex digit, a 0x
        // prefix, a space inside, a non-ASCII digit of the same byte length, a BOM, twice the length.
        for text in [
            "",
            "\n",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789abcdef0123456789abcdeg",
            "0x23456789abcdef0123456789abcdef",
            "01234567 9abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcd\u{e9}",
            "\u{feff}0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ] {
            std::fs::write(&file, text).unwrap();
            let id = install_id(&dir);
            assert!(lower_hex(&id) && id != kept, "{text:?}: {id}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), id, "{text:?}: the new id is saved");
            assert_eq!(install_id(&dir), id, "{text:?}: and reused");
        }
        // Not UTF-8 at all.
        std::fs::write(&file, [0xff, 0xfe, 0x00, 0x41]).unwrap();
        let id = install_id(&dir);
        assert!(lower_hex(&id), "{id}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_id_without_a_writable_app_home_still_returns_an_id_the_same_one_within_the_process() {
        let root = std::env::temp_dir().join(format!("h0x-install-id-unwritable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let lower_hex = |id: &str| id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        // The app home would be inside a file, so it can't be made.
        let blocker = root.join("a-file");
        std::fs::write(&blocker, "not a folder").unwrap();
        let home = blocker.join("h0x-ade");
        let (first, second) = (install_id(&home), install_id(&home));
        assert!(lower_hex(&first) && lower_hex(&second), "{first} {second}");
        assert_eq!(first, second, "nothing was saved, so the process keeps one id");
        assert!(!home.exists());
        // install-id is a folder: it can't be read as text or written over.
        let home = root.join("home");
        std::fs::create_dir_all(home.join("install-id")).unwrap();
        assert!(lower_hex(&install_id(&home)));
        assert!(home.join("install-id").is_dir(), "the folder is left alone");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_id_makes_missing_parents_in_an_app_home_with_spaces_and_non_ascii_names() {
        let root = std::env::temp_dir().join(format!("h0x-install-id-spaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Like C:\Users\Zoë Q\AppData\Roaming\h0x-ade: parents that don't exist yet.
        let home = root.join("Zo\u{eb} Q").join("App Data").join("h0x-ade");
        let id = install_id(&home);
        assert_eq!(std::fs::read_to_string(home.join("install-id")).unwrap(), id);
        assert_eq!(install_id(&home), id);
        // The same folder spelled another way (on Windows a \\?\ verbatim path) is the same install.
        assert_eq!(install_id(&std::fs::canonicalize(&home).unwrap()), id);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_guide_states_the_real_clipboard_write_and_check_updates_defaults() {
        let guide = include_str!("../defaults/AGENTS.md");
        let entry = |key: &str| {
            let start = guide.find(&format!("- `{key}`:")).unwrap_or_else(|| panic!("`{key}` is not documented"));
            let rest = &guide[start + 3..];
            rest[..rest.find("\n- ").unwrap_or(rest.len())].to_owned()
        };
        let clipboard = serde_json::from_str::<Person>("{}").unwrap().clipboard_write;
        assert!(entry("clipboard_write").contains(&format!("(default `{clipboard}`")), "guide: {:?}, code default: {clipboard}", entry("clipboard_write"));
        assert!(entry("check_updates").contains("(default `true`)"), "{:?}", entry("check_updates"));
        let off = serde_json::to_value(serde_json::from_str::<Person>(r#"{"check_updates":false}"#).unwrap()).unwrap();
        assert_eq!(off.get("check_updates"), Some(&serde_json::json!(false)), "settings.json drops the documented check_updates key: {off}");
    }

    // The six colour themes (TH-1): (id, light, primary, background), the hex values converted
    // here from .cache/reports/THEMES-SOURCE.md's oklch (primary and background tokens).
    const NEW_THEMES: [(&str, bool, u32, u32); 6] = [
        ("amber-minimal", false, 0xf59e0b, 0x171717),
        ("candyland", false, 0xff99cc, 0x1a1d23),
        ("claymorphism", false, 0x818cf8, 0x1e1b18),
        ("darkmatter", false, 0xe78a53, 0x121113),
        ("soft-pop", true, 0x4f46e5, 0xf7f9f3),
        ("neo-brutalism", true, 0xff3333, 0xffffff),
    ];

    // A shipped theme as the app shows it: parsed, then resolved with its `colors`.
    fn shipped(name: &str) -> (Palette, Theme) {
        let text = THEMES.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} is not shipped")).1;
        let (p, o) = parse_theme_file(name, text).unwrap_or_else(|e| panic!("{e}"));
        let t = resolve(&p, &o);
        (p, t)
    }

    #[test]
    fn the_six_colour_themes_ship_beside_the_existing_presets_and_themes() {
        for (name, text) in THEMES {
            parse_theme_file(name, text).unwrap_or_else(|e| panic!("{e}"));
        }
        let names: Vec<&str> = THEMES.iter().map(|(n, _)| *n).collect();
        for want in ["atom-one-dark", "win98", "amber-minimal", "candyland", "claymorphism", "darkmatter", "soft-pop", "neo-brutalism"] {
            assert!(names.contains(&want), "{want} is not shipped: {names:?}");
        }
        assert_eq!(names.len(), 8, "{names:?}");
        assert_eq!(PRESETS, ["graphite", "ink", "moss", "paper"]);
        for name in PRESETS {
            assert!(preset(name).is_some(), "{name}");
            assert!(!names.contains(name), "a theme file never shadows the preset {name}");
        }
        // Ids are lowercase-dashed: the Look picker shows "amber minimal" for amber-minimal.
        for (name, ..) in NEW_THEMES {
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !name.starts_with('-') && !name.ends_with('-'), "{name}");
        }
    }

    #[test]
    fn the_new_themes_are_dark_or_light_as_designed_and_presets_keep_their_side() {
        // theme_is_light and the Look picker's Light/Dark sections go by is_light(background).
        for (name, light, ..) in NEW_THEMES {
            let (p, _) = shipped(name);
            assert_eq!(is_light(p.background), light, "{name}: background {}", hex(p.background));
        }
        for (name, light) in [("graphite", false), ("ink", false), ("moss", false), ("paper", true)] {
            assert_eq!(is_light(preset(name).unwrap().background), light, "{name}");
        }
        assert!(!theme_is_light("graphite"));
        assert!(theme_is_light("paper"));
        // An unknown name is never light (and never panics).
        assert!(!theme_is_light("no-such-theme-th-t") && !theme_exists("no-such-theme-th-t"));
        assert!(!theme_is_light(""));
    }

    #[test]
    fn the_new_themes_keep_terminal_and_panel_text_readable() {
        fn channel(c: u32, shift: u32) -> f64 {
            let v = ((c >> shift) & 255) as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        }
        fn luminance(c: u32) -> f64 {
            0.2126 * channel(c, 16) + 0.7152 * channel(c, 8) + 0.0722 * channel(c, 0)
        }
        fn contrast(a: u32, b: u32) -> f64 {
            let (x, y) = (luminance(a), luminance(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        let rgb = |t: &Theme, k: &str| {
            let v = t.get(k).unwrap_or_else(|| panic!("no {k}"));
            assert!(v.len() == 7 && v.starts_with('#'), "{k} {v} is not #rrggbb");
            u32::from_str_radix(&v[1..], 16).unwrap()
        };
        // The formula itself: black on white is 21:1, a color on itself 1:1.
        assert!((contrast(0x000000, 0xffffff) - 21.0).abs() < 1e-9);
        assert!((contrast(0x777777, 0x777777) - 1.0).abs() < 1e-9);
        for (name, ..) in NEW_THEMES {
            let (_, t) = shipped(name);
            let bg = rgb(&t, "background");
            for key in ["foreground", "terminal_foreground"] {
                let ratio = contrast(rgb(&t, key), bg);
                assert!(ratio >= 7.0, "{name}: {key} on background is {ratio:.2}:1, under 7:1");
            }
            for i in (1..=7).chain(9..=15) {
                let ratio = contrast(rgb(&t, &format!("ansi{i}")), bg);
                assert!(ratio >= 3.0, "{name}: ansi{i} {} on {} is {ratio:.2}:1, under 3:1", t[&format!("ansi{i}")], t["background"]);
            }
        }
    }

    #[test]
    fn the_new_themes_keep_their_source_primary_and_background() {
        let near = |got: &str, want: u32| {
            let got = u32::from_str_radix(got.trim_start_matches('#').get(..6).unwrap_or(""), 16).unwrap_or_else(|_| panic!("{got} is not hex"));
            [16, 8, 0].iter().all(|s| (((got >> s) & 255) as i32 - ((want >> s) & 255) as i32).abs() <= 2)
        };
        for (name, _, primary, background) in NEW_THEMES {
            let (p, t) = shipped(name);
            for key in ["accent", "cursor"] {
                assert!(near(&t[key], primary), "{name}: {key} {} is not the source primary #{primary:06x} (±2)", t[key]);
            }
            assert!(near(&t["background"], background), "{name}: background {} is not #{background:06x}", t["background"]);
            assert!(near(&hex(p.background), background), "{name}: the terminal background {} is not #{background:06x}", hex(p.background));
        }
        // ±2 is a real bound: 3 off in one channel is not the source.
        assert!(!near("#f5a10b", 0xf59e0b) && near("#f7a00d", 0xf59e0b));
    }

    #[test]
    fn a_broken_copy_of_a_new_theme_is_refused_with_the_bad_key_named() {
        let text = THEMES.iter().find(|(n, _)| *n == "neo-brutalism").unwrap().1;
        let file: serde_json::Value = serde_json::from_str(text).unwrap();
        let broken = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut f = file.clone();
            change(&mut f);
            parse_theme_file("neo-brutalism", &f.to_string()).err().unwrap_or_else(|| panic!("accepted {f}"))
        };
        assert!(broken(&|f| f["cursor"] = "#ff333".into()).contains("cursor"));
        assert!(broken(&|f| f["background"] = "".into()).contains("background"));
        assert!(broken(&|f| { f["ansi"].as_array_mut().unwrap().pop(); }).contains("neo-brutalism"));
        assert!(broken(&|f| f["colors"]["accent"] = "red".into()).contains("colors.accent"));
        assert!(broken(&|f| f["radius"] = 0.into()).contains("radius"), "unknown keys are refused, not ignored");
        assert!(parse_theme_file("neo-brutalism", "").is_err());
        assert!(parse_theme_file("neo-brutalism", "null").is_err());
    }
}
