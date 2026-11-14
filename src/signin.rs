//! The sign-in screen: shown instead of the main window until this computer has signed in or
//! skipped once (auth.rs keeps the marker). The browser does the signing in; this window only
//! waits for it. Then the main window opens.
//!
//! The look is cook/SPEC.md section 10: the app theme, a frosted card, a dot-matrix mark and a
//! calm band of falling code at the bottom. Colors are theme tokens (`settings::color`) with alpha.

use crate::mods::Request;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The HyperOx mark, the same asset the workspace chrome ships.
const MARK: &[u8] = include_bytes!("../defaults/mods/workspace-chrome/hyperox-mark.svg");
// Lucide, stroke 1.5 (cook/SPEC.md section 9).
// Duotone icons (cook/SPEC-v2.md section 5): a soft fill layer and a line layer, painted in two tints.
const TOKENS_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><rect x="3" y="4" width="18" height="4.5" rx="2.25"/></svg>"##;
const TOKENS_LINE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="4.5" rx="2.25"/><rect x="5.5" y="10.25" width="13" height="4.5" rx="2.25"/><rect x="8.5" y="16.5" width="7" height="4.5" rx="2.25"/></svg>"##;
const OBSERVE_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><rect x="3" y="4.5" width="18" height="15" rx="3.5"/></svg>"##;
const OBSERVE_LINE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4.5" width="18" height="15" rx="3.5"/><path d="M6.5 14h2.5l1.75-4 2.5 7 1.75-3H17.5"/><circle cx="17.5" cy="14" r="1.25" fill="#000" stroke="none"/></svg>"##;
const PII_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><path d="M12 3 5 5.8v5.4c0 4.6 2.9 8.1 7 9.8 4.1-1.7 7-5.2 7-9.8V5.8z"/></svg>"##;
const PII_LINE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 5 5.8v5.4c0 4.6 2.9 8.1 7 9.8 4.1-1.7 7-5.2 7-9.8V5.8z"/><path d="M9.5 15.5h5"/><circle cx="9" cy="11.5" r="1.1" fill="#000" stroke="none"/><circle cx="12" cy="11.5" r="1.1" fill="#000" stroke="none"/><circle cx="15" cy="11.5" r="1.1" fill="#000" stroke="none"/></svg>"##;
const GLOBE_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><circle cx="12" cy="12" r="9"/></svg>"##;
const GLOBE_LINE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3c2.6 2.5 3.9 5.5 3.9 9s-1.3 6.5-3.9 9c-2.6-2.5-3.9-5.5-3.9-9S9.4 5.5 12 3z"/></svg>"##;
const CHAIN_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><path d="M8 3.5 13 6.3v5.6L8 14.7 3 11.9V6.3z"/></svg>"##;
const CHAIN_LINE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3.5 13 6.3v5.6L8 14.7 3 11.9V6.3z"/><path d="M3 6.3 8 9.1l5-2.8M8 9.1v5.6"/><path d="M16 11.2 21 14v5.6l-5 2.8-5-2.8V17"/><path d="M16 16.8l5-2.8M16 16.8v5.6"/></svg>"##;
const OPEN_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/></svg>"##;
const ARROW_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M7 7h10v10"/><path d="M7 17 17 7"/></svg>"##;
const ALERT_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 8v4.5"/><circle cx="12" cy="16" r="1" fill="#000" stroke="none"/></svg>"##;
/// An arc of 270 degrees.
const SPIN_ARC: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 1 1-9 9"/></svg>"##;

const TAGLINE: &str = "Your one-stop ADE for development, security and observability.";
const SIGN_IN: &str = "Sign in with your browser";
const TRY_AGAIN: &str = "Try again";
const PERKS_LEAD: &str = "Signing in turns on";
/// The line layer of each perk's icon and its name; `PERK_FILLS` holds the fill layers, `PERK_IDS` the tile ids.
const PERKS: [(&[u8], &str); 3] = [(TOKENS_LINE, "Token saving"), (OBSERVE_LINE, "Observability engine"), (PII_LINE, "Blockchain security for PII masking")];
const PERK_FILLS: [&[u8]; 3] = [TOKENS_FILL, OBSERVE_FILL, PII_FILL];
const PERK_IDS: [&str; 3] = ["feat-tokens", "feat-observe", "feat-pii"];
/// One line under each perk (the owner has seen them), in the same order.
const BENEFITS: [&str; 3] =
    ["Leaner prompts, so agent sessions use fewer tokens.", "See what each agent ran, changed and spent.", "Personal data is masked before it leaves this computer."];
const SKIP: &str = "Skip sign-in";
const SKIP_NOTE: [&str; 2] = ["Skipping turns these off.", "Sign in later in Settings \u{2192} Profile."];
const SIGN_IN_NOTE: &str = "Opens pavii.tech. Use Google, GitHub or email.";
const WAITING_TITLE: &str = "Finish signing in using your browser";
const WAITING_BODY: &str = "Then come back to this window.";
const TEAM_URL: &str = "https://pavii.tech";
const PARTNER_URL: &str = "https://buckleson.com";
const TEAM_DOMAIN: &str = "pavii.tech";
const TEAM_NOTE: &str = "Made by Team Pavii";
const PARTNER_DOMAIN: &str = "buckleson.com";
const PARTNER_NOTE: &str = "Blockchain & security partner";

const CANCELLED: &str = "Sign-in was cancelled.";
const TIMEOUT: &str = "Sign-in took too long. Please try again.";
const REJECTED: &str = "This account can't sign in right now (it may be disabled). Try another account.";
const OTHER: &str = "Something went wrong while signing in. Please try again.";
const UNREACHABLE: &str = "Couldn't reach Pavii. Check your connection and try again.";

/// Whether the sign-in screen comes first: only on a computer that has never signed in and
/// never skipped. Test and debug builds can also skip it with `H0X_ADE_SKIP_SIGNIN=1`; that
/// bypass is kept by user decision, although the e2e tests now write the marker instead.
pub fn required() -> bool {
    decide(bypass(), || !crate::auth::screen_needed())
}

fn bypass() -> bool {
    cfg!(any(test, debug_assertions)) && std::env::var_os("H0X_ADE_SKIP_SIGNIN").is_some_and(|v| v == "1")
}

/// The marker is only looked up when there is no bypass; `done` is "signed in or skipped".
fn decide(bypass: bool, done: impl FnOnce() -> bool) -> bool {
    !bypass && !done()
}

/// What to tell the person about a failed sign-in.
fn friendly(error: &str) -> &'static str {
    if error == "cancelled" {
        CANCELLED
    } else if error == "timeout" {
        TIMEOUT
    } else if error.starts_with("network:") {
        UNREACHABLE
    } else if error.starts_with("rejected:") {
        REJECTED
    } else {
        OTHER
    }
}

#[derive(Clone, Copy)]
enum State {
    Idle,
    Waiting,
    Failed(&'static str),
}

pub struct SignIn {
    state: State,
    /// Bumped by every attempt and by Cancel: a result from an older attempt is ignored.
    attempt: u64,
    core: Option<Arc<crate::core_link::Core>>,
    requests: Option<async_channel::Receiver<Request>>,
    focus: FocusHandle,
    skip_focus: FocusHandle,
    /// Code-texture ticks since the window opened (it only moves while `rain_runs`).
    frame: u64,
    rain: Rc<RefCell<RainPaint>>,
}

/// The window height: 760, or less on a short screen (visible height minus 80, never below 460).
fn window_height(visible: Option<f32>) -> f32 {
    visible.filter(|h| !h.is_nan()).map_or(760.0, |h| (h - 80.0).clamp(460.0, 760.0))
}

/// The window width: 1040, or less on a narrow screen (visible width minus 80, never below 620).
/// From 900 wide and 680 high the screen shows the regular grid, below that the compact one.
fn window_width(visible: Option<f32>) -> f32 {
    visible.filter(|w| !w.is_nan()).map_or(1040.0, |w| (w - 80.0).clamp(620.0, 1040.0))
}

/// The window's size on a screen with `visible` room (width, height).
fn window_size(visible: Option<(f32, f32)>) -> Size<Pixels> {
    size(px(window_width(visible.map(|v| v.0))), px(window_height(visible.map(|v| v.1))))
}

/// Not resizable: that also takes away the maximize box, the double-click and Win+Up maximize.
/// The minimum size only matters if that ever changes.
fn window_options(bounds: WindowBounds) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(620.0), px(460.0))),
        is_resizable: false,
        is_minimizable: true,
        titlebar: Some(TitlebarOptions { title: Some("HyperOx".into()), ..Default::default() }),
        app_id: Some("h0x-ade".into()),
        ..Default::default()
    }
}

/// The settings the main window will start with, as far as the theme goes: settings.json's
/// appearance (the system's when it says so), then its light or dark theme, else darkmatter or paper.
fn theme_settings(person: &crate::settings::Person, system_dark: bool) -> crate::settings::Settings {
    crate::settings::Settings::compose(person, &crate::layout::Look::default(), system_dark)
}

