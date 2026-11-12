//! The frame's core screens, shown instead of the main window until a core
//! that can serve this app runs:
//! - none found (core_bin.rs): install it for me (the core's own installer),
//!   or point at one;
//! - found but it won't start or answer (its own error, with a retry);
//! - too old (or missing a method the app calls): update it (`h0x-core update`,
//!   terminals kept);
//! - a newer generation than this app speaks: update the app.
//! Then the main window opens.

use crate::core_link::{Core, Mismatch};
use crate::mods::Request;
use crate::settings::{self, color as c};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::path::PathBuf;
use std::sync::Arc;

const ICON: &[u8] = include_bytes!("../assets/icons/h0x-ade-icon.png");
#[cfg(not(windows))]
const APP_INSTALL: &str = "curl -fsSL https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/install.sh | sh";
#[cfg(windows)]
const APP_INSTALL: &str = "powershell -c \"irm https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/install.ps1 | iex\"";
const CORE_PLACE: &str = if cfg!(windows) { r"%LOCALAPPDATA%\Programs\h0x-core" } else { "~/.local/bin" };

/// What's missing before the main window can open.
pub enum Need {
    Install,
    /// The core is there but failed to start; carries its own message and hint.
    Connect(String),
    Fix(Mismatch),
}

impl Need {
    /// A core that reports its own startup error gets that error and a retry;
    /// one that can't be run or read at all gets the install screen.
    pub fn connection(error: anyhow::Error) -> Self {
        match error.downcast_ref::<crate::core_link::ConnectError>() {
            Some(error) => Self::Connect(plain_text(&error.to_string())),
            None => Self::Install,
        }
    }
}

/// The core's words as shown to the person: no control characters (new lines
/// stay) and no bidi or zero-width characters that could disguise the text.
fn plain_text(text: &str) -> String {
    let hidden = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}' | '\u{FEFF}');
    text.chars().filter(|&c| c == '\n' || !(c.is_control() || hidden(c))).collect()
}

#[cfg(test)]
mod tests {
    use super::{Need, plain_text};

    #[test]
    fn the_cores_words_are_shown_without_control_or_bidi_characters() {
        assert_eq!(plain_text("a\u{1b}[31mred\u{7}\r\n\tb"), "a[31mred\nb");
        assert_eq!(plain_text("x\u{202E}gpj.exe\u{2066}y\u{200B}z\u{200F}\u{FEFF}"), "xgpj.exeyz");
        assert_eq!(plain_text("line\n\nhint \u{e9} \u{4e2d}"), "line\n\nhint \u{e9} \u{4e2d}");
        assert_eq!(plain_text(""), "");
        let error = anyhow::Error::new(crate::core_link::ConnectError("bad\u{202E}\u{1b}\n\nhint".into()));
        assert!(matches!(Need::connection(error), Need::Connect(text) if text == "bad\n\nhint"));
    }

    #[test]
    fn unreadable_core_connections_offer_installation_instead_of_retry() {
        let error = anyhow::anyhow!("bad hello reply from core: EOF");
        assert!(matches!(Need::connection(error), Need::Install));
    }

    #[test]
    fn structured_core_errors_keep_the_recovery_hint_and_retry() {
        let message = "the core pipe cannot be accessed

try again without elevation";
        let error = anyhow::Error::new(crate::core_link::ConnectError(message.into())).context("connecting");
        assert!(matches!(Need::connection(error), Need::Connect(text) if text == message));
    }

    #[test]
    fn a_missing_binary_or_a_refused_hello_is_not_a_reachable_core_error() {
        let missing = anyhow::anyhow!("no such file").context("cannot run `h0x-core connect`: is h0x-core installed?");
        assert!(matches!(Need::connection(missing), Need::Install));
        let refused = anyhow::anyhow!("core refused hello: {{\"message\":\"nope\"}}");
        assert!(matches!(Need::connection(refused), Need::Install));
    }

    #[test]
    fn an_empty_core_message_still_retries() {
        let error = anyhow::Error::new(crate::core_link::ConnectError(String::new()));
        assert!(matches!(Need::connection(error), Need::Connect(text) if text.is_empty()));
    }
}

enum State {
    Ready,
    Busy(&'static str),
    Failed(String),
}

pub struct Setup {
    need: Need,
    state: State,
    requests: Option<async_channel::Receiver<Request>>,
    icon: Arc<Image>,
}

pub fn open(need: Need, requests: async_channel::Receiver<Request>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(620.0), px(460.0)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("HyperOx".into()), ..Default::default() }),
        app_id: Some("h0x-ade".into()),
        ..Default::default()
    };
    let icon = Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec()));
    cx.open_window(options, |_, cx| cx.new(|_| Setup { need, state: State::Ready, requests: Some(requests), icon }))
        .expect("cannot open the HyperOx window");
    cx.activate(true);
}

