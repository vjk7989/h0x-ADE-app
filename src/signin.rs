//! The sign-in screen: shown instead of the main window until a Pavii account is signed in.
//! The browser does the signing in (auth.rs); this window only waits for it. Then the main
//! window opens.

use crate::mods::Request;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::sync::Arc;
use std::time::Duration;

/// The HyperOx mark, the same asset the workspace chrome ships.
const MARK: &[u8] = include_bytes!("../defaults/mods/workspace-chrome/hyperox-mark.svg");
const OPEN_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6"/><path d="M20 4 11 13"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/></svg>"##;
const ERROR_ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5"/><path d="M12 16.5v.01"/></svg>"##;
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
const SIGN_IN_NOTE: &str = "Opens pavii.tech. Use Google, GitHub or email.";
const WAITING_TITLE: &str = "Finish signing in using your browser";
const WAITING_BODY: &str = "Then come back to this window.";
const TEAM_URL: &str = "https://pavii.tech";
const PARTNER_URL: &str = "https://buckleson.com";

const CANCELLED: &str = "Sign-in was cancelled.";
const TIMEOUT: &str = "Sign-in took too long. Please try again.";
const REJECTED: &str = "This account can't sign in right now (it may be disabled). Try another account.";
const OTHER: &str = "Something went wrong while signing in. Please try again.";
const UNREACHABLE: &str = "Couldn't reach Pavii \u{2014} check your connection and try again.";

/// Whether the main window must wait for a sign-in. Test and debug builds can skip it with
/// `H0X_ADE_SKIP_SIGNIN=1` (the e2e tests run headless); release builds never can.
pub fn required() -> bool {
    decide(bypass(), crate::auth::signed_in)
}

fn bypass() -> bool {
    cfg!(any(test, debug_assertions)) && std::env::var_os("H0X_ADE_SKIP_SIGNIN").is_some_and(|v| v == "1")
}

/// The account is only looked up when there is no bypass.
fn decide(bypass: bool, signed_in: impl FnOnce() -> bool) -> bool {
    !bypass && !signed_in()
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
}

pub fn open(core: Arc<crate::core_link::Core>, requests: async_channel::Receiver<Request>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(760.0), px(520.0)), cx))),
        window_min_size: Some(size(px(620.0), px(460.0))),
        titlebar: Some(TitlebarOptions { title: Some("HyperOx".into()), ..Default::default() }),
        app_id: Some("h0x-ade".into()),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.new(|_| SignIn { state: State::Idle, attempt: 0, core: Some(core), requests: Some(requests), focus })
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
        if matches!(self.state, State::Waiting) {
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
            "enter" => self.start(window, cx),
            "escape" => self.cancel(cx),
            "tab" if ev.keystroke.modifiers.shift => window.focus_prev(cx),
            "tab" => window.focus_next(cx),
            _ => {}
        }
    }
}

/// What the card shows for a state: which status line, buttons and note.
#[derive(Debug, PartialEq)]
struct Content {
    status: Option<Status>,
    primary: Option<&'static str>,
    secondary: Option<&'static str>,
    note: Option<&'static str>,
}

#[derive(Debug, PartialEq)]
enum Status {
    Waiting,
    Error(&'static str),
}

fn content(state: State) -> Content {
    match state {
        State::Idle => Content { status: None, primary: Some(SIGN_IN), secondary: None, note: Some(SIGN_IN_NOTE) },
        State::Waiting => Content { status: Some(Status::Waiting), primary: None, secondary: Some("Cancel"), note: None },
        State::Failed(e) => Content { status: Some(Status::Error(e)), primary: Some(TRY_AGAIN), secondary: None, note: None },
    }
}

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

/// A fully rounded 44px button with a 2px focus ring when reached by keyboard.
fn pill(id: &'static str, label: &'static str, primary: bool, icon: Option<&'static [u8]>) -> Stateful<Div> {
    let ring = vec![
        BoxShadow::new(px(0.0), px(0.0), hex(PANEL)).spread_radius(px(2.0)),
        BoxShadow::new(px(0.0), px(0.0), hex(FOCUS)).spread_radius(px(4.0)),
    ];
    div()
        .id(id)
        .tab_index(0)
        .h(px(44.0))
        .px(px(24.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .rounded(px(999.0))
        .border_1()
        .cursor_pointer()
        .text_size(px(15.0))
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
        .cursor_pointer()
        .underline()
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
            .child(div().flex_1().min_h(px(12.0)).max_h(px(56.0)))
            .child(div().text_size(px(34.0)).line_height(px(36.0)).font_weight(FontWeight::NORMAL).text_color(hex(INK)).child("Welcome to HyperOx"))
            .child(div().mt(px(12.0)).max_w(px(300.0)).text_size(px(16.0)).line_height(px(25.0)).text_color(hex(INK_2)).child(TAGLINE))
            .child(
                div()
                    .mt(px(32.0))
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
            .child(div().flex_1().min_h(px(8.0)))
            .child(
                div()
                    .flex()
                    .flex_col()
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
    use super::{CANCELLED, OTHER, REJECTED, SIGN_IN, SIGN_IN_NOTE, State, Status, TIMEOUT, TRY_AGAIN, UNREACHABLE, content, decide, friendly};

    #[test]
    fn idle_offers_sign_in_with_a_note_and_nothing_else() {
        let c = content(State::Idle);
        assert_eq!((c.primary, c.secondary, c.note), (Some(SIGN_IN), None, Some(SIGN_IN_NOTE)));
        assert_eq!(c.status, None);
    }

    #[test]
    fn waiting_shows_the_spinner_and_only_cancel() {
        let c = content(State::Waiting);
        assert_eq!((c.primary, c.secondary, c.note), (None, Some("Cancel"), None));
        assert_eq!(c.status, Some(Status::Waiting));
    }

    #[test]
    fn a_failure_shows_the_friendly_message_and_try_again() {
        for error in ["timeout", "cancelled", "network: x", "rejected: x", "other"] {
            let c = content(State::Failed(friendly(error)));
            assert_eq!(c.status, Some(Status::Error(friendly(error))));
            assert_eq!((c.primary, c.secondary), (Some(TRY_AGAIN), None));
        }
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
}