pub fn open(core: Arc<crate::core_link::Core>, requests: async_channel::Receiver<Request>, cx: &mut App) {
    // This screen opens before the main window loads settings and themes: until then the colors are the built-in graphite.
    let person = crate::settings::load_person().unwrap_or_default();
    let system_dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    crate::settings::set(theme_settings(&person, system_dark));
    let visible = cx.primary_display().map(|d| (f32::from(d.visible_bounds().size.width), f32::from(d.visible_bounds().size.height)));
    // The operating system's reduced-motion setting, for the code texture.
    gpui_base::apply_system_reduce_motion(cx);
    let options = window_options(WindowBounds::Windowed(Bounds::centered(None, window_size(visible), cx)));
    cx.open_window(options, |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let skip_focus = cx.focus_handle().tab_stop(true);
        let view = cx.new(|_| SignIn {
            state: State::Idle,
            attempt: 0,
            core: Some(core),
            requests: Some(requests),
            focus,
            skip_focus,
            frame: 0,
            rain: Rc::new(RefCell::new(RainPaint::default())),
        });
        view.update(cx, |_, cx| {
            // One timer for the texture: it only repaints while the window is in front and motion is allowed.
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let open = this.update_in(cx, |this, window, cx| {
                        let wide = layout(f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)).rain;
                        if rain_runs(cx.reduce_motion(), window.is_window_active(), wide) {
                            this.frame += 1;
                            cx.notify();
                        }
                    });
                    if open.is_err() {
                        break;
                    }
                }
            })
            .detach();
        });
        view
    })
    .expect("cannot open the HyperOx window");
    cx.activate(true);
}

impl Drop for SignIn {
    /// Closing the window while the browser is still being waited for stops the wait.
    fn drop(&mut self) {
        if matches!(self.state, State::Waiting) {
            crate::auth::cancel_sign_in();
        }
    }
}

