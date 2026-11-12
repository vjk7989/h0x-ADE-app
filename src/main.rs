// Windows: a window app, no console of its own (see `attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod agent_usage;
mod analytics;
mod bench;
mod commands;
mod control;
mod core_bin;
mod files;
mod core_link;
mod keys;
mod launch;
mod layout;
mod layout_sync;
mod links;
mod model;
mod mirror;
mod mirror_sync;
mod place;
mod persist;
#[cfg(target_os = "macos")]
mod macos;
mod mods;
mod mod_files;
mod os_notice;
mod process_cwd;
mod programs;
mod remote_shell;
mod settings;
mod setup;
mod sound;
mod spaces_share;
mod spawn;
mod sprites;
mod feedback;
mod terminal;
mod update;
mod update_notes;
mod vt;
mod win32_keys;
mod window;

use gpui::*;

const HELP: &str = "\
h0x-ade: the HyperOx desktop app. With no arguments it opens the window.

Usage:
  h0x-ade                        open the window
  h0x-ade ls                     spaces, their tabs and terminals, in the open window
  h0x-ade do <command> [args]    run a command: --name api --argv '[\"pi\"]', or one JSON object
  h0x-ade feedback <text>        draft feedback for the person to review and send
  h0x-ade commands               every command, its arguments, what it does
  h0x-ade status                 what the window shows, and any errors
  h0x-ade shot [file.png]        save the last window frame (default: h0x-shot.png)
  h0x-ade check <mod folder>     load a mod once and report what's wrong
  h0x-ade --skill                Agent Skill: driving the window, and where h0x-core fits
  h0x-ade --version

The terminals run in h0x-core, which keeps them alive when the
window closes. `h0x-core --help`: start, read, wait on and talk to terminals and
agents. The app shows them and arranges them in spaces and tabs.

Home: ~/.config/h0x-ade (H0X_ADE_HOME picks another).
";

/// A Windows window app has no console: answers to `h0x-ade ls` and the
/// like go to the terminal it was started from. Output already piped (agents,
/// scripts) keeps its pipes.
/// True when started from a shell: the output is already a pipe or file, or
/// the parent's console was attached.
#[cfg(windows)]
fn attach_console() -> bool {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // Attaching would point the output at the console instead of the pipe or file.
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if out.is_null() || out == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
    }
    true
}