impl Setup {
    /// Connects to the core; if it serves this app, closes this window and
    /// opens the main one, else says what's still needed.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match Core::connect() {
            Ok(core) => match crate::core_link::check(&core.hello) {
                Ok(()) => {
                    let Some(requests) = self.requests.take() else { return };
                    window.remove_window();
                    crate::open_main(core, requests, cx);
                    return;
                }
                Err(m) => {
                    self.need = Need::Fix(m);
                    self.state = State::Ready;
                }
            },
            Err(e) => {
                self.need = Need::connection(e);
                self.state = State::Ready;
            }
        }
        cx.notify();
    }

    /// Runs `work` off the UI thread, then `start` again (or shows the error).
    fn run(&mut self, label: &'static str, work: fn() -> Result<Option<PathBuf>, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.state = State::Busy(label);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(bin) => {
                    if let Some(bin) = bin {
                        crate::core_bin::use_core(bin);
                    }
                    this.start(window, cx);
                }
                Err(e) => {
                    this.state = State::Failed(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run("Installing h0x-core…", || crate::core_bin::install().map(Some).inspect_err(|e| crate::analytics::report("core_install_failed", "install", e)), window, cx);
    }

    fn update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run("Updating h0x-core… your terminals keep running.", || crate::core_bin::update().map(|_| None).inspect_err(|e| crate::analytics::report("core_update_failed", "update", e)), window, cx);
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Use this h0x-core".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(bin) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if crate::core_bin::version(&bin).is_none() {
                    this.state = State::Failed(format!("{} isn't h0x-core (it should answer `--version` with `h0x-core …`)", bin.display()));
                    cx.notify();
                    return;
                }
                // Remembered: next time HyperOx looks there first.
                let mut person = settings::load_person().unwrap_or_default();
                person.core = Some(bin.to_string_lossy().into_owned());
                let _ = settings::write_person(&person);
                crate::core_bin::use_core(bin);
                this.start(window, cx);
            });
        })
        .detach();
    }

    /// Title, what's going on, and the line under the buttons.
    fn words(&self) -> (String, String, String) {
        match &self.need {
            Need::Install => (
                "HyperOx needs h0x-core".into(),
                "h0x-core runs your terminals and agents, and keeps them alive when this window closes. HyperOx couldn't find it on this computer.".into(),
                format!("Install it for me puts it in {CORE_PLACE}, like its own installer, after checking the release's signature."),
            ),
            Need::Connect(error) => (
                "HyperOx can't connect to h0x-core".into(),
                error.clone(),
                "Once you've resolved this, try again.".into(),
            ),
            Need::Fix(Mismatch::TooOld { have, need }) => (
                "HyperOx needs a newer h0x-core".into(),
                format!("Yours is {have}; this HyperOx needs {need} or newer. Updating keeps your terminals running."),
                "Or update it yourself: h0x-core update".into(),
            ),
            Need::Fix(Mismatch::Missing(methods)) => (
                "HyperOx needs a newer h0x-core".into(),
                format!("Your h0x-core can't do everything this HyperOx needs ({}). Updating keeps your terminals running.", methods.join(", ")),
                "Or update it yourself: h0x-core update".into(),
            ),
            Need::Fix(Mismatch::Newer { generation }) => (
                "This HyperOx is older than your h0x-core".into(),
                format!("Your h0x-core speaks a newer protocol (generation {generation}) than this HyperOx understands. Update HyperOx; your terminals keep running."),
                format!("Update HyperOx: {APP_INSTALL}"),
            ),
        }
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

impl Render for Setup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = matches!(self.state, State::Busy(_));
        let (title, body, foot) = self.words();
        let actions = match &self.need {
            Need::Install => div()
                .flex()
                .gap(px(10.0))
                .child(button("install", "Install it for me", true).when(!busy, |d| d.on_click(cx.listener(|this, _, w, cx| this.install(w, cx)))))
                .child(button("choose", "I have it somewhere…", false).when(!busy, |d| d.on_click(cx.listener(|this, _, w, cx| this.choose(w, cx))))),
            Need::Connect(_) => div().child(button("retry", "Retry", true).when(!busy, |d| d.on_click(cx.listener(|this, _, w, cx| this.start(w, cx))))),
            Need::Fix(Mismatch::Newer { .. }) => div()
                .flex()
                .gap(px(10.0))
                .child(button("quit", "Quit", false).on_click(cx.listener(|_, _, _, cx| cx.quit()))),
            Need::Fix(_) => div()
                .flex()
                .gap(px(10.0))
                .child(button("update", "Update h0x-core", true).when(!busy, |d| d.on_click(cx.listener(|this, _, w, cx| this.update(w, cx))))),
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
                            .child(div().text_size(px(22.0)).font_weight(FontWeight::SEMIBOLD).child(title)),
                    )
                    .child(div().text_size(px(14.0)).line_height(px(21.0)).text_color(c("muted")).child(body))
                    .child(actions)
                    .child(div().text_size(px(12.5)).line_height(px(19.0)).text_color(c("dim")).child(foot))
                    .child(match &self.state {
                        State::Ready => div(),
                        State::Busy(what) => div().text_size(px(13.0)).text_color(c("accent")).child(*what),
                        State::Failed(e) => div().text_size(px(13.0)).text_color(c("blocked")).child(e.clone()),
                    }),
            )
    }
}