impl SignIn {
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Enter and a click can arrive together: one attempt at a time.
        if matches!(self.state, State::Waiting) || self.core.is_none() {
            return;
        }
        self.attempt += 1;
        let attempt = self.attempt;
        self.state = State::Waiting;
        // The button that was clicked is gone now: the keys (Esc, Tab) go to the screen again.
        window.focus(&self.focus, cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::auth::sign_in(|url| crate::links::open(&crate::links::Target::Url(url.to_string()))) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.attempt != attempt {
                    return;
                }
                match result {
                    Ok(account) => {
                        crate::analytics::identify(Some(&account));
                        let (Some(core), Some(requests)) = (this.core.take(), this.requests.take()) else { return };
                        // Done: closing the window must not cancel anything.
                        this.state = State::Idle;
                        window.remove_window();
                        crate::open_main(core, requests, cx);
                    }
                    Err(e) => {
                        this.state = State::Failed(friendly(&e));
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Opens the main window signed out, the way a successful sign-in does (minus `identify`).
    fn skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A browser sign-in in flight must finish or be cancelled first.
        if matches!(self.state, State::Waiting) {
            return;
        }
        let (Some(core), Some(requests)) = (self.core.take(), self.requests.take()) else { return };
        if let Err(e) = crate::auth::skip_screen() {
            tracing::warn!("sign-in skip not remembered: {e}");
        }
        crate::analytics::event("signin_skipped", serde_json::json!({}));
        window.remove_window();
        crate::open_main(core, requests, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Only a wait can be cancelled (Esc is also pressed in the other states).
        if !matches!(self.state, State::Waiting) {
            return;
        }
        crate::auth::cancel_sign_in();
        self.attempt += 1;
        self.state = State::Failed(CANCELLED);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "enter" => match enter_action(self.state, self.skip_focus.is_focused(window)) {
                Action::Skip => self.skip(window, cx),
                // A focused button or link activates itself; only the bare root signs in.
                Action::Start if self.focus.is_focused(window) => self.start(window, cx),
                Action::Start | Action::None => {}
            },
            "escape" => self.cancel(window, cx),
            "tab" if ev.keystroke.modifiers.shift => window.focus_prev(cx),
            "tab" => window.focus_next(cx),
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Start,
    Skip,
    None,
}

/// What Enter does: nothing while waiting, Skip when Skip has focus, else sign in.
fn enter_action(state: State, skip_focused: bool) -> Action {
    match state {
        State::Waiting => Action::None,
        _ if skip_focused => Action::Skip,
        _ => Action::Start,
    }
}

/// What the card shows for a state: which status line, buttons and note.
#[derive(Debug, PartialEq)]
struct Content {
    status: Option<Status>,
    primary: Option<&'static str>,
    secondary: Option<&'static str>,
    note: Option<&'static str>,
    skip: bool,
}

#[derive(Debug, PartialEq)]
enum Status {
    Waiting,
    Error(&'static str),
}

fn content(state: State) -> Content {
    match state {
        State::Idle => Content { status: None, primary: Some(SIGN_IN), secondary: None, note: Some(SIGN_IN_NOTE), skip: true },
        State::Waiting => Content { status: Some(Status::Waiting), primary: None, secondary: Some("Cancel"), note: None, skip: false },
        State::Failed(e) => Content { status: Some(Status::Error(e)), primary: Some(TRY_AGAIN), secondary: None, note: None, skip: true },
    }
}

/// The footer's key hints for a state: (key, what it does); Tab always moves focus.
fn hints(state: State) -> [(&'static str, &'static str); 2] {
    let first = match state {
        State::Idle => ("Enter", "sign in"),
        State::Waiting => ("Esc", "cancel"),
        State::Failed(_) => ("Enter", "try again"),
    };
    [first, ("Tab", "focus")]
}

// ---- layout ---------------------------------------------------------------------------------

/// The grid of tiles at a window size (cook/SPEC-v2.md section 1).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// 3 columns of 288, the hero over two of them.
    Regular,
    /// One narrow column of tiles, no mark and no code band.
    Compact,
}

#[derive(Debug, PartialEq)]
struct Layout {
    mode: Mode,
    /// Every size, radius, font size and gap is multiplied by this (1.12 from 1600 x 950).
    scale: f32,
    /// Height of the code band.
    band: f32,
    /// The code texture draws only in the regular layout: in the compact one the tiles cover it.
    rain: bool,
}

fn layout(width: f32, height: f32) -> Layout {
    let mode = if width >= 900.0 && height >= 680.0 { Mode::Regular } else { Mode::Compact };
    let k = if height <= 700.0 { 0.40 } else { 0.24 };
    Layout {
        mode,
        scale: if mode == Mode::Regular && width >= 1600.0 && height >= 950.0 { 1.12 } else { 1.0 },
        band: ((height - 44.0) * k).round().clamp(120.0, 260.0),
        rain: mode == Mode::Regular,
    }
}

/// The regular grid, in px at scale 1: three columns, the hero 312 high, the lead line, the feature row.
const COL: f32 = 288.0;
const GAP: f32 = 12.0;
const HERO_H: f32 = 312.0;
const LEAD_H: f32 = 16.0;
const FEATURE_H: f32 = 148.0;
const GRID_W: f32 = 3.0 * COL + 2.0 * GAP;
const GRID_H: f32 = HERO_H + GAP + LEAD_H + GAP + FEATURE_H;

/// The five tiles of the regular grid in window coordinates as (left, top, right, bottom): hero, mark,
/// then the three features. The grid is centred, 3% of the window height below the middle.
fn tile_rects(width: f32, height: f32, scale: f32) -> [(f32, f32, f32, f32); 5] {
    let (x0, y0) = ((width - GRID_W * scale) / 2.0, height / 2.0 + 0.03 * height - GRID_H * scale / 2.0);
    let rect = |col: f32, row_top: f32, cols: f32, h: f32| {
        let (left, top) = (x0 + col * (COL + GAP) * scale, y0 + row_top * scale);
        (left, top, left + (cols * COL + (cols - 1.0) * GAP) * scale, top + h * scale)
    };
    let features = HERO_H + GAP + LEAD_H + GAP;
    [rect(0.0, 0.0, 2.0, HERO_H), rect(2.0, 0.0, 1.0, HERO_H), rect(0.0, features, 1.0, FEATURE_H), rect(1.0, features, 1.0, FEATURE_H), rect(2.0, features, 1.0, FEATURE_H)]
}

/// The compact grid's width: 712, or the window less 16 on each side.
fn compact_width(window_width: f32) -> f32 {
    712.0_f32.min(window_width - 32.0)
}

// ---- the code texture ------------------------------------------------------------------------

/// One step of the texture, in time (about 9 frames a second).
const TICK: Duration = Duration::from_millis(110);
const GLYPHS: &[u8] = b"0123456789abcdef{}[]<>()=+-*/;:$#&|_.";
const RAIN_SEED: u32 = 0x48304f58;
const CELL_H: f32 = 14.0;
const GLYPH_SIZE: f32 = 11.0;

/// Whether the texture moves: motion is allowed, the window is in front and the texture shows at all.
/// Otherwise it stays on its first frame (the spinner still turns: it shows progress).
fn rain_runs(reduce_motion: bool, window_active: bool, shown: bool) -> bool {
    shown && window_active && !reduce_motion
}

/// The frame to draw: the ticked one while the texture runs, else the first.
fn shown_frame(frame: u64, animating: bool) -> u64 {
    if animating { frame } else { 0 }
}

/// mulberry32, as the mock-up's `rnd`.
struct Mulberry(u32);

impl Mulberry {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6D2B79F5);
        let s = self.0;
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        t ^ (t >> 14)
    }

    fn next(&mut self) -> f64 {
        self.next_u32() as f64 / 4294967296.0
    }
}

/// A falling column: where its two heads start, how long a trail is, how often it steps.
#[derive(Debug, PartialEq)]
struct Column {
    h1: i64,
    h2: i64,
    len: i64,
    every: i64,
    glyph: Vec<u8>,
    noise: Vec<bool>,
}

/// The texture's state: built once from a fixed seed, so every start looks the same.
#[derive(Debug, PartialEq)]
struct Rain {
    cols: Vec<Column>,
    rows: usize,
}

impl Rain {
    fn new(cols: usize, rows: usize) -> Rain {
        let mut r = Mulberry(RAIN_SEED);
        let cols = (0..cols)
            .map(|_| {
                let h1 = (r.next() * rows as f64 * 1.3).floor() as i64;
                let h2 = (r.next() * rows as f64 * 1.3).floor() as i64 - rows as i64;
                let len = 10 + (r.next() * 8.0).floor() as i64;
                let every = 1 + (r.next() * 3.0).floor() as i64;
                let glyph = (0..rows).map(|_| (r.next() * GLYPHS.len() as f64).floor() as u8).collect();
                let noise = (0..rows).map(|_| r.next() < 0.06).collect();
                Column { h1, h2, len, every, glyph, noise }
            })
            .collect();
        Rain { cols, rows }
    }

    /// The glyph (index into `GLYPHS`) and its strength at a cell on a frame, None when it is blank.
    /// Rows count down from the top of the band; a head shows a new glyph each step, a trail keeps its own.
    fn cell(&self, col: usize, row: usize, frame: u64) -> Option<(usize, f32)> {
        let c = self.cols.get(col)?;
        let step = (frame / c.every as u64) as i64;
        let cycle = self.rows as i64 + 18;
        let mut strength = 0.0_f32;
        let mut head = false;
        for h in [(c.h1 + step).rem_euclid(cycle), (c.h2 + step).rem_euclid(cycle)] {
            let d = h - row as i64;
            if d == 0 {
                strength = strength.max(0.26);
                head = true;
            } else if d > 0 && d < c.len {
                strength = strength.max(0.15 * (1.0 - d as f32 / c.len as f32) + 0.04);
            }
        }
        if strength == 0.0 && *c.noise.get(row)? {
            strength = 0.05;
        }
        if strength == 0.0 {
            return None;
        }
        let glyph = *c.glyph.get(row)? as i64;
        Some((if head { (glyph + step) as usize % GLYPHS.len() } else { glyph as usize }, strength))
    }
}

/// How much of a glyph shows: faded toward the top of the band, and away from the card.
/// `y` is the cell's top in the band, `distance` the gap from its box to the card.
fn cell_weight(y: f32, band: f32, distance: f32, light: bool) -> f32 {
    let t = (y / (band * 0.75)).min(1.0);
    let fade = t * t * (3.0 - 2.0 * t);
    fade * if light { 0.55 } else { 1.0 } * ((distance - 6.0) / 28.0).clamp(0.0, 1.0)
}

/// The distance between a cell's box and the card's box (0 when they overlap).
fn card_distance(cell: (f32, f32, f32, f32), card: (f32, f32, f32, f32)) -> f32 {
    let (x, y, w, h) = cell;
    let (left, top, right, bottom) = card;
    let dx = (left - (x + w)).max(0.0).max(x - right);
    let dy = (top - (y + h)).max(0.0).max(y - bottom);
    dx.hypot(dy)
}

/// Shaped glyphs, kept between frames: a glyph at a strength rounded to a hundredth.
#[derive(Default)]
struct RainPaint {
    rain: Option<((usize, usize), Rain)>,
    lines: HashMap<(u8, u8), ShapedLine>,
    cell_w: Option<f32>,
}

fn paint_rain(state: &Rc<RefCell<RainPaint>>, tiles: &[(f32, f32, f32, f32)], light: bool, frame: u64, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let mut guard = state.borrow_mut();
    let s = &mut *guard;
    let size_px = px(GLYPH_SIZE);
    let font = font(crate::settings::DEFAULT_FONT);
    let color = crate::settings::color("muted");
    let run = |color: Hsla| TextRun { len: 1, font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
    let cell_w = *s.cell_w.get_or_insert_with(|| f32::from(window.text_system().shape_line("0".into(), size_px, &[run(color)], None).width).ceil() + 2.0);
    let (width, band) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (cols, rows) = ((width / cell_w).ceil() as usize, (band / CELL_H).ceil() as usize);
    if s.rain.as_ref().is_none_or(|(k, _)| *k != (cols, rows)) {
        s.rain = Some(((cols, rows), Rain::new(cols, rows)));
    }
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    for c in 0..cols {
        for r in 0..rows {
            let Some((glyph, strength)) = s.rain.as_ref().and_then(|(_, rain)| rain.cell(c, r, frame)) else { continue };
            let (x, y) = (c as f32 * cell_w + 1.0, r as f32 * CELL_H);
            let distance = tiles.iter().map(|tile| card_distance((left + x, top + y, cell_w, CELL_H), *tile)).fold(f32::MAX, f32::min);
            let alpha = strength * cell_weight(y, band, distance, light);
            let level = (alpha * 100.0).round() as u8;
            if level == 0 {
                continue;
            }
            let line = s.lines.entry((glyph as u8, level)).or_insert_with(|| {
                let text = (GLYPHS[glyph] as char).to_string();
                window.text_system().shape_line(text.into(), size_px, &[run(color.opacity(level as f32 / 100.0))], None)
            });
            let _ = line.paint(point(bounds.origin.x + px(x), bounds.origin.y + px(y)), px(CELL_H), TextAlign::Left, None, window, cx);
        }
    }
}

// ---- the dot-matrix mark ---------------------------------------------------------------------

const DOT_BOX: (f32, f32) = (120.0, 141.0);
const DOT_STEP: f32 = 5.0;
const DOT_RADIUS: f32 = 1.6;

/// The centres of the dots that draw the mark: its outline (read from the shipped SVG, straight
/// lines only) sampled on a 5 px grid in a 120 x 141 box, every point inside it (even-odd).
fn mark_dots() -> &'static [(f32, f32)] {
    static DOTS: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
    DOTS.get_or_init(|| dots_in(std::str::from_utf8(MARK).unwrap_or_default()))
}

fn dots_in(svg: &str) -> Vec<(f32, f32)> {
    let Some(d) = svg.split_once(" d=\"").and_then(|(_, rest)| rest.split_once('"')).map(|(d, _)| d) else { return Vec::new() };
    // The mark's own coordinate box (its viewBox).
    let (vw, vh) = (715.0_f32, 842.0_f32);
    let mut shapes: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut numbers: Vec<f32> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let flush = |numbers: &mut Vec<f32>, current: &mut Vec<(f32, f32)>| {
        current.extend(numbers.chunks_exact(2).map(|p| (p[0] * DOT_BOX.0 / vw, p[1] * DOT_BOX.1 / vh)));
        numbers.clear();
    };
    let mut token = String::new();
    for ch in d.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || ch == '.' || ch == '-' {
            token.push(ch);
            continue;
        }
        if let Ok(n) = token.parse::<f32>() {
            numbers.push(n);
        }
        token.clear();
        match ch {
            'M' | 'Z' | 'z' => {
                flush(&mut numbers, &mut current);
                if !current.is_empty() {
                    shapes.push(std::mem::take(&mut current));
                }
            }
            _ => {}
        }
    }
    let inside = |x: f32, y: f32| {
        let mut crossings = 0;
        for shape in &shapes {
            for (i, a) in shape.iter().enumerate() {
                let b = shape[(i + 1) % shape.len()];
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                    crossings += 1;
                }
            }
        }
        crossings % 2 == 1
    };
    let mut dots = Vec::new();
    let mut y = DOT_STEP / 2.0;
    while y < DOT_BOX.1 {
        let mut x = DOT_STEP / 2.0;
        while x < DOT_BOX.0 {
            // As the mock-up reads its raster: the pixel holding the point must be over a third covered.
            let (px0, py0) = (x.floor(), y.floor());
            let covered = (0..16).filter(|k| inside(px0 + ((k % 4) as f32 + 0.5) / 4.0, py0 + ((k / 4) as f32 + 0.5) / 4.0)).count();
            if covered as f32 / 16.0 > 0.35 {
                dots.push((x, y));
            }
            x += DOT_STEP;
        }
        y += DOT_STEP;
    }
    dots
}

// ---- elements --------------------------------------------------------------------------------

fn c(name: &str) -> Hsla {
    crate::settings::color(name)
}

fn light() -> bool {
    crate::settings::theme_is_light(&crate::settings::get().theme)
}

/// The decorative color: the accent on dark themes, the ink on light ones.
fn tint() -> Hsla {
    if light() { c("foreground") } else { c("accent") }
}

/// Outer 2px `ring` band (4px out) with a 2px `background` gap against the element; later shadows paint on top.
fn ring() -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.0), px(0.0), c("ring")).spread_radius(px(4.0)), BoxShadow::new(px(0.0), px(0.0), c("background")).spread_radius(px(2.0))]
}

/// A size at the layout's scale.
fn z(v: f32, s: f32) -> Pixels {
    px(v * s)
}