fn main() {
    analytics::install_crash_hook();
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.len() > 1 {
        attach_console();
    }
    // `h0x-ade check <mod>` / `h0x-ade types <mod>`: gpui-shell's own checker, with HyperOx's module installed.
    if matches!(args.get(1).map(String::as_str), Some("check" | "types")) {
        mods::install_check_policy();
        gpui_shell::host::main_with_components(gpui_shell::FrozenComponentRegistry::default());
        return;
    }
    if matches!(args.get(1).map(String::as_str), Some("do" | "status" | "ls" | "commands" | "feedback" | "shot")) {
        std::process::exit(control::client(&args[1..]));
    }
    // Anything else from a shell answers in the shell; only a bare launch opens the window.
    let mut quiet = false;
    match args.get(1).map(String::as_str) {
        // Already open on this home: that window comes to the front.
        // `--launched-in <folder>` is the detached copy a shell launch starts on unix: a bare launch in that folder.
        None | Some("--launched-in") => {
            let detached = args.len() > 1;
            let from_shell = !detached && launch::from_shell();
            if detached {
                launch::init_with(args.get(2).cloned());
            } else {
                launch::init(from_shell);
            }
            let folder = launch::folder();
            if control::raise_running(folder.as_deref()) {
                if from_shell {
                    launch::wait(launch::announce(&launch::running_line(folder.as_deref()), folder.as_deref()));
                }
                return;
            }
            if from_shell {
                let _notice = launch::announce(&launch::started_line(folder.as_deref()), folder.as_deref());
                // The window runs in a detached copy; this process is done once the notification is out.
                #[cfg(unix)]
                if launch::detach(folder.as_deref()).is_ok() {
                    launch::wait(_notice);
                    return;
                }
                // The started line is all the terminal gets: let go of it, and log nowhere unless asked.
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::System::Console::FreeConsole();
                }
                quiet = std::env::var_os("H0X_ADE_LOG").is_none();
            }
        }
        Some("-h" | "--help" | "help") => return print!("{HELP}"),
        Some("--skill") => return print!("{}", include_str!("SKILL.md")),
        Some("-V" | "--version" | "version") => return println!("h0x-ade {}", env!("CARGO_PKG_VERSION")),
        Some(other) => {
            eprint!("h0x-ade: unknown argument {other:?}\n\n{HELP}");
            std::process::exit(2);
        }
    }

    #[cfg(target_os = "macos")]
    core_bin::fill_path();
    let log = tracing_subscriber::fmt().with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("H0X_ADE_LOG")
                .unwrap_or_else(|_| "warn,h0x_ade=info,gpui_shell=info".into()),
        );
    if quiet {
        log.with_writer(std::io::sink).init();
    } else {
        log.init();
    }

    if update::apply_on_startup() {
        return;
    }

    // A first start sizes the app to the screen and the system's text size.
    let fresh = !settings::path().exists();
    // The core people installed; without one, the setup screen installs it.
    let person = settings::load_person();
    let configured = person.as_ref().ok().and_then(|p| p.core.clone());
    let core: Result<std::sync::Arc<core_link::Core>, setup::Need> = match core_bin::find(configured.as_deref()) {
        None => Err(setup::Need::Install),
        Some(bin) => {
            core_bin::use_core(bin);
            match core_link::Core::connect() {
                // One that can't serve this app gets the screen that fixes it.
                Ok(core) => core_link::check(&core.hello).map(|()| core).map_err(setup::Need::Fix),
                Err(e) => {
                    eprintln!("h0x-ade: {e:#}");
                    Err(setup::Need::connection(e))
                }
            }
        }
    };
    #[cfg(target_os = "macos")]
    core_bin::install_cli_link();
    if core.is_ok() {
        update::after_update();
    }
    // A first run: no update check, and no usage event, before the welcome has shown the setting.
    let welcomed = mods::shared("frame.welcomed").is_some();
    if !welcomed {
        update::hold_until_welcomed();
    }
    update::start();
    analytics::start(welcomed && person.as_ref().map_or(true, |p| p.share_analytics));
    analytics::event("app_started", serde_json::json!({}));
    bench::start();
    if let Err(e) = mods::write_workspace() {
        eprintln!("h0x-ade: cannot write {}: {e}", mods::app_dir().display());
    }

    gpui_platform::application()
        .with_assets(gpui_shell::AppAssets::new(mods::app_dir()))
        .run(move |cx| {
            let fonts = settings::BUNDLED_FONTS.iter().map(|f| std::borrow::Cow::Borrowed(*f)).collect();
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                tracing::warn!("bundled fonts not loaded: {e:#}");
            }
            settings::note_fonts(cx.text_system().all_font_names());
            settings::load_home_fonts(cx);
            mods::init(cx);
            // Not restarted for it: the update goes in when the app quits.
            cx.on_app_quit(|cx| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
                // Where the window was, for the next start (a quit never asks the window).
                for w in cx.windows() {
                    if let Some(w) = w.downcast::<window::AppWindow>() {
                        let _ = w.update(cx, |this, window, cx| {
                            this.drain_documents(window, cx);
                            this.close_mods_now(deadline, window, cx);
                            place::save(window, cx);
                        });
                    }
                }
                mods::finish_shared();
                analytics::event("app_closed", serde_json::json!({ "session_seconds": analytics::session_seconds() }));
                analytics::flush();
                update::apply();
                async {}
            })
            .detach();
            #[cfg(target_os = "macos")]
            macos::init(cx);
            if fresh {
                let width = cx.primary_display().map(|d| f32::from(d.bounds().size.width));
                let zoom = settings::first_zoom(width);
                if zoom != 1.0 {
                    let _ = settings::write_person(&settings::Person { zoom, ..Default::default() });
                }
            }
            let requests = mods::install_policy();
            if let Some(tx) = mods::request_sender() {
                control::listen(tx);
            }
            match core {
                Ok(core) => open_main(core, requests, cx),
                Err(need) => setup::open(need, requests, cx),
            }
        });
}

/// The main window, on a connected core.
pub fn open_main(core: std::sync::Arc<core_link::Core>, requests: async_channel::Receiver<mods::Request>, cx: &mut App) {
            let (window_bounds, display_id) = place::initial(cx);
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                display_id,
                // macOS: always see-through, so a layout can draw its own title bar
                // live; the app drags the window (window/titlebar.rs).
                titlebar: Some(TitlebarOptions {
                    title: Some("HyperOx".into()),
                    // Windows: hidden for good (it can't change live); the app draws it.
                    appears_transparent: cfg!(any(target_os = "macos", windows)),
                    traffic_light_position: cfg!(target_os = "macos").then(|| point(px(12.0), px(7.0))),
                }),
                app_owns_titlebar_drag: cfg!(target_os = "macos"),
                // Linux: open with the title bar the shipped layouts draw. Turning the
                // system's off on the first render, while KWin maps the window, shrinks
                // it to 20x20; turning it on later (a "system" layout) is fine.
                window_decorations: Some(WindowDecorations::Client),
                // HyperOx Dev and other side-by-side builds get their own dock icon.
                app_id: Some(std::env::var("H0X_ADE_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "h0x-ade".into())),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| cx.new(|cx| window::AppWindow::new(core, requests, window, cx)))
                .expect("cannot open the HyperOx window");
            cx.activate(true);
}

#[cfg(test)]
mod tests {
    #[test]
    fn skill_printed_by_dash_dash_skill_uses_h0x_names_only() {
        let skill = include_str!("SKILL.md");
        assert!(!skill.trim().is_empty());
        assert!(!skill.to_ascii_lowercase().contains("herdr"));
        assert!(!skill.contains("HC_"));
        assert!(skill.contains("$H0X_TERMINAL_ID"));
    }
}
