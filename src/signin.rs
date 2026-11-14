//! The sign-in screen: shown instead of the main window until this computer has signed in or
//! skipped once (auth.rs keeps the marker). The browser does the signing in; this window only
//! waits for it. Then the main window opens.
//!
//! The look is cook/SPEC.md section 10: the app theme, a frosted card, a dot-matrix mark and a
//! calm band of falling code at the bottom. Colors are theme tokens (`settings::color`) with alpha.

use crate::mods::Request;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The HyperOx mark, the same asset the workspace chrome ships.
const MARK: &[u8] = include_bytes!("../defaults/mods/workspace-chrome/hyperox-mark.svg");
// Lucide, stroke 1.5 (cook/SPEC.md section 9).
const EXTERNAL_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/></svg>"##;
const ERROR_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/></svg>"##;
const GAUGE_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/></svg>"##;
const ACTIVITY_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/></svg>"##;
const SHIELD_CHECK_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>"##;
const GLOBE_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></svg>"##;
const SHIELD_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/></svg>"##;
const ARROW_UP_RIGHT_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M7 7h10v10"/><path d="M7 17 17 7"/></svg>"##;
/// An arc of 270 degrees.
const SPIN_ARC: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 1 1-9 9"/></svg>"##;

const TAGLINE: &str = "Your one-stop ADE for development, security and observability.";
const SIGN_IN: &str = "Sign in with your browser";
const TRY_AGAIN: &str = "Try again";
const PERKS_LEAD: &str = "Signing in turns on";
const PERKS: [(&[u8], &str); 3] =
    [(GAUGE_ICON, "Token saving"), (ACTIVITY_ICON, "Observability engine"), (SHIELD_CHECK_ICON, "Blockchain security for PII masking")];
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
    /// The card's bounds from the last paint: the texture keeps clear of it.
    card: Rc<Cell<Option<Bounds<Pixels>>>>,
    rain: Rc<RefCell<RainPaint>>,
}

/// The window height: 640, or less on a short screen (visible height minus 80, never below 460).
fn window_height(visible: Option<f32>) -> f32 {
    visible.filter(|h| !h.is_nan()).map_or(640.0, |h| (h - 80.0).clamp(460.0, 640.0))
}