/// A veil over the glass: white at `dark` on dark themes, black at `lite` on light ones.
fn veil(light: bool, dark: f32, lite: f32) -> Hsla {
    if light { black().opacity(lite) } else { white().opacity(dark) }
}

/// A layer filling its parent, with the parent's corners.
fn inset(radius: Pixels) -> Div {
    div().absolute().top(px(0.0)).left(px(0.0)).right(px(0.0)).bottom(px(0.0)).rounded(radius)
}

/// A glass tile: no backdrop blur in GPUI, so a translucent fill over the wash and the code band, a sheen,
/// a hairline edge, a highlight along the top and a soft drop shadow (cook/SPEC-v2.md section 2).
fn glass(id: &'static str, radius: f32, s: f32, light: bool) -> Stateful<Div> {
    let r = z(radius, s);
    let mut tile = div().id(id).relative().rounded(r).border_1().border_color(veil(light, 0.10, 0.07));
    tile.style().box_shadow = Some(if light {
        vec![
            BoxShadow::new(px(0.0), z(24.0, s), black().opacity(0.20)).blur_radius(z(60.0, s)).spread_radius(z(-28.0, s)),
            BoxShadow::new(px(0.0), z(1.0, s), black().opacity(0.06)).blur_radius(z(2.0, s)),
        ]
    } else {
        vec![
            BoxShadow::new(px(0.0), z(24.0, s), black().opacity(0.70)).blur_radius(z(60.0, s)).spread_radius(z(-24.0, s)),
            BoxShadow::new(px(0.0), z(1.0, s), black().opacity(0.30)).blur_radius(z(2.0, s)),
        ]
    });
    let (top, bottom) = if light { (0.80, 0.55) } else { (0.075, 0.030) };
    tile.when(!light, |d| d.child(inset(r).bg(c("raised").opacity(0.55))))
        .child(inset(r).bg(linear_gradient(180.0, linear_color_stop(white().opacity(top), 0.0), linear_color_stop(white().opacity(bottom), 1.0))))
        .child(div().absolute().top(px(0.0)).left(r).right(r).h(px(1.0)).bg(if light { white() } else { white().opacity(0.20) }))
}

/// A duotone icon: its soft fill layer under its line layer, one tint at two strengths.
fn duo(fill: &'static [u8], line: &'static [u8], size: Pixels, color: Hsla) -> Div {
    div()
        .relative()
        .flex_none()
        .size(size)
        .child(svg().data(fill).absolute().size_full().text_color(color.opacity(0.18)))
        .child(svg().data(line).absolute().size_full().text_color(color))
}

fn open_url(url: &'static str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, _| crate::links::open(&crate::links::Target::Url(url.to_string()))
}

/// The main action: a solid button with a lit top edge and a soft lift, and the focus ring while it has focus
/// (not `focus_visible`: the ring must not hide after a mouse move).
fn primary_button(id: &'static str, label: &'static str, s: f32) -> Stateful<Div> {
    let lift = BoxShadow::new(px(0.0), z(8.0, s), c("solid").opacity(0.7)).blur_radius(z(20.0, s)).spread_radius(z(-10.0, s));
    let mut focused = ring();
    focused.push(lift.clone());
    let mut button = div()
        .id(id)
        .tab_index(0)
        .relative()
        .h(z(40.0, s))
        .px(z(18.0, s))
        .flex()
        .flex_none()
        .items_center()
        .gap(z(7.0, s))
        .rounded(z(12.0, s))
        .border_1()
        .border_color(c("solid"))
        .bg(c("solid"))
        .text_color(c("solid_foreground"))
        .cursor_pointer()
        .text_size(z(14.0, s))
        .font_weight(FontWeight::MEDIUM)
        .hover(|d| d.opacity(0.92))
        .focus(move |mut d| {
            d.box_shadow = Some(focused);
            d
        });
    button.style().box_shadow = Some(vec![lift]);
    button
        .child(div().absolute().top(px(0.0)).left(z(12.0, s)).right(z(12.0, s)).h(px(1.0)).bg(white().opacity(0.25)))
        .child(label)
        .child(svg().data(OPEN_ICON).size(z(15.0, s)).flex_none().text_color(c("solid_foreground")))
}

/// Cancel and Skip: a quiet glass button.
fn quiet_button(id: &'static str, label: &'static str, s: f32, light: bool) -> Stateful<Div> {
    let ring = ring();
    div()
        .id(id)
        .tab_index(0)
        .h(z(34.0, s))
        .px(z(14.0, s))
        .flex()
        .flex_none()
        .items_center()
        .rounded(z(10.0, s))
        .border_1()
        .border_color(veil(light, 0.12, 0.10))
        .bg(veil(light, 0.06, 0.04))
        .text_color(c("foreground"))
        .cursor_pointer()
        .text_size(z(13.0, s))
        .font_weight(FontWeight::MEDIUM)
        .hover(move |d| d.bg(veil(light, 0.10, 0.07)))
        .focus(move |mut d| {
            d.box_shadow = Some(ring);
            d
        })
        .child(label)
}

/// A link of the mark tile: an icon box, the domain over what it is, an arrow.
#[allow(clippy::too_many_arguments)]
fn link_row(id: &'static str, fill: &'static [u8], line: &'static [u8], domain: &'static str, note: &'static str, url: &'static str, s: f32, light: bool) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .h(z(52.0, s))
        .px(z(8.0, s))
        .flex()
        .items_center()
        .gap(z(10.0, s))
        .rounded(z(14.0, s))
        .cursor_pointer()
        .hover(move |d| d.bg(veil(light, 0.05, 0.04)))
        .focus(|mut d| {
            d.box_shadow = Some(ring());
            d
        })
        .on_click(open_url(url))
        .child(
            div()
                .size(z(34.0, s))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(z(10.0, s))
                .border_1()
                .border_color(veil(light, 0.08, 0.07))
                .bg(veil(light, 0.05, 0.03))
                .child(duo(fill, line, z(18.0, s), c("muted"))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(div().font_family(crate::settings::DEFAULT_FONT).text_size(z(12.5, s)).line_height(z(16.0, s)).text_color(c("foreground")).child(domain))
                .child(div().text_size(z(11.5, s)).line_height(z(15.0, s)).text_color(c("muted")).child(note)),
        )
        .child(svg().data(ARROW_ICON).size(z(14.0, s)).flex_none().text_color(c("dim")))
}

fn spinner() -> Div {
    div().flex_none().size(px(14.0)).mt(px(3.0)).child(svg().data(SPIN_ARC).size_full().text_color(c("accent")).with_animation(
        "spin",
        Animation::new(Duration::from_millis(900)).repeat(),
        |s, delta| s.with_transformation(Transformation::rotate(percentage(delta))),
    ))
}

/// The soft tint behind the grid: ten stacked ellipses centred on (`cx`, `cy`), each a tenth of the peak, from the full size down to a tenth.
fn wash(tint: Hsla, peak: f32, cx: f32, cy: f32, w: f32, h: f32) -> Div {
    div().absolute().left(px(cx - w / 2.0)).top(px(cy - h / 2.0)).w(px(w)).h(px(h)).children((0..10).map(move |i| {
        let k = 1.0 - i as f32 * 0.1;
        let (lw, lh) = (w * k, h * k);
        div().absolute().left(px((w - lw) / 2.0)).top(px((h - lh) / 2.0)).w(px(lw)).h(px(lh)).rounded(px(999.0)).bg(tint.opacity(peak / 10.0))
    }))
}

/// A feature tile: a tinted icon box, then the name and what it does at the bottom.
fn feature(i: usize, s: f32, compact: bool, tint: Hsla, light: bool) -> Stateful<Div> {
    let (pad, icon_box, icon, name, name_lh, benefit, benefit_lh) = if compact { (14.0, 32.0, 20.0, 13.0, 17.0, 11.5, 15.0) } else { (18.0, 44.0, 24.0, 14.0, 19.0, 12.5, 17.0) };
    glass(PERK_IDS[i], 22.0, s, light)
        .flex()
        .flex_col()
        .px(z(pad, s))
        .pt(z(pad, s))
        .pb(z(pad - 2.0, s))
        .child(
            div()
                .size(z(icon_box, s))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(z(if compact { 10.0 } else { 12.0 }, s))
                .bg(tint.opacity(0.10))
                .border_1()
                .border_color(tint.opacity(0.22))
                .child(duo(PERK_FILLS[i], PERKS[i].0, z(icon, s), tint)),
        )
        .child(div().flex_1())
        .child(div().text_size(z(name, s)).line_height(z(name_lh, s)).font_weight(FontWeight::SEMIBOLD).text_color(c("foreground")).child(PERKS[i].1))
        .child(div().mt(z(3.0, s)).text_size(z(benefit, s)).line_height(z(benefit_lh, s)).text_color(c("muted")).child(BENEFITS[i]))
}

