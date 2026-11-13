//! The sign-in screen: shown instead of the main window until a Pavii account is signed in.
//! The browser does the signing in (auth.rs); this window only waits for it. Then the main
//! window opens.

use crate::mods::Request;
use crate::settings::{self, color as c};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::sync::Arc;

const ICON: &[u8] = include_bytes!("../assets/icons/h0x-ade-icon.png");
const CANCELLED: &str = "Sign-in was cancelled.";
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
    if error.to_ascii_lowercase().contains("cancel") { CANCELLED } else { UNREACHABLE }
}

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
    icon: Arc<Image>,
}

pub fn open(core: Arc<crate::core_link::Core>, requests: async_channel::Receiver<Request>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(620.0), px(460.0)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("HyperOx".into()), ..Default::default() }),
        app_id: Some("h0x-ade".into()),
        ..Default::default()
    };
    let icon = Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec()));
    cx.open_window(options, |_, cx| {
        cx.new(|_| SignIn { state: State::Idle, attempt: 0, core: Some(core), requests: Some(requests), icon })
    })
    .expect("cannot open the HyperOx window");
    cx.activate(true);
}

impl SignIn {
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.attempt += 1;
        self.state = State::Failed(CANCELLED);
        cx.notify();
    }
}

fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(38.0))
        .px(px(18.0))
        .flex()
        .items_center()
        .rounded(px(9.0))
        .cursor_pointer()
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .when(primary, |d| d.bg(c("accent")).text_color(c("accent_foreground")).hover(|s| s.opacity(0.9)))
        .when(!primary, |d| d.border_1().border_color(c("border")).text_color(c("foreground")).hover(|s| s.bg(c("raised"))))
        .child(label)
}

impl Render for SignIn {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let waiting = matches!(self.state, State::Waiting);
        let actions = if waiting {
            div().flex().gap(px(10.0)).child(button("cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
        } else {
            div().child(button("signin", "Sign in with your browser", true).on_click(cx.listener(|this, _, w, cx| this.start(w, cx))))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(c("background"))
            .text_color(c("foreground"))
            .font_family(settings::get().ui_font_family.clone())
            .child(
                div()
                    .w(px(520.0))
                    .flex()
                    .flex_col()
                    .gap(px(18.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .child(img(self.icon.clone()).w(px(48.0)).h(px(48.0)))
                            .child(div().text_size(px(22.0)).font_weight(FontWeight::SEMIBOLD).child("Sign in to HyperOx")),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(px(21.0))
                            .text_color(c("muted"))
                            .child("Use your Pavii account \u{2014} Google, GitHub or email."),
                    )
                    .child(actions)
                    .child(match &self.state {
                        State::Idle => div(),
                        State::Waiting => div().text_size(px(13.0)).text_color(c("accent")).child("Finish signing in in your browser\u{2026}"),
                        State::Failed(e) => div().text_size(px(13.0)).text_color(c("blocked")).child(*e),
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cancelled_and_unreachable_have_their_own_words() {
        assert_eq!(friendly("Sign-in was cancelled"), CANCELLED);
        assert_eq!(friendly("CANCELLED by user"), CANCELLED);
        assert_eq!(friendly("connection refused"), UNREACHABLE);
        assert_eq!(friendly(""), UNREACHABLE);
    }
}
