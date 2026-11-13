//! The sign-in screen: shown instead of the main window until this computer has signed in or
//! skipped once (auth.rs keeps the marker). The browser does the signing in; this window only
//! waits for it. Then the main window opens.

use crate::mods::Request;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::sync::Arc;
use std::time::Duration;

/// The HyperOx mark, the same asset the workspace chrome ships.
const MARK: &[u8] = include_bytes!("../defaults/mods/workspace-chrome/hyperox-mark.svg");
const OPEN_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6"/><path d="M20 4 11 13"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/></svg>"##;
const ERROR_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5"/><path d="M12 16.5v.01"/></svg>"##;
const TOKEN_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2 4 14h7l-1 8 9-12h-7z"/></svg>"##;
const OBSERVE_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>"##;
const SHIELD_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/></svg>"##;
const SPIN_TRACK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.5"><circle cx="12" cy="12" r="9"/></svg>"##;
const SPIN_ARC: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 0 1 9 9"/></svg>"##;

// The screen is light whatever the app theme says, so these are fixed, not theme colors.
const PAGE_FROM: u32 = 0xfaf9f5;
const PAGE_TO: u32 = 0xacdae4;
const PANEL: u32 = 0xfffefb;
const INK: u32 = 0x0e1f2c;
const INK_2: u32 = 0x2e3f4f;
const MUTED: u32 = 0x576570;
const RULE_2: u32 = 0x85939d;
const ACCENT: u32 = 0x006c9c;
const ACCENT_HI: u32 = 0x005a8b;
const ACCENT_INK: u32 = 0xfcfcf9;
const ACCENT_SOFT: u32 = 0xdaf0f7;
const PAPER_2: u32 = 0xf4f3ef;
const ERROR: u32 = 0xa8372e;
const FOCUS: u32 = 0x007fb9;
const CAST: u32 = 0x3f8fa6;

const TAGLINE: &str = "Your one-stop ADE for development, security and observability.";
const SIGN_IN: &str = "Sign in with your browser";
const TRY_AGAIN: &str = "Try again";
const PERKS_LEAD: &str = "Signing in turns on";
const PERKS: [(&[u8], &str); 3] =
    [(TOKEN_ICON, "Token saving"), (OBSERVE_ICON, "Observability engine"), (SHIELD_ICON, "Blockchain security for PII masking")];
const SKIP: &str = "Skip sign-in";
const SKIP_NOTE: [&str; 2] = ["Skipping turns these off.", "Sign in later in Settings \u{2192} Profile."];
const SIGN_IN_NOTE: &str = "Opens pavii.tech. Use Google, GitHub or email.";
const WAITING_TITLE: &str = "Finish signing in using your browser";
const WAITING_BODY: &str = "Then come back to this window.";
const TEAM_URL: &str = "https://pavii.tech";
const PARTNER_URL: &str = "https://buckleson.com";

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
}

/// The window height: 640, or less on a short screen (visible height minus 80, never below 460).
fn window_height(visible: Option<f32>) -> f32 {
    visible.filter(|h| !h.is_nan()).map_or(640.0, |h| (h - 80.0).clamp(460.0, 640.0))
}

pub fn open(core: Arc<crate::core_link::Core>, requests: async_channel::Receiver<Request>, cx: &mut App) {
    let height = window_height(cx.primary_display().map(|d| f32::from(d.visible_bounds().size.height)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(760.0), px(height)), cx))),
        window_min_size: Some(size(px(620.0), px(460.0))),
        titlebar: Some(TitlebarOptions { title: Some("HyperOx".into()), ..Default::default() }),
        app_id: Some("h0x-ade".into()),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let skip_focus = cx.focus_handle().tab_stop(true);
        cx.new(|_| SignIn { state: State::Idle, attempt: 0, core: Some(core), requests: Some(requests), focus, skip_focus })
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

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