/// The window's size: 760 wide, `window_height` high.
fn window_size(height: f32) -> Size<Pixels> {
    size(px(760.0), px(height))
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

pub fn open(core: Arc<crate::core_link::Core>, requests: async_channel::Receiver<Request>, cx: &mut App) {
    let height = window_height(cx.primary_display().map(|d| f32::from(d.visible_bounds().size.height)));
    // The operating system's reduced-motion setting, for the code texture.
    gpui_base::apply_system_reduce_motion(cx);
    let options = window_options(WindowBounds::Windowed(Bounds::centered(None, window_size(height), cx)));
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
            card: Rc::new(Cell::new(None)),
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

    fn cancel(&mut self, cx: &mut Context<Self>) {
        // Only a wait can be cancelled (Esc is also pressed in the other states).
        if !matches!(self.state, State::Waiting) {
            return;
        }
        crate::auth::cancel_sign_in();
        self.attempt += 1;
        self.state = State::Failed(CANCELLED);
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
            "escape" => self.cancel(cx),
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

/// How the screen is arranged at a window size (cook/SPEC.md "Large sizes").
#[derive(Debug, PartialEq)]
struct Layout {
    /// Height of the code band.
    band: f32,
    /// The side column (mark and links) and the code texture show from 700 wide.
    side: bool,
    rain: bool,
    /// Taller than 700: the card group sits in the middle above the band, else at the top.
    centered: bool,
    /// Left padding of the footer, in line with the group's left edge.
    footer_left: f32,
}

fn layout(width: f32, height: f32) -> Layout {
    let k = if height <= 700.0 { 0.40 } else { 0.22 };
    Layout {
        band: ((height - 44.0) * k).round().clamp(120.0, 260.0),
        side: width >= 700.0,
        rain: width >= 700.0,
        centered: height > 700.0,
        footer_left: 28.0_f32.max((width - 920.0) / 2.0 + 28.0),
    }
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

fn paint_rain(state: &Rc<RefCell<RainPaint>>, card: Option<Bounds<Pixels>>, light: bool, frame: u64, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
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
    let card = card.map(|b| (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.origin.x + b.size.width), f32::from(b.origin.y + b.size.height)));
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    for c in 0..cols {
        for r in 0..rows {
            let Some((glyph, strength)) = s.rain.as_ref().and_then(|(_, rain)| rain.cell(c, r, frame)) else { continue };
            let (x, y) = (c as f32 * cell_w + 1.0, r as f32 * CELL_H);
            let distance = card.map_or(f32::MAX, |card| card_distance((left + x, top + y, cell_w, CELL_H), card));
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

const DOT_BOX: (f32, f32) = (96.0, 113.0);
const DOT_STEP: f32 = 4.0;
const DOT_RADIUS: f32 = 1.35;

/// The centres of the dots that draw the mark: its outline (read from the shipped SVG, straight
/// lines only) sampled on a 4 px grid in a 96 x 113 box, every point inside it (even-odd).
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

/// A 34px button with a focus ring while it has focus (not `focus_visible`: the ring must not hide after a mouse move).
fn button(id: &'static str, label: &'static str, primary: bool, icon: Option<&'static [u8]>) -> Stateful<Div> {
    let ring = ring();
    div()
        .id(id)
        .tab_index(0)
        .h(px(34.0))
        .px(px(14.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .rounded(px(9.0))
        .border_1()
        .cursor_pointer()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |d| d.bg(c("solid")).border_color(c("solid")).text_color(c("solid_foreground")).hover(|s| s.opacity(0.9)))
        .when(!primary, |d| {
            d.bg(c("foreground").opacity(0.06)).border_color(c("foreground").opacity(0.12)).text_color(c("foreground")).hover(|s| s.bg(c("foreground").opacity(0.10)))
        })
        .focus(move |mut s| {
            s.box_shadow = Some(ring);
            s
        })
        .child(label)
        .when_some(icon, |d, icon| d.child(svg().data(icon).size(px(14.0)).flex_none()))
}

fn open_url(url: &'static str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, _| crate::links::open(&crate::links::Target::Url(url.to_string()))
}

/// A footer credit (a narrow window has no side column): the domain, underlined.
fn credit(id: &'static str, domain: &'static str, url: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .cursor_pointer()
        .font_family(crate::settings::DEFAULT_FONT)
        .text_color(c("foreground"))
        .underline()
        .focus(|mut s| {
            s.box_shadow = Some(ring());
            s
        })
        .on_click(open_url(url))
        .child(domain)
}

/// A hairline row of the side column: icon, the domain over what it is, an arrow.
fn link_row(id: &'static str, icon: &'static [u8], domain: &'static str, note: &'static str, url: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .self_stretch()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(2.0))
        .py(px(9.0))
        .cursor_pointer()
        .hover(|s| s.bg(c("foreground").opacity(0.04)))
        .focus(|mut s| {
            s.box_shadow = Some(ring());
            s
        })
        .on_click(open_url(url))
        .child(svg().data(icon).size(px(15.0)).flex_none().text_color(c("muted")))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(div().font_family(crate::settings::DEFAULT_FONT).text_size(px(12.0)).line_height(px(16.0)).text_color(c("foreground")).child(domain))
                .child(div().text_size(px(11.5)).line_height(px(16.0)).text_color(c("muted")).child(note)),
        )
        .child(svg().data(ARROW_UP_RIGHT_ICON).size(px(13.0)).flex_none().text_color(c("dim")))
}

fn rule() -> Div {
    div().h(px(1.0)).self_stretch().bg(c("foreground").opacity(0.09))
}

fn spinner() -> Div {
    div().flex_none().size(px(14.0)).mt(px(3.0)).child(svg().data(SPIN_ARC).size_full().text_color(c("accent")).with_animation(
        "spin",
        Animation::new(Duration::from_millis(900)).repeat(),
        |s, delta| s.with_transformation(Transformation::rotate(percentage(delta))),
    ))
}

/// The soft tint behind the card: ten stacked ellipses, each a tenth of the peak, from the full size down to a tenth.
fn wash(tint: Hsla, peak: f32) -> Div {
    let (w, h) = (620.0_f32, 520.0_f32);
    div().absolute().left(px(-60.0)).top(px(-40.0)).w(px(w)).h(px(h)).children((0..10).map(move |i| {
        let k = 1.0 - i as f32 * 0.1;
        let (lw, lh) = (w * k, h * k);
        div().absolute().left(px((w - lw) / 2.0)).top(px((h - lh) / 2.0)).w(px(lw)).h(px(lh)).rounded(px(999.0)).bg(tint.opacity(peak / 10.0))
    }))
}

impl Render for SignIn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        let lay = layout(width, height);
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
                Status::Error(_) => svg().data(ERROR_ICON).flex_none().size(px(14.0)).mt(px(3.0)).text_color(c("blocked")).into_any_element(),
            };
            div()
                .self_stretch()
                .mb(px(6.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(10.0))
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
        let narrow = !lay.side;
        let perks = PERKS.iter().zip(BENEFITS).map(|((icon, label), benefit)| {
            div()
                .flex()
                .gap(px(12.0))
                .when(narrow, |d| d.items_center())
                .when(!narrow, |d| d.items_start())
                .child(
                    div()
                        .size(px(if narrow { 26.0 } else { 30.0 }))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(tint.opacity(0.30))
                        .child(svg().data(*icon).size(px(16.0)).text_color(tint)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(div().text_size(px(13.5)).line_height(px(18.0)).font_weight(FontWeight::SEMIBOLD).text_color(c("foreground")).child(*label))
                        .when(!narrow, |d| d.child(div().text_size(px(12.5)).line_height(px(17.0)).text_color(c("muted")).child(benefit))),
                )
        });
        let card_bounds = self.card.clone();
        let mut card = div()
            .id("signin-card")
            .relative()
            .w(px(440.0))
            .flex_none()
            .max_h(px(height - 36.0 - 28.0))
            .overflow_y_scroll()
            .px(px(24.0))
            .pt(px(22.0))
            .pb(px(20.0))
            .flex()
            .flex_col()
            .rounded(px(18.0))
            .border_1()
            .border_color(c("foreground").opacity(0.20));
        card.style().box_shadow = Some(vec![
            BoxShadow::new(px(0.0), px(30.0), black().opacity(if light { 0.22 } else { 0.65 })).blur_radius(px(60.0)).spread_radius(px(-20.0)),
            BoxShadow::new(px(0.0), px(2.0), black().opacity(if light { 0.08 } else { 0.30 })).blur_radius(px(8.0)),
        ]);
        let card = card
            // The frost: a translucent fill with a sheen on top, then the highlight line.
            .child(div().absolute().top(px(0.0)).left(px(0.0)).right(px(0.0)).bottom(px(0.0)).rounded(px(18.0)).bg(c("raised").opacity(if light { 0.70 } else { 0.58 })))
            .when(!light, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .right(px(0.0))
                        .bottom(px(0.0))
                        .rounded(px(18.0))
                        .bg(linear_gradient(180.0, linear_color_stop(white().opacity(0.09), 0.0), linear_color_stop(white().opacity(0.0), 0.4))),
                )
            })
            .child(div().absolute().top(px(0.0)).left(px(14.0)).right(px(14.0)).h(px(1.0)).bg(white().opacity(if light { 0.9 } else { 0.32 })))
            .child(canvas(
                |bounds, _, _| bounds,
                move |_, bounds, window, _| {
                    // The texture keeps clear of the card: when it moved or resized, paint once more.
                    if card_bounds.get() != Some(bounds) {
                        card_bounds.set(Some(bounds));
                        window.refresh();
                    }
                },
            )
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0)))
            .child(div().text_size(px(26.0)).line_height(px(32.0)).font_weight(FontWeight::SEMIBOLD).text_color(c("foreground")).child("Welcome to HyperOx"))
            .child(div().mt(px(6.0)).text_size(px(13.5)).line_height(px(20.0)).text_color(c("muted")).child(TAGLINE))
            .child(
                div()
                    .id("signin-perks")
                    .mt(px(18.0))
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(12.0)).line_height(px(16.0)).text_color(c("muted")).child(PERKS_LEAD))
                    .child(div().mt(px(10.0)).flex().flex_col().gap(px(10.0)).children(perks)),
            )
            .child(
                div()
                    .mt(px(18.0))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(8.0))
                    .when_some(status, |d, s| d.child(s))
                    .when_some(view.primary, |d, label| {
                        let id = if idle { "signin" } else { "retry" };
                        d.child(button(id, label, true, Some(EXTERNAL_ICON)).on_click(cx.listener(|this, _, w, cx| this.start(w, cx))))
                    })
                    .when_some(view.secondary, |d, label| d.child(button("cancel", label, false, None).on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))))
                    .when_some(view.note, |d, note| {
                        let row = div().flex().text_size(px(12.5)).line_height(px(18.0)).text_color(c("muted"));
                        d.child(match note.split_once(TEAM_DOMAIN) {
                            Some((before, after)) => row
                                .child(before)
                                .child(div().font_family(crate::settings::DEFAULT_FONT).text_size(px(11.5)).child(TEAM_DOMAIN))
                                .child(after),
                            None => row.child(note),
                        })
                    }),
            )
            .when(view.skip, |d| {
                d.child(div().mt(px(16.0)).self_stretch().child(rule())).child(
                    div()
                        .mt(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(14.0))
                        .child(button("signin-skip", SKIP, false, None).track_focus(&self.skip_focus).on_click(cx.listener(|this, _, w, cx| this.skip(w, cx))))
                        .child(div().id("signin-skip-note").text_size(px(12.5)).line_height(px(18.0)).text_color(c("muted")).children(SKIP_NOTE.map(|line| div().child(line)))),
                )
            });

        let side = lay.side.then(|| {
            div()
                .id("signin-side")
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .items_center()
                .self_start()
                .pt(px(96.0))
                .child(canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, window, _| {
                        for (x, y) in mark_dots() {
                            let at = bounds.origin + point(px(x - DOT_RADIUS), px(y - DOT_RADIUS));
                            window.paint_quad(fill(Bounds::new(at, size(px(DOT_RADIUS * 2.0), px(DOT_RADIUS * 2.0))), tint.opacity(0.95)).corner_radii(px(DOT_RADIUS)));
                        }
                    },
                )
                .w(px(DOT_BOX.0))
                .h(px(DOT_BOX.1))
                .flex_none())
                .child(
                    div()
                        .mt(px(110.0))
                        .self_stretch()
                        .flex()
                        .flex_col()
                        .child(rule())
                        .child(link_row("team", GLOBE_ICON, TEAM_DOMAIN, TEAM_NOTE, TEAM_URL))
                        .child(rule())
                        .child(link_row("partner", SHIELD_ICON, PARTNER_DOMAIN, PARTNER_NOTE, PARTNER_URL))
                        .child(rule()),
                )
        });

        let group = div()
            .relative()
            .w_full()
            .max_w(px(920.0))
            .px(px(28.0))
            .flex()
            .gap(px(28.0))
            .child(wash(tint, if light { 0.05 } else { 0.24 }))
            .child(card)
            .children(side);

        let hint_line = div().flex().items_center().gap(px(8.0)).font_family(crate::settings::DEFAULT_FONT).text_size(px(11.5)).children(hints(self.state).into_iter().enumerate().flat_map(
            |(i, (key, verb))| {
                let sep = (i > 0).then(|| div().text_color(c("dim")).child("\u{b7}").into_any_element());
                let item = div().flex().gap(px(6.0)).child(div().font_weight(FontWeight::MEDIUM).text_color(c("foreground")).child(key)).child(div().text_color(c("muted")).child(verb));
                sep.into_iter().chain(std::iter::once(item.into_any_element()))
            },
        ));
        let footer = div()
            .absolute()
            .bottom(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .h(px(36.0))
            .pl(px(lay.footer_left))
            .pr(px(28.0))
            .flex()
            .items_center()
            .gap(px(14.0))
            .when(narrow, |d| {
                d.justify_between().child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .font_family(crate::settings::DEFAULT_FONT)
                        .text_size(px(11.5))
                        .text_color(c("muted"))
                        .child(credit("team", TEAM_DOMAIN, TEAM_URL))
                        .child(" Team Pavii \u{b7} ")
                        .child(credit("partner", PARTNER_DOMAIN, PARTNER_URL))
                        .child(" partner"),
                )
            })
            .child(hint_line);

        let animating = rain_runs(cx.reduce_motion(), window.is_window_active(), lay.rain);
        let frame = shown_frame(self.frame, animating);
        let (card_now, rain_state) = (self.card.clone(), self.rain.clone());
        let band = lay.band;

        div()
            .id("signin-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev, window, cx| this.key_down(ev, window, cx)))
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family("Geist")
            .text_color(c("foreground"))
            .bg(c("background"))
            .when(lay.rain, |d| {
                d.child(
                    div().id("signin-rain").absolute().left(px(0.0)).right(px(0.0)).bottom(px(44.0)).h(px(band)).child(
                        canvas(
                            |bounds, _, _| bounds,
                            move |_, bounds, window, cx| paint_rain(&rain_state, card_now.get(), light, frame, bounds, window, cx),
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
                    .bottom(px(36.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .when(lay.centered, |d| d.justify_center().pb(px(band * 0.6)))
                    .when(!lay.centered, |d| d.pt(px(28.0)))
                    .child(group),
            )
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in `gpui::test`, which `#[test]` would then expand forever.
    use super::{
        Action, BENEFITS, CANCELLED, OTHER, PERKS, PERKS_LEAD, REJECTED, SIGN_IN, SIGN_IN_NOTE, SKIP_NOTE, State, Status, TIMEOUT, TRY_AGAIN, UNREACHABLE, content, decide,
        enter_action, friendly, ring, window_height,
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
        let bounds = WindowBounds::Windowed(Bounds::new(point(px(0.0), px(0.0)), super::window_size(640.0)));
        let options = super::window_options(bounds);
        assert!(!options.is_resizable, "no maximize box, no double-click or Win+Up maximize");
        assert!(options.is_minimizable);
        assert_eq!(options.window_min_size, Some(size(px(620.0), px(460.0))));
        assert!(options.window_bounds.is_some());
        assert_eq!(options.titlebar.and_then(|t| t.title).as_deref(), Some("HyperOx"));
        for (visible, high) in [(None, 640.0), (Some(1040.0), 640.0), (Some(500.0), 460.0)] {
            let s = super::window_size(window_height(visible));
            assert_eq!((f32::from(s.width), f32::from(s.height)), (760.0, high));
        }
    }

    #[test]
    fn the_layout_follows_the_window_size() {
        use super::{Layout, layout};
        assert_eq!(layout(760.0, 640.0), Layout { band: 238.0, side: true, rain: true, centered: false, footer_left: 28.0 });
        assert_eq!(layout(620.0, 460.0), Layout { band: 166.0, side: false, rain: false, centered: false, footer_left: 28.0 });
        assert_eq!(layout(699.0, 700.0).side, false);
        assert_eq!(layout(700.0, 700.0).side, true);
        assert!(!layout(1000.0, 700.0).centered && layout(1000.0, 701.0).centered, "taller than 700 centres the group");
        // The band: 40% of the room up to 700 high, 22% above, never under 120 or over 260.
        assert_eq!(layout(1000.0, 760.0).band, (716.0_f32 * 0.22).round());
        assert_eq!(layout(1920.0, 1080.0).band, 228.0);
        assert_eq!(layout(1920.0, 3000.0).band, 260.0);
        assert_eq!(layout(800.0, 200.0).band, 120.0);
        assert_eq!(layout(1920.0, 1080.0).footer_left, 28.0_f32.max((1920.0 - 920.0) / 2.0 + 28.0));
        assert_eq!(layout(1000.0, 760.0).footer_left, 68.0);
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
        assert!(dots.iter().all(|(x, y)| (0.0..96.0).contains(x) && (0.0..113.0).contains(y)));
        assert_eq!(super::dots_in(std::str::from_utf8(super::MARK).unwrap()), dots, "the same every time");
        assert!(super::dots_in("<svg/>").is_empty());
        // A square outline with a square hole (even-odd) leaves a ring.
        let ring = super::dots_in(r#"<svg viewBox="0 0 715 842"><path d="M0 0 715 0 715 842 0 842ZM170 200 545 200 545 640 170 640Z"/></svg>"#);
        let full = super::dots_in(r#"<svg viewBox="0 0 715 842"><path d="M0 0 715 0 715 842 0 842Z"/></svg>"#);
        assert!(ring.len() < full.len() && !ring.is_empty());
        assert!(!ring.contains(&(46.0, 58.0)), "the hole is empty");
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
        for id in ["signin-root", "signin-card", "signin-perks", "\"signin\"", "\"retry\"", "\"cancel\"", "\"signin-skip\"", "signin-skip-note", "\"team\"", "\"partner\"", "signin-rain", "signin-side"] {
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
    fn the_window_is_640_high_and_shrinks_on_a_short_screen_but_never_below_460() {
        assert_eq!(window_height(None), 640.0);
        assert_eq!(window_height(Some(1040.0)), 640.0);
        assert_eq!(window_height(Some(728.0)), 640.0);
        assert_eq!(window_height(Some(700.0)), 620.0);
        assert_eq!(window_height(Some(500.0)), 460.0);
        assert_eq!(window_height(Some(0.0)), 460.0);
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
        // 460 + 80 = 540 and 640 + 80 = 720 are the two corners.
        for (visible, want) in [
            (539.0, 460.0),
            (540.0, 460.0),
            (541.0, 461.0),
            (719.0, 639.0),
            (720.0, 640.0),
            (721.0, 640.0),
            (80.0, 460.0),
            (-0.0, 460.0),
            (-500.0, 460.0),
            (f32::MIN, 460.0),
            (f32::NEG_INFINITY, 460.0),
            (f32::MAX, 640.0),
            (f32::INFINITY, 640.0),
            (f32::MIN_POSITIVE, 460.0),
        ] {
            assert_eq!(window_height(Some(visible)), want, "{visible}");
        }
        assert_eq!(window_height(None), 640.0);
    }

    #[test]
    fn a_display_that_reports_no_usable_height_still_gets_a_window_between_460_and_640() {
        let h = window_height(Some(f32::NAN));
        assert!(h.is_finite() && (460.0..=640.0).contains(&h), "{h}");
    }
}