impl Render for SignIn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        let lay = layout(width, height);
        let (s, compact) = (lay.scale, lay.mode == Mode::Compact);
        let (light, tint) = (light(), tint());
        let view = content(self.state);
        let idle = matches!(self.state, State::Idle);
        let status = view.status.map(|status| {
            let (title, body, tone) = match &status {
                Status::Waiting => (WAITING_TITLE, Some(WAITING_BODY), c("accent")),
                Status::Error(e) => (*e, None, c("blocked")),
            };
            let lead = match status {
                Status::Waiting => spinner().into_any_element(),
                Status::Error(_) => svg().data(ALERT_ICON).flex_none().size(px(15.0)).mt(px(2.0)).text_color(c("blocked")).into_any_element(),
            };
            div()
                .self_stretch()
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(12.0))
                .flex()
                .gap(px(10.0))
                .items_start()
                .bg(tone.opacity(0.10))
                .border_1()
                .border_color(tone.opacity(if body.is_some() { 0.22 } else { 0.25 }))
                .child(lead)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(match body {
                            Some(_) => div().text_size(px(13.5)).line_height(px(20.0)).font_weight(FontWeight::SEMIBOLD).text_color(c("foreground")).child(title),
                            None => div().text_size(px(12.5)).line_height(px(20.0)).text_color(c("foreground")).child(title),
                        })
                        .when_some(body, |d, body| d.child(div().text_size(px(13.0)).line_height(px(20.0)).text_color(c("muted")).child(body))),
                )
        });

        // The hero: title, what the app is, the sign-in action and, until a sign-in is under way, Skip.
        let (pad_x, pad_top, pad_bottom, title, title_lh) = if compact { (26.0, 24.0, 20.0, 26.0, 32.0) } else { (32.0, 30.0, 26.0, 32.0, 38.0) };
        let hero = glass("signin-card", if compact { 22.0 } else { 26.0 }, s, light)
            .flex()
            .flex_col()
            .px(z(pad_x, s))
            .pt(z(pad_top, s))
            .pb(z(pad_bottom, s))
            .when(compact, |d| d.w_full())
            .when(!compact, |d| d.flex_none().w(z(2.0 * COL + GAP, s)).h(z(HERO_H, s)))
            .child(div().text_size(z(title, s)).line_height(z(title_lh, s)).font_weight(FontWeight::SEMIBOLD).text_color(c("foreground")).child("Welcome to HyperOx"))
            .child(div().mt(z(8.0, s)).max_w(z(470.0, s)).text_size(z(14.5, s)).line_height(z(22.0, s)).text_color(c("muted")).child(TAGLINE))
            .child(
                div()
                    .mt(z(26.0, s))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(z(10.0, s))
                    .when_some(status, |d, st| d.child(st))
                    .when_some(view.primary, |d, label| {
                        let id = if idle { "signin" } else { "retry" };
                        d.child(primary_button(id, label, s).on_click(cx.listener(|this, _, w, cx| this.start(w, cx))))
                    })
                    .when_some(view.secondary, |d, label| d.child(quiet_button("cancel", label, s, light).on_click(cx.listener(|this, _, w, cx| this.cancel(w, cx)))))
                    .when_some(view.note, |d, note| {
                        let row = div().flex().text_size(z(12.5, s)).line_height(z(18.0, s)).text_color(c("muted"));
                        d.child(match note.split_once(TEAM_DOMAIN) {
                            Some((before, after)) => row
                                .child(before)
                                .child(div().font_family(crate::settings::DEFAULT_FONT).text_size(z(11.5, s)).child(TEAM_DOMAIN))
                                .child(after),
                            None => row.child(note),
                        })
                    }),
            )
            .when(view.skip && !compact, |d| d.child(div().flex_1()))
            .when(view.skip, |d| {
                d.child(
                    div()
                        .self_stretch()
                        .when(compact, |d| d.mt(px(16.0)))
                        .flex()
                        .flex_col()
                        .child(div().h(px(1.0)).self_stretch().bg(veil(light, 0.08, 0.07)))
                        .child(
                            div()
                                .pt(z(16.0, s))
                                .flex()
                                .items_center()
                                .gap(z(14.0, s))
                                .child(quiet_button("signin-skip", SKIP, s, light).track_focus(&self.skip_focus).on_click(cx.listener(|this, _, w, cx| this.skip(w, cx))))
                                .child(div().id("signin-skip-note").text_size(z(12.5, s)).line_height(z(18.0, s)).text_color(c("muted")).children(SKIP_NOTE.map(|line| div().child(line)))),
                        ),
                )
            });

        // The mark and the two links. Compact: only the links, side by side, in a row.
        let team = link_row("team", GLOBE_FILL, GLOBE_LINE, TEAM_DOMAIN, TEAM_NOTE, TEAM_URL, s, light);
        let partner = link_row("partner", CHAIN_FILL, CHAIN_LINE, PARTNER_DOMAIN, PARTNER_NOTE, PARTNER_URL, s, light);
        let mark = if compact {
            glass("signin-mark", 22.0, s, light)
                .w_full()
                .h(px(56.0))
                .flex()
                .items_center()
                .child(team.flex_1().min_w(px(0.0)))
                .child(div().w(px(1.0)).h(px(32.0)).flex_none().bg(veil(light, 0.07, 0.06)))
                .child(partner.flex_1().min_w(px(0.0)))
        } else {
            glass("signin-mark", 22.0, s, light)
                .flex_none()
                .w(z(COL, s))
                .h(z(HERO_H, s))
                .flex()
                .flex_col()
                .items_center()
                .pt(z(26.0, s))
                .px(z(12.0, s))
                .pb(z(10.0, s))
                .child(
                    canvas(
                        |bounds, _, _| bounds,
                        move |_, bounds, window, _| {
                            for (x, y) in mark_dots() {
                                let at = bounds.origin + point(px((x - DOT_RADIUS) * s), px((y - DOT_RADIUS) * s));
                                let d = px(DOT_RADIUS * 2.0 * s);
                                window.paint_quad(fill(Bounds::new(at, size(d, d)), tint.opacity(0.95)).corner_radii(px(DOT_RADIUS * s)));
                            }
                        },
                    )
                    .w(z(DOT_BOX.0, s))
                    .h(z(DOT_BOX.1, s))
                    .flex_none(),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .self_stretch()
                        .flex()
                        .flex_col()
                        .child(team.self_stretch())
                        .child(div().h(px(1.0)).self_stretch().bg(veil(light, 0.07, 0.06)))
                        .child(partner.self_stretch()),
                )
        };

        // "Signing in turns on" and the three features.
        let features = (0..3).map(|i| {
            let tile = feature(i, s, compact, tint, light);
            if compact { tile.flex_1().min_w(px(0.0)).h(px(122.0)) } else { tile.flex_none().w(z(COL, s)).h(z(FEATURE_H, s)) }
        });
        let gap = if compact { 10.0 } else { GAP };
        let perks = div()
            .id("signin-perks")
            .flex()
            .flex_col()
            .gap(z(gap, s))
            .child(div().h(z(LEAD_H, s)).pl(z(4.0, s)).text_size(z(12.0, s)).line_height(z(LEAD_H, s)).text_color(c("muted")).child(PERKS_LEAD))
            .child(div().flex().gap(z(gap, s)).children(features));

        // The grid, centred as one piece: 3% of the height below the middle, or 8 above it when compact.
        let grid = if compact {
            let w = compact_width(width);
            div()
                .relative()
                .w(px(w))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(wash(tint, if light { 0.07 } else { 0.16 }, w / 2.0, 100.0, w * 1.5, 361.0))
                .child(hero)
                .child(perks)
                .child(mark)
        } else {
            div()
                .relative()
                .w(z(GRID_W, s))
                .flex()
                .flex_col()
                .gap(z(GAP, s))
                .child(wash(tint, if light { 0.07 } else { 0.16 }, (2.0 * COL + GAP) * s / 2.0, HERO_H * s / 2.0, 1.5 * (2.0 * COL + GAP) * s, 1.9 * HERO_H * s))
                .child(div().flex().gap(z(GAP, s)).child(hero).child(mark))
                .child(perks)
        };

        let hint_line = div().flex().items_center().gap(px(8.0)).font_family(crate::settings::DEFAULT_FONT).text_size(px(11.5)).children(hints(self.state).into_iter().enumerate().flat_map(
            |(i, (key, verb))| {
                let sep = (i > 0).then(|| div().text_color(c("dim")).child("\u{b7}").into_any_element());
                let item = div().flex().gap(px(6.0)).child(div().font_weight(FontWeight::MEDIUM).text_color(c("foreground")).child(key)).child(div().text_color(c("muted")).child(verb));
                sep.into_iter().chain(std::iter::once(item.into_any_element()))
            },
        ));
        let footer = div().absolute().bottom(px(0.0)).left(px(0.0)).right(px(0.0)).h(px(36.0)).flex().items_center().justify_center().child(hint_line);

        let animating = rain_runs(cx.reduce_motion(), window.is_window_active(), lay.rain);
        let frame = shown_frame(self.frame, animating);
        let rain_state = self.rain.clone();
        let tiles = tile_rects(width, height, s).to_vec();
        let band = lay.band;

        div()
            .id("signin-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev, window, cx| this.key_down(ev, window, cx)))
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family(crate::settings::get().ui_font_family.clone())
            .text_color(c("foreground"))
            .bg(c("background"))
            .when(lay.rain, |d| {
                d.child(
                    div().id("signin-rain").absolute().left(px(0.0)).right(px(0.0)).bottom(px(44.0)).h(px(band)).child(
                        canvas(
                            |bounds, _, _| bounds,
                            move |_, bounds, window, cx| paint_rain(&rain_state, &tiles, light, frame, bounds, window, cx),
                        )
                        .size_full(),
                    ),
                )
            })
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    // Twice the offset of the centre: the box is centred in what the padding leaves.
                    .when(!compact, |d| d.pt(px(0.06 * height)))
                    .when(compact, |d| d.pb(px(16.0)))
                    .child(grid),
            )
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in `gpui::test`, which `#[test]` would then expand forever.
    use super::{
        Action, BENEFITS, CANCELLED, OTHER, PERKS, PERKS_LEAD, REJECTED, SIGN_IN, SIGN_IN_NOTE, SKIP_NOTE, State, Status, TIMEOUT, TRY_AGAIN, UNREACHABLE, content, decide,
        enter_action, friendly, ring, window_height, window_width,
    };

    #[test]
    fn the_focus_ring_is_a_2px_ring_band_outside_a_2px_background_gap() {
        let ring = ring();
        let widths: Vec<f32> = ring.iter().map(|s| f32::from(s.spread_radius)).collect();
        assert_eq!(widths, [4.0, 2.0], "the wide ring shadow paints first, the background gap over its inner half");
        assert_eq!(ring[0].color, crate::settings::color("ring"));
        assert_eq!(ring[1].color, crate::settings::color("background"));
    }

    #[test]
    fn the_window_cannot_be_maximized_and_keeps_its_size_limits() {
        use gpui::{Bounds, WindowBounds, point, px, size};
        let bounds = WindowBounds::Windowed(Bounds::new(point(px(0.0), px(0.0)), super::window_size(None)));
        let options = super::window_options(bounds);
        assert!(!options.is_resizable, "no maximize box, no double-click or Win+Up maximize");
        assert!(options.is_minimizable);
        assert_eq!(options.window_min_size, Some(size(px(620.0), px(460.0))));
        assert!(options.window_bounds.is_some());
        assert_eq!(options.titlebar.and_then(|t| t.title).as_deref(), Some("HyperOx"));
        let dims = |visible| {
            let s = super::window_size(visible);
            (f32::from(s.width), f32::from(s.height))
        };
        assert_eq!(dims(None), (1040.0, 760.0));
        assert_eq!(dims(Some((1920.0, 1040.0))), (1040.0, 760.0));
        assert_eq!(dims(Some((1366.0, 728.0))), (1040.0, 648.0), "a short laptop screen");
        assert_eq!(dims(Some((500.0, 500.0))), (620.0, 460.0));
        // The window's own size picks the grid: regular on a normal screen, compact on a small one.
        for (visible, regular) in [(None, true), (Some((1920.0, 1080.0)), true), (Some((1366.0, 768.0)), true), (Some((1024.0, 700.0)), false), (Some((979.0, 1000.0)), false), (Some((980.0, 1000.0)), true)] {
            let (w, h) = dims(visible);
            assert_eq!(super::layout(w, h).mode == super::Mode::Regular, regular, "{visible:?} gives {w}x{h}");
        }
    }

    #[test]
    fn the_layout_follows_the_window_size() {
        use super::{Layout, Mode, layout};
        // A small window (760 x 640) is the compact grid: no code band, tiles in one column.
        assert_eq!(layout(760.0, 640.0), Layout { mode: Mode::Compact, scale: 1.0, band: 238.0, rain: false });
        assert_eq!(layout(620.0, 460.0), Layout { mode: Mode::Compact, scale: 1.0, band: 166.0, rain: false });
        assert_eq!(layout(1280.0, 800.0).mode, Mode::Regular);
        assert!(layout(1280.0, 800.0).rain);
        // The band: 40% of the room up to 700 high, 24% above, never under 120 or over 260.
        assert_eq!(layout(1280.0, 800.0).band, (756.0_f32 * 0.24).round());
        assert_eq!(layout(1280.0, 700.0).band, (656.0_f32 * 0.40).round().clamp(120.0, 260.0));
        assert_eq!(layout(1920.0, 1080.0).band, (1036.0_f32 * 0.24).round().min(260.0));
        assert_eq!(layout(1920.0, 3000.0).band, 260.0);
        assert_eq!(layout(900.0, 680.0).band, (636.0_f32 * 0.40).round().clamp(120.0, 260.0));
    }

    #[test]
    fn the_compact_grid_replaces_the_regular_one_below_900_by_680() {
        use super::{Mode, compact_width, layout};
        assert_eq!(layout(900.0, 680.0).mode, Mode::Regular, "both limits met");
        assert_eq!(layout(899.0, 680.0).mode, Mode::Compact, "too narrow");
        assert_eq!(layout(900.0, 679.0).mode, Mode::Compact, "too short");
        assert_eq!(layout(760.0, 640.0).mode, Mode::Compact);
        assert_eq!(layout(2560.0, 1440.0).mode, Mode::Regular);
        // No code band in the compact grid: the tiles would cover it.
        assert!(!layout(760.0, 640.0).rain && !layout(899.0, 1200.0).rain && layout(1280.0, 800.0).rain);
        // 712 wide, or the window less 16 each side.
        assert_eq!(compact_width(760.0), 712.0);
        assert_eq!(compact_width(740.0), 708.0);
        assert_eq!(compact_width(620.0), 588.0);
        assert_eq!(compact_width(899.0), 712.0);
    }

    #[test]
    fn the_whole_grid_grows_by_112_percent_from_1600_by_950() {
        use super::layout;
        assert_eq!(layout(1600.0, 950.0).scale, 1.12);
        assert_eq!(layout(1920.0, 1080.0).scale, 1.12);
        assert_eq!(layout(1599.0, 1080.0).scale, 1.0);
        assert_eq!(layout(1920.0, 949.0).scale, 1.0);
        assert_eq!(layout(1280.0, 800.0).scale, 1.0);
        assert_eq!(layout(760.0, 640.0).scale, 1.0, "compact never scales");
    }

    #[test]
    fn the_tiles_sit_in_a_grid_centred_three_percent_below_the_middle() {
        use super::{GRID_H, GRID_W, tile_rects};
        let [hero, mark, a, b, c] = tile_rects(1280.0, 800.0, 1.0);
        // The 1280 x 800 still: the hero at (196, 174), 588 x 312, the mark tile beside it, the features at 526.
        assert_eq!(hero, (196.0, 174.0, 784.0, 486.0));
        assert_eq!(mark, (796.0, 174.0, 1084.0, 486.0));
        assert_eq!((a, b, c), ((196.0, 526.0, 484.0, 674.0), (496.0, 526.0, 784.0, 674.0), (796.0, 526.0, 1084.0, 674.0)));
        assert_eq!((GRID_W, GRID_H), (888.0, 500.0));
        // Centred as a unit: 3% of the height below the middle, at any size and scale.
        for (w, h, s) in [(1280.0, 800.0, 1.0), (1920.0, 1080.0, 1.12), (900.0, 680.0, 1.0)] {
            let rects = tile_rects(w, h, s);
            let (left, top) = (rects[0].0, rects[0].1);
            let (right, bottom) = (rects[1].2, rects[4].3);
            assert!(((left + right) / 2.0 - w / 2.0).abs() < 0.01, "{w}x{h}");
            assert!(((top + bottom) / 2.0 - (h / 2.0 + 0.03 * h)).abs() < 0.01, "{w}x{h}");
            assert!((right - left - GRID_W * s).abs() < 0.01 && (bottom - top - GRID_H * s).abs() < 0.01);
        }
    }

    #[test]
    fn the_icons_are_duotone_pairs_a_soft_fill_and_a_line() {
        use super::{ALERT_ICON, ARROW_ICON, CHAIN_FILL, CHAIN_LINE, GLOBE_FILL, GLOBE_LINE, OBSERVE_FILL, OBSERVE_LINE, OPEN_ICON, PERK_FILLS, PERKS, PII_FILL, PII_LINE, TOKENS_FILL, TOKENS_LINE};
        let text = |b: &[u8]| std::str::from_utf8(b).unwrap().to_owned();
        for (fill, line) in [(TOKENS_FILL, TOKENS_LINE), (OBSERVE_FILL, OBSERVE_LINE), (PII_FILL, PII_LINE), (GLOBE_FILL, GLOBE_LINE), (CHAIN_FILL, CHAIN_LINE)] {
            let (fill, line) = (text(fill), text(line));
            assert!(fill.contains("viewBox=\"0 0 24 24\"") && fill.contains("fill=\"#000\"") && !fill.contains("stroke"), "{fill}");
            assert!(line.contains("viewBox=\"0 0 24 24\"") && line.contains("stroke-width=\"1.5\"") && line.contains("fill=\"none\""), "{line}");
            // Both layers draw the same shape.
            let shape = fill.split_once('>').unwrap().1.trim_end_matches("</svg>").to_owned();
            assert!(line.contains(&shape), "the line layer holds the fill layer's shape: {shape}");
        }
        for single in [OPEN_ICON, ARROW_ICON, ALERT_ICON] {
            assert!(text(single).contains("stroke-width=\"1.5\""));
        }
        assert_eq!(PERK_FILLS.len(), PERKS.len());
        assert_eq!(PERK_FILLS[0], TOKENS_FILL);
        assert_eq!(PERKS[1].0, OBSERVE_LINE);
        assert_eq!(PERKS[2].0, PII_LINE);
    }

    #[test]
    fn the_screen_takes_the_theme_the_main_window_will_use() {
        use crate::settings::{Appearance, Person, color, parse_hex, set};
        let dark = |system_dark, appearance| {
            set(super::theme_settings(&Person { appearance, ..Default::default() }, system_dark));
            (color("accent"), color("background"))
        };
        let (accent, background) = dark(false, Appearance::Dark);
        assert_eq!(accent, parse_hex("#e78a53").unwrap(), "a dark settings file: darkmatter's orange");
        assert_eq!(background, parse_hex("#121113").unwrap());
        let (_, background) = dark(true, Appearance::Light);
        assert_eq!(background, parse_hex("#ffffff").unwrap(), "a light settings file: paper");
        // The system's mode counts only when the person did not choose.
        assert_eq!(dark(true, Appearance::System).1, parse_hex("#121113").unwrap());
        assert_eq!(dark(false, Appearance::System).1, parse_hex("#ffffff").unwrap());
        assert_eq!(super::theme_settings(&Person::default(), true).theme, "darkmatter");
        assert_eq!(super::theme_settings(&Person::default(), false).theme, "paper");
    }

    #[test]
    fn mulberry32_matches_the_mockup() {
        let mut r = super::Mulberry(super::RAIN_SEED);
        assert_eq!([r.next_u32(), r.next_u32(), r.next_u32()], [1487234826, 2853541191, 1518954671]);
    }

    #[test]
    fn the_code_texture_is_the_same_for_the_same_seed_and_size() {
        use super::Rain;
        assert_eq!(Rain::new(30, 12), Rain::new(30, 12));
        let (a, b) = (Rain::new(30, 12), Rain::new(30, 12));
        for frame in [0, 1, 7, 100, 4_000] {
            for c in 0..30 {
                for r in 0..12 {
                    assert_eq!(a.cell(c, r, frame), b.cell(c, r, frame), "{c},{r}@{frame}");
                }
            }
        }
        assert_ne!(Rain::new(30, 12), Rain::new(31, 12));
        // The first column's draws, in the mock-up's order (rows 10: heads 4 and -2, trail 12).
        let first = &Rain::new(5, 10).cols[0];
        assert_eq!((first.h1, first.h2, first.len), (4, -2, 12));
        assert!((1..=3).contains(&first.every));
        assert_eq!((first.glyph.len(), first.noise.len()), (10, 10));
    }

    #[test]
    fn a_head_is_bright_with_a_fading_trail_above_it_and_glyphs_stay_ascii() {
        use super::{GLYPHS, Rain};
        let rain = Rain::new(5, 10);
        let (glyph, strength) = rain.cell(0, 4, 0).expect("column 0's first head");
        assert_eq!(strength, 0.26);
        assert!(glyph < GLYPHS.len());
        let (_, trail) = rain.cell(0, 3, 0).expect("the trail above the head");
        assert!(trail < 0.26 && trail > 0.04);
        assert!(rain.cell(0, 3, 0).unwrap().1 > rain.cell(0, 1, 0).map_or(0.0, |c| c.1) || rain.cell(0, 1, 0).is_none());
        assert!(GLYPHS.iter().all(u8::is_ascii) && GLYPHS.len() == 37);
        assert_eq!(rain.cell(9, 0, 0), None, "past the last column");
        for frame in 0..200 {
            for r in 0..10 {
                if let Some((g, s)) = rain.cell(1, r, frame) {
                    assert!(g < GLYPHS.len() && (0.04..=0.26).contains(&s), "{r}@{frame}: {g} {s}");
                }
            }
        }
    }

    #[test]
    fn a_head_shows_a_new_glyph_every_step_and_a_trail_keeps_its_glyph() {
        use super::Rain;
        let rain = Rain::new(1, 30);
        let c = &rain.cols[0];
        let step = c.every as u64;
        let head = |frame| (0..30).find_map(|r| rain.cell(0, r, frame).filter(|(_, s)| *s == 0.26).map(|(g, _)| (r, g)));
        let (r0, g0) = head(0).unwrap();
        let (r1, g1) = head(step).unwrap();
        assert_eq!(r1, r0 + 1, "the head moves a row each step");
        assert_eq!(g0, c.glyph[r0] as usize, "the first step shows the cell's own glyph");
        assert_eq!(g1, (c.glyph[r1] as usize + 1) % super::GLYPHS.len(), "the next step moves the glyph on by one");
    }

    #[test]
    fn the_texture_fades_toward_the_top_and_away_from_the_card() {
        use super::{card_distance, cell_weight};
        assert_eq!(cell_weight(0.0, 200.0, 1000.0, false), 0.0, "invisible at the top of the band");
        assert_eq!(cell_weight(150.0, 200.0, 1000.0, false), 1.0, "full from three quarters down");
        assert_eq!(cell_weight(190.0, 200.0, 1000.0, false), 1.0);
        assert!(cell_weight(75.0, 200.0, 1000.0, false) > 0.0 && cell_weight(75.0, 200.0, 1000.0, false) < 1.0);
        assert_eq!(cell_weight(150.0, 200.0, 1000.0, true), 0.55, "light themes are calmer");
        assert_eq!(cell_weight(150.0, 200.0, 6.0, false), 0.0, "within 6 px of the card: nothing");
        assert_eq!(cell_weight(150.0, 200.0, 20.0, false), 0.5);
        assert_eq!(cell_weight(150.0, 200.0, 34.0, false), 1.0);
        let card = (100.0, 100.0, 500.0, 400.0);
        assert_eq!(card_distance((200.0, 200.0, 9.0, 14.0), card), 0.0, "inside");
        assert_eq!(card_distance((520.0, 200.0, 9.0, 14.0), card), 20.0, "right of it");
        assert_eq!(card_distance((200.0, 420.0, 9.0, 14.0), card), 20.0, "below it");
        assert_eq!(card_distance((60.0, 60.0, 10.0, 10.0), card), (30.0_f32).hypot(30.0), "diagonal");
    }

    #[test]
    fn reduced_motion_gives_a_static_first_frame() {
        use super::{rain_runs, shown_frame};
        assert!(rain_runs(false, true, true));
        assert!(!rain_runs(true, true, true), "the system asks for reduced motion");
        assert!(!rain_runs(false, false, true), "a window behind others does not repaint");
        assert!(!rain_runs(false, true, false), "a narrow window has no texture");
        assert_eq!(shown_frame(57, true), 57);
        assert_eq!(shown_frame(57, false), 0, "static: the first frame, whatever the tick");
        assert_eq!(super::TICK.as_millis(), 110);
    }

    #[test]
    fn the_mark_is_drawn_as_a_few_hundred_dots_inside_its_box() {
        let dots = super::mark_dots();
        assert!((200..=700).contains(&dots.len()), "{}", dots.len());
        assert!(dots.iter().all(|(x, y)| (0.0..120.0).contains(x) && (0.0..141.0).contains(y)));
        assert_eq!(super::dots_in(std::str::from_utf8(super::MARK).unwrap()), dots, "the same every time");
        assert!(super::dots_in("<svg/>").is_empty());
        // A square outline with a square hole (even-odd) leaves a ring.
        let ring = super::dots_in(r#"<svg viewBox="0 0 715 842"><path d="M0 0 715 0 715 842 0 842ZM170 200 545 200 545 640 170 640Z"/></svg>"#);
        let full = super::dots_in(r#"<svg viewBox="0 0 715 842"><path d="M0 0 715 0 715 842 0 842Z"/></svg>"#);
        assert!(ring.len() < full.len() && !ring.is_empty());
        assert!(!ring.contains(&(47.5, 57.5)), "the hole is empty");
    }

    #[test]
    fn the_footer_hints_name_the_keys_each_state_answers() {
        use super::hints;
        assert_eq!(hints(State::Idle), [("Enter", "sign in"), ("Tab", "focus")]);
        assert_eq!(hints(State::Waiting), [("Esc", "cancel"), ("Tab", "focus")]);
        assert_eq!(hints(State::Failed(OTHER)), [("Enter", "try again"), ("Tab", "focus")]);
    }

    #[test]
    fn the_links_ids_and_copy_are_there() {
        assert_eq!((super::TEAM_URL, super::TEAM_DOMAIN, super::TEAM_NOTE), ("https://pavii.tech", "pavii.tech", "Made by Team Pavii"));
        assert_eq!((super::PARTNER_URL, super::PARTNER_DOMAIN, super::PARTNER_NOTE), ("https://buckleson.com", "buckleson.com", "Blockchain & security partner"));
        // Only the screen's own code counts, not this list.
        let source = include_str!("signin.rs").split("#[cfg(test)]").next().unwrap();
        for id in ["signin-root", "signin-card", "signin-perks", "\"signin\"", "\"retry\"", "\"cancel\"", "\"signin-skip\"", "signin-skip-note", "\"team\"", "\"partner\"", "signin-rain", "signin-mark", "feat-tokens", "feat-observe", "feat-pii"] {
            assert!(source.contains(id), "{id}");
        }
        assert_eq!(BENEFITS.len(), PERKS.len());
        assert_eq!(BENEFITS[0], "Leaner prompts, so agent sessions use fewer tokens.");
        assert_eq!(BENEFITS[1], "See what each agent ran, changed and spent.");
        assert_eq!(BENEFITS[2], "Personal data is masked before it leaves this computer.");
    }

    #[test]
    fn idle_offers_sign_in_with_a_note_and_nothing_else() {
        let c = content(State::Idle);
        assert_eq!((c.primary, c.secondary, c.note), (Some(SIGN_IN), None, Some(SIGN_IN_NOTE)));
        assert_eq!(c.status, None);
        assert!(c.skip);
    }

    #[test]
    fn waiting_shows_the_spinner_and_only_cancel() {
        let c = content(State::Waiting);
        assert_eq!((c.primary, c.secondary, c.note), (None, Some("Cancel"), None));
        assert_eq!(c.status, Some(Status::Waiting));
        assert!(!c.skip, "a browser sign-in is in flight: no Skip");
    }

    #[test]
    fn a_failure_shows_the_friendly_message_and_try_again() {
        for error in ["timeout", "cancelled", "network: x", "rejected: x", "other"] {
            let c = content(State::Failed(friendly(error)));
            assert_eq!(c.status, Some(Status::Error(friendly(error))));
            assert_eq!((c.primary, c.secondary), (Some(TRY_AGAIN), None));
            assert!(c.skip);
        }
    }

    #[test]
    fn the_perks_come_in_the_owners_words_and_order() {
        assert_eq!(PERKS_LEAD, "Signing in turns on");
        let labels: Vec<&str> = PERKS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Token saving", "Observability engine", "Blockchain security for PII masking"]);
    }

    #[test]
    fn the_skip_note_points_to_settings_profile() {
        assert_eq!(SKIP_NOTE[0], "Skipping turns these off.");
        assert!(SKIP_NOTE[1].starts_with("Sign in later in Settings") && SKIP_NOTE[1].ends_with("Profile."));
    }

    #[test]
    fn no_em_dash_on_the_screen() {
        let mut texts = vec![PERKS_LEAD, SIGN_IN, SIGN_IN_NOTE, TRY_AGAIN, CANCELLED, TIMEOUT, REJECTED, OTHER, UNREACHABLE, SKIP_NOTE[0], SKIP_NOTE[1]];
        texts.extend(PERKS.iter().map(|(_, l)| *l));
        texts.extend(BENEFITS);
        for t in texts {
            assert!(!t.contains('\u{2014}'), "{t}");
        }
        assert_eq!(UNREACHABLE, "Couldn't reach Pavii. Check your connection and try again.");
    }

    #[test]
    fn enter_signs_in_at_the_root_skips_on_skip_and_does_nothing_while_waiting() {
        assert_eq!(enter_action(State::Waiting, false), Action::None);
        assert_eq!(enter_action(State::Waiting, true), Action::None);
        assert_eq!(enter_action(State::Idle, false), Action::Start);
        assert_eq!(enter_action(State::Idle, true), Action::Skip);
        assert_eq!(enter_action(State::Failed(CANCELLED), false), Action::Start);
        assert_eq!(enter_action(State::Failed(CANCELLED), true), Action::Skip);
    }

    #[test]
    fn the_window_is_760_high_and_shrinks_on_a_short_screen_but_never_below_460() {
        assert_eq!(window_height(None), 760.0);
        assert_eq!(window_height(Some(1040.0)), 760.0);
        assert_eq!(window_height(Some(840.0)), 760.0);
        assert_eq!(window_height(Some(768.0)), 688.0);
        assert_eq!(window_height(Some(700.0)), 620.0);
        assert_eq!(window_height(Some(500.0)), 460.0);
        assert_eq!(window_height(Some(0.0)), 460.0);
    }

    #[test]
    fn the_window_is_1040_wide_and_shrinks_on_a_narrow_screen_but_never_below_620() {
        assert_eq!(window_width(None), 1040.0);
        assert_eq!(window_width(Some(1920.0)), 1040.0);
        assert_eq!(window_width(Some(1120.0)), 1040.0);
        assert_eq!(window_width(Some(1366.0)), 1040.0);
        assert_eq!(window_width(Some(1024.0)), 944.0);
        assert_eq!(window_width(Some(700.0)), 620.0);
        assert_eq!(window_width(Some(0.0)), 620.0);
        for (visible, want) in [(699.0, 620.0), (700.0, 620.0), (701.0, 621.0), (1119.0, 1039.0), (1121.0, 1040.0), (f32::MAX, 1040.0), (f32::INFINITY, 1040.0), (f32::NEG_INFINITY, 620.0), (f32::MIN, 620.0)] {
            assert_eq!(window_width(Some(visible)), want, "{visible}");
        }
        let w = window_width(Some(f32::NAN));
        assert!(w.is_finite() && (620.0..=1040.0).contains(&w), "{w}");
    }

    #[test]
    fn the_regular_grid_fits_the_window_with_margins_and_stays_centred_below_the_middle() {
        use super::{GRID_W, layout, tile_rects};
        let (w, h) = (1040.0, 760.0);
        assert_eq!(layout(w, h).mode, super::Mode::Regular);
        let [hero, mark, a, _, c] = tile_rects(w, h, 1.0);
        assert_eq!(hero, (76.0, 0.03 * h + h / 2.0 - 250.0, 664.0, 0.03 * h + h / 2.0 - 250.0 + 312.0));
        assert!(hero.0 >= 40.0 && w - mark.2 >= 40.0, "margins of 40 px or more at the sides: {} / {}", hero.0, w - mark.2);
        assert_eq!(w - GRID_W, 152.0);
        assert!(a.1 > hero.3 && c.3 + 36.0 <= h, "the grid sits above the footer line");
        assert!(hero.1 >= 0.0, "and below the top edge");
    }

    #[test]
    fn signed_out_shows_the_screen_signed_in_opens_main() {
        assert!(decide(false, || false));
        assert!(!decide(false, || true));
    }

    #[test]
    fn the_bypass_skips_the_screen_without_asking_the_account() {
        assert!(!decide(true, || panic!("must not be asked")));
    }

    #[test]
    fn each_stable_sign_in_error_gets_its_own_plain_words_never_the_raw_error() {
        assert_eq!(friendly("cancelled"), CANCELLED);
        assert_eq!(friendly("timeout"), TIMEOUT);
        assert_eq!(friendly("network: curl: (6) Could not resolve host: securetoken.googleapis.com"), UNREACHABLE);
        assert_eq!(friendly("network: HTTP 503"), UNREACHABLE);
        assert_eq!(friendly("rejected: USER_DISABLED"), REJECTED);
        assert_eq!(friendly("rejected: account mismatch"), REJECTED);
        for error in ["listener: unusable port", "secret: access denied", "", "Sign-in was cancelled", "CANCELLED by user", "cancelled later", "connection refused", "timeout again"] {
            assert_eq!(friendly(error), OTHER, "{error}");
        }
    }

    #[test]
    fn a_cancelled_sign_in_offers_skip_again_next_to_try_again() {
        let c = content(State::Failed(friendly("cancelled")));
        assert!(c.skip, "Skip is back after Cancel");
        assert_eq!(c.status, Some(Status::Error(CANCELLED)));
        assert_eq!((c.primary, c.secondary, c.note), (Some(TRY_AGAIN), None, None));
    }

    #[test]
    fn enter_follows_the_skip_focus_after_every_failure_and_never_acts_while_waiting() {
        for error in [CANCELLED, TIMEOUT, REJECTED, OTHER, UNREACHABLE] {
            assert_eq!(enter_action(State::Failed(error), false), Action::Start, "{error}");
            assert_eq!(enter_action(State::Failed(error), true), Action::Skip, "{error}");
        }
        for skip_focused in [false, true] {
            assert_eq!(enter_action(State::Waiting, skip_focused), Action::None);
        }
    }

    #[test]
    fn enter_can_skip_exactly_when_the_skip_button_is_shown() {
        let states = [State::Idle, State::Waiting, State::Failed(CANCELLED), State::Failed(TIMEOUT), State::Failed(UNREACHABLE)];
        for state in states {
            let shown = content(state).skip;
            assert_eq!(enter_action(state, true) == Action::Skip, shown, "{:?}", content(state).status);
            // While waiting, a sign-in is in flight: no Skip shown, and Enter does nothing at all.
            if !shown {
                assert_eq!(enter_action(state, false), Action::None);
            }
        }
    }

    #[test]
    fn the_window_height_at_and_around_each_limit() {
        // 460 + 80 = 540 and 760 + 80 = 840 are the two corners.
        for (visible, want) in [
            (539.0, 460.0),
            (540.0, 460.0),
            (541.0, 461.0),
            (839.0, 759.0),
            (840.0, 760.0),
            (841.0, 760.0),
            (80.0, 460.0),
            (-0.0, 460.0),
            (-500.0, 460.0),
            (f32::MIN, 460.0),
            (f32::NEG_INFINITY, 460.0),
            (f32::MAX, 760.0),
            (f32::INFINITY, 760.0),
            (f32::MIN_POSITIVE, 460.0),
        ] {
            assert_eq!(window_height(Some(visible)), want, "{visible}");
        }
        assert_eq!(window_height(None), 760.0);
    }

    #[test]
    fn a_display_that_reports_no_usable_height_still_gets_a_window_between_460_and_760() {
        let h = window_height(Some(f32::NAN));
        assert!(h.is_finite() && (460.0..=760.0).contains(&h), "{h}");
    }
}