fn ring() -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.0), px(0.0), hex(PANEL)).spread_radius(px(2.0)),
        BoxShadow::new(px(0.0), px(0.0), hex(FOCUS)).spread_radius(px(4.0)),
    ]
}

/// A fully rounded button with a 2px focus ring when reached by keyboard: 44px, or the quieter
/// 40px Skip pill.
fn pill(id: &'static str, label: &'static str, primary: bool, icon: Option<&'static [u8]>) -> Stateful<Div> {
    let ring = ring();
    let quiet = id == "signin-skip";
    div()
        .id(id)
        .tab_index(0)
        .h(px(if quiet { 40.0 } else { 44.0 }))
        .px(px(if quiet { 20.0 } else { 24.0 }))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .rounded(px(999.0))
        .border_1()
        .cursor_pointer()
        .text_size(px(if quiet { 14.0 } else { 15.0 }))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |d| d.bg(hex(ACCENT)).border_color(hex(ACCENT)).text_color(hex(ACCENT_INK)).hover(|s| s.bg(hex(ACCENT_HI))))
        .when(!primary, |d| d.bg(hex(PANEL)).border_color(hex(RULE_2)).text_color(hex(INK)).hover(|s| s.bg(hex(PAPER_2))))
        .focus_visible(move |mut s| {
            s.box_shadow = Some(ring);
            s
        })
        .when_some(icon, |d, icon| d.child(svg().data(icon).size(px(18.0)).flex_none()))
        .child(label)
}

fn link(id: &'static str, label: &'static str, url: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .cursor_pointer()
        .underline()
        .focus_visible(|mut s| {
            s.box_shadow = Some(ring());
            s
        })
        .on_click(move |_, _, _| crate::links::open(&crate::links::Target::Url(url.to_string())))
        .child(label)
}

fn spinner() -> Div {
    div()
        .relative()
        .flex_none()
        .size(px(18.0))
        .mt(px(1.0))
        .child(svg().data(SPIN_TRACK).absolute().size_full().text_color(hex(ACCENT_SOFT)))
        .child(
            svg()
                .data(SPIN_ARC)
                .absolute()
                .size_full()
                .text_color(hex(ACCENT))
                .with_animation("spin", Animation::new(Duration::from_millis(900)).repeat(), |s, delta| {
                    s.with_transformation(Transformation::rotate(percentage(delta)))
                }),
        )
}

impl Render for SignIn {
    /// Light whatever the app theme says: the sign-in screen has its own look.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = content(self.state);
        let idle = matches!(self.state, State::Idle);
        let status = view.status.map(|status| {
            let (title, body, color) = match &status {
                Status::Waiting => (WAITING_TITLE, Some(WAITING_BODY), INK),
                Status::Error(e) => (*e, None, ERROR),
            };
            let lead = match status {
                Status::Waiting => spinner().into_any_element(),
                Status::Error(_) => svg().data(ERROR_ICON).flex_none().size(px(18.0)).mt(px(1.0)).text_color(hex(ERROR)).into_any_element(),
            };
            div().flex().gap(px(12.0)).items_start().child(lead).child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().text_size(px(15.0)).line_height(px(21.0)).font_weight(FontWeight::MEDIUM).text_color(hex(color)).child(title))
                    .when_some(body, |d, body| d.child(div().mt(px(4.0)).text_size(px(14.0)).line_height(px(21.0)).text_color(hex(MUTED)).child(body))),
            )
        });
        let mut card = div()
            .id("signin-card")
            .overflow_y_scroll()
            .absolute()
            .left(px(40.0))
            .top(px(40.0))
            .bottom(px(40.0))
            .w(px(412.0))
            .p(px(32.0))
            .flex()
            .flex_col()
            .bg(hex(PANEL))
            .rounded(px(22.0));
        card.style().box_shadow = Some(vec![
            BoxShadow::new(px(18.0), px(28.0), hex(CAST).opacity(0.55)).blur_radius(px(48.0)).spread_radius(px(-26.0)),
            BoxShadow::new(px(2.0), px(3.0), hex(INK).opacity(0.12)).blur_radius(px(6.0)).spread_radius(px(-2.0)),
        ]);
        let card = card
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(svg().data(MARK).w(px(18.0)).h(px(21.0)).flex_none().text_color(hex(ACCENT)))
                    .child(div().text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD).text_color(hex(INK)).child("HyperOx")),
            )
            // Gives way first when the window is short.
            .child(div().flex_1().min_h(px(12.0)).max_h(px(40.0)))
            .child(div().text_size(px(34.0)).line_height(px(36.0)).font_weight(FontWeight::NORMAL).text_color(hex(INK)).child("Welcome to HyperOx"))
            .child(div().mt(px(12.0)).max_w(px(300.0)).text_size(px(16.0)).line_height(px(25.0)).text_color(hex(INK_2)).child(TAGLINE))
            .child(
                div()
                    .id("signin-perks")
                    .mt(px(20.0))
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.0)).line_height(px(20.0)).text_color(hex(INK_2)).child(PERKS_LEAD))
                    .child(div().mt(px(6.0)).flex().flex_col().gap(px(6.0)).children(PERKS.map(|(icon, label)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(svg().data(icon).size(px(16.0)).flex_none().text_color(hex(ACCENT)))
                            .child(div().text_size(px(14.0)).line_height(px(20.0)).font_weight(FontWeight::MEDIUM).text_color(hex(INK)).child(label))
                    }))),
            )
            .child(
                div()
                    .mt(px(24.0))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(16.0))
                    .when_some(status, |d, s| d.child(s))
                    .when_some(view.primary, |d, label| {
                        let id = if idle { "signin" } else { "retry" };
                        d.child(pill(id, label, true, Some(OPEN_ICON)).on_click(cx.listener(|this, _, w, cx| this.start(w, cx))))
                    })
                    .when_some(view.secondary, |d, label| d.child(pill("cancel", label, false, None).on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))))
                    .when_some(view.note, |d, note| d.child(div().text_size(px(13.0)).line_height(px(20.0)).text_color(hex(MUTED)).child(note))),
            )
            .when(view.skip, |d| {
                d.child(
                    div()
                        .mt(px(16.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(pill("signin-skip", SKIP, false, None).track_focus(&self.skip_focus).on_click(cx.listener(|this, _, w, cx| this.skip(w, cx))))
                        .child(
                            div()
                                .id("signin-skip-note")
                                .text_size(px(13.0))
                                .line_height(px(18.0))
                                .text_color(hex(MUTED))
                                .children(SKIP_NOTE.map(|line| div().child(line))),
                        ),
                )
            })
            .child(div().flex_1().min_h(px(8.0)))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(13.0))
                    .line_height(px(21.0))
                    .text_color(hex(MUTED))
                    .child(div().flex().child("Made by ").child(link("team", "Team Pavii", TEAM_URL)))
                    .child(div().flex().child("Security partner ").child(link("partner", "Buckleson", PARTNER_URL))),
            );
        div()
            .id("signin-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev, window, cx| this.key_down(ev, window, cx)))
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family("Geist")
            .text_color(hex(INK))
            // The one gradient: a 2-stop wash, filled natively by the element background.
            .bg(linear_gradient(110.0, linear_color_stop(hex(PAGE_FROM), 0.38), linear_color_stop(hex(PAGE_TO), 1.0)))
            .child(div().absolute().right(px(60.0)).top(px(130.0)).w(px(220.0)).h(px(260.0)).child(svg().data(MARK).size_full().text_color(hex(PANEL))))
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in `gpui::test`, which `#[test]` would then expand forever.
    use super::{
        Action, CANCELLED, OTHER, PERKS, PERKS_LEAD, REJECTED, SIGN_IN, SIGN_IN_NOTE, SKIP_NOTE, State, Status, TIMEOUT, TRY_AGAIN, UNREACHABLE, content, decide,
        enter_action, friendly, window_height,
    };

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
