# Changelog

## [Unreleased]

## [0.1.26] - 2026-11-13

Sign in with your Pavii account. Bundles h0x-core 0.1.16.

### Added
- Sign in to HyperOx with your Pavii account (Google, GitHub or email). HyperOx opens www.pavii.tech in your browser and you return with one click; the sign-in is kept encrypted on your computer and lasts 30 days offline.
- Several accounts can be signed in on one computer: switch between them, sign out, or remove one from this computer, from Settings → Info or the HyperOx logo menu.
- A redesigned Settings → Info page with your account, resources, partners and privacy in one place.
- Pairing a phone or Telegram needs a signed-in account.
- Hold the mouse over a file link in a terminal to see "Ctrl+click to open", and Ctrl+click file names like plan.md to open them in Files.

### Changed
- Usage analytics are linked to your HyperOx account ID once you're signed in (never your email).

### Fixed
- HyperOx finds its own runtime instead of another program named h0x.
- Settings → Updates says "Not checked yet." before the first check instead of showing an error.
- The welcome window closes properly.

## [0.1.25] - 2026-11-13

Updates that look after themselves, and a better to-do list. Bundles h0x-core 0.1.16.

### Added
- HyperOx keeps the app, the core and the runtime up to date by itself. The core and runtime update once no agent has worked for 2 minutes; the app updates when you quit. A short notice says what was updated.
- If a new version doesn't start properly, HyperOx goes back to the previous one by itself and tells you. A version that fails twice is skipped until a newer one comes out.
- Usage analytics and failure reports help us fix problems during the beta. Turn usage analytics off any time in Settings → Info; the "i" next to the switch explains exactly what is sent. Failure reports stay on.
- To-do: Undo and OK buttons after deleting a task (6 seconds), and drag tasks and sub-tasks to move them, including onto another task.
- Ctrl+click a file name in a terminal to open it: text, code and Markdown open in HyperOx, other files in your usual program.
- A one-time "How did you find HyperOx?" question at the end of the welcome.

### Fixed
- After double-clicking a to-do to edit it, typing goes straight into the box.

## [0.1.24] - 2026-11-12

Read long to-do tasks in full. Bundles h0x-core 0.1.15.

### Added

- Click a task or sub-task in the to-do list to see its whole text over several lines; click it again to go back to one line. Double-click still edits it.

## [0.1.23] - 2026-11-12

A sharper logo and image paste on Windows. Bundles h0x-core 0.1.15.

### Fixed

- The HyperOx logo at the top left is sharp in every theme and follows the theme's text colour.
- On Windows, Ctrl+V with a screenshot on the clipboard now works in a terminal: the image is saved as a file and its path is pasted, so Claude Code and other agents attach it.

## [0.1.22] - 2026-11-12

A sturdier to-do list. Bundles h0x-core 0.1.15.

### Fixed

- A to-do save that fails retries by itself (after 5 s, growing to once a minute) instead of waiting for your next change.
- A save interrupted halfway recovers by itself instead of blocking further saves.
- Notes or front matter above a list's `# Title` stay above it, and a `# comment` inside front matter is no longer taken as the list name.
- The Congratulations pop-up now follows what was actually saved, also when another program edits the list at the same time.
- Big lists save faster.

## [0.1.21] - 2026-11-12

New themes, a nicer to-do list and Telegram you can navigate. Bundles h0x-core 0.1.15.

### Added

- Six new themes in Settings → Look: Amber Minimal, Candyland, Claymorphism and Darkmatter (dark), Soft Pop and Neo Brutalism (light). The existing themes stay.
- Telegram: the bot's menu button lists its commands; `/menu`, `/sessions` and `/projects` (your saved spaces) let you pick a session and type into it, with the same safety checks as `/send`.

### Changed

- The to-do list: ticking a task ticks its steps and ticking the last step ticks the task; done items are struck through; tasks are written in a handwritten font (Patrick Hand); a progress bar, card rows and friendlier empty and all-done states.
- Telegram pairing: the bot token saves itself when you paste it (or press Enter), pairing sits right beside it, the `/pair` code is large with a Copy button, and a How to use list explains the commands.
- In light themes the top-left logo is a dark tile with the white mark.

### Fixed

- To-do lists: deleting, switching, creating and renaming a list, and starting a new index, no longer fail with "cx is no longer valid"; a rename no longer stops saving; the Enter that closes a box no longer reopens it.

## [0.1.20] - 2026-11-11

A to-do list. Bundles h0x-core 0.1.14. (0.1.19 was tagged but never published: a flaky test stopped its release.)

### Added

- **To-do** next to Files and Changes, or `Ctrl+Shift+J` (`Cmd+Shift+J` on macOS): several lists, tasks with steps, a folded "Completed" section, and undo for deletes.
- Keyboard first: arrows move, `Space` ticks, `Enter` edits, `N` adds, `Tab`/`Shift+Tab` make a step or a task, `Alt+↑/↓` reorder.
- Finishing the last open task shows a small Congratulations pop-up with an optional Celebrate.
- Lists are Markdown files in the app's folder (`todo/`), the same in every space; agents can update them and the panel follows along. A list holds up to 64 KB (about 2,900 tasks).
- Settings → Look can hide the to-do list; your lists are kept.

## [0.1.18] - 2026-11-11

A new Info page. Bundles h0x-core 0.1.14.

### Changed

- Settings → Info is redone: HyperOx, what it is, an installation guide and documentation, feedback (coming soon), built by Team Pavii, with Buckleson as security and blockchain partner, and the versions in use.

### Security

- Mods installed by a person can no longer use the names reserved for the app's own pages.

## [0.1.17] - 2026-11-11

A clean start from the terminal. Bundles h0x-core 0.1.14.

### Changed

- Typing `h0x-ade` in a terminal shows only `HyperOx started in <folder>.` and then gives the terminal back: no log lines follow, you can keep using the terminal or close it, and HyperOx keeps running.
- macOS and Linux: HyperOx runs on its own after a terminal launch instead of inside that terminal.

## [0.1.16] - 2026-11-11

Open HyperOx where you are. Bundles h0x-core 0.1.14.

### Added

- Typing `h0x-ade` in a terminal opens HyperOx in that terminal's folder: the window shows a space for the folder (an existing one is reused) with its terminal there. The terminal says `HyperOx started in <folder>.` and a "HyperOx has been launched" notification appears.
- When HyperOx is already open, `h0x-ade` brings the window to the front and opens the folder in it, then gives the terminal back.

### Changed

- Opening HyperOx from the Start menu, Desktop, Dock or file manager works as before.

### Fixed

- Windows: the window no longer risks running out of stack space while it opens.

## [0.1.15] - 2026-11-11

Security release from the final QA round. Bundles h0x-core 0.1.14.

### Security

- Terminal engine updated to Ghostty 1.4.0, which fixes two terminal-escape security issues (GHSA-jqr7, GHSA-w9q4).
- The mods script engine is updated to QuickJS-ng 0.16.2, which fixes known memory-safety bugs.
- Installers only download over https from the official release pages, install the core version this app was tested with, and check the app archive's signature before unpacking it. The setup screen installs the core with its signature check on, and no longer suggests piping a script into your shell.

### Changed

- The Info link points to Buckleson (https://github.com/buckleson).
- On a first run the automatic update check waits until the welcome screen is closed. "Check now" and turning the check off work as before.
- Mods' commands find h0x-core on their own PATH; the app no longer changes its own PATH while running.

## [0.1.14] - 2026-11-10

Performance release. Bundles h0x-core 0.1.13 (opt-in `keep_awake` in core's config.toml).

### Changed

- Scrollback per pane is a setting (Settings → Look → Terminal, 1,000 to 100,000 lines; default unchanged at 10,000). Wide panes and long wrapped lines keep fewer lines than the number set.
- Mods pause their timers while the window is minimized or hidden, and catch up when it's back; a visible but unfocused window keeps updating.
- Smaller release builds (thin link-time optimization, stripped symbols).

### Fixed

- Settings never overwrites a settings.json it can't read (it used to replace it with a single key, losing your other settings); a settings file with a byte-order mark loads.

## [0.1.13] - 2026-11-09

Fixes ported from upstream herdr-app, hardened and tested. Bundles h0x-core 0.1.11.

### Changed

- When h0x-core is installed but fails to start, the app shows its error with a Retry button instead of the install screen; a core that never answers is stopped after 20 seconds.
- Add machine prints the exact command to run, quoted for PowerShell on Windows and for your shell elsewhere.
- Mods reuse their code across copies, fixing a slow memory leak that eventually stopped all mods; an edited mod still asks for your OK, and old code versions are released.
- Layouts can choose the notifications mod; `pane.swap` swaps panes by id.

### Fixed

- A terminal that ends or can't be reached keeps its place in the layout.
- Replays wait for the pane's size, so long lines are no longer cut.
- Ctrl+W (Cmd+W) closes the open dialog, not the terminal behind it.
- The app only uses an h0x-core at an absolute path, never one in the current folder or on a relative PATH entry.
- Add machine shows its instructions when h0x-core is missing on the other machine.
- Layouts from another machine can no longer choose your notification or terminal-menu mods.

## [0.1.12] - 2026-11-09

Security release from the second audit (Cloudflare security-audit tool).

### Changed

- Mods you added yourself may ask for your OK once more: the check now covers every file a mod can run, mods can no longer load their helper files as code, and a mod with more than 20,000 files or 512 MiB is refused.
- Files dropped or pasted into a terminal are quoted for the shell actually running there (PowerShell, cmd, fish, POSIX shells); behind ssh, tmux or similar, unsafe names are refused.

### Fixed

- Layouts from another machine can no longer load images from network shares or crash the app.
- Pasted images go to a private temporary folder.
- The Files panel's git calls no longer run repository hooks; the app's control socket uses your account's SID.

### Requires

- h0x-core 0.1.10.

## [0.1.11] - 2026-11-09

Security release from a full audit.

### Changed

- Settings → Look → Terminal: "Let programs copy to the clipboard" is off by default (programs could otherwise replace your clipboard with a command you might paste).
- Settings → Updates: "Check for updates automatically" (on by default); "Check now" always works.
- Settings → Remote shows the phone panel's warning about who delivers the page.
- Mods you added that pull in code from a git repository no longer run; fonts load only from mods you allowed.
- Updates install exactly the files that were verified.

### Fixed

- Reading a file on a remote Windows machine can no longer run a command, and network-share paths are refused there.
- Pasted text can no longer leave paste mode; dropped paths with curly quotes, `-`, `@` or `,` are quoted correctly for PowerShell.
- The app's control socket is limited to your account and SYSTEM, with request size and time limits.
- Sessions "Reveal" refuses paths Explorer would split; worktree submodule paths must stay inside the repository; panel git commands run without external diff or text conversion.
- The notification sound is written to a private temporary folder.

### Requires

- h0x-core 0.1.9.

## [0.1.10] - 2026-11-08

### Changed

- Settings → Remote: the Telegram bot token is deleted from disk right after it is handed to the core, and any leftover from an interrupted save is removed first.

### Requires

- h0x-core 0.1.8: the phone panel is end-to-end encrypted, and bridge secrets are kept in the system's secret store. Phones paired with an older version must scan the QR code once more.

## [0.1.9] - 2026-11-08

### New

- Settings → Remote: connect Telegram (paste the bot token, pair your account) and start the phone web panel with a QR code, on the same Wi-Fi, through a Cloudflare tunnel or through your own VPS. It also lists paired devices and recent activity, and has Lock, Unlock and Unpair.

### Fixed

- The Files panel previews text files again in every folder of a space, instead of showing "Cannot preview this file". When a preview fails, it now says why.

### Requires

- h0x-core 0.1.7.

## [0.1.8] - 2026-11-08

### New

- A small "Beta" label next to the HyperOx name.
- Ctrl+Shift+Q quits on Windows and Linux.
- Includes upstream fixes: kept sub-agents listed once (with their busy word), confirm dialogs sized to content with rounded corners, pane navigation while zoomed, find highlighting wide characters, a second rename press closes the prompt, Linux window decorations and input-method fixes.

### Requires

- h0x-core 0.1.6, which adds remote control via Telegram and a phone web panel.

## [0.1.7] - 2026-11-06

### Changed

- Lower idle CPU: hidden Files and Changes panels stop polling git, panels redraw only when something changed, and the Sessions panel reuses its rows.

## [0.1.6] - 2026-11-05

### New

- Sessions panel: browse past Claude Code, Codex and OpenCode sessions for each space. The ⋯ menu offers Resume, Resume in new tab, Pin, Rename, Copy resume command, Show file and Delete (Remove from HyperOx, or move to the Recycle Bin/Trash). Spaces show a count badge.
- Save a layout and restore it together with its agents; "Resume all agents" brings them back after a reboot.
- Rotating HyperOx busy words while an agent works.
- Windows 98 theme: HyperOx wording, logo and banner.

### Fixed

- Tasks with quotes or several lines reach the agent intact on Windows.
- Requires h0x-core 0.1.5.

## [0.1.5] - 2026-11-05

### New

- Includes upstream 0.1.13 improvements: find in a terminal's screen and scrollback, a draggable scrollbar on terminal panes, and terminal.scroll for mods and agents.

### Fixed

- The command palette keeps scrolling past group titles and shows ⌘ only on macOS.
- Sidebar rail items sit centred on one line, and the changes list fills the panel and scrolls through every changed file.
- The settings dialog draws its rounded background, and long theme names stay inside their cards.
- Escape reaches the program when the terminal has focus, and the keyboard returns to the focused pane when a stack shows its tab again.
- Faster agent and worktree rendering.
- Windows terminal links open in the default browser or mail app, still limited to web and mail addresses.
- Requires h0x-core 0.1.4.

## [0.1.4] - 2026-11-05

### Fixed

- The HyperOx logo shows again in the sidebar header and in What's new.

## [0.1.3] - 2026-11-05

### New

- New icon: the HyperOx mark on a white rounded tile, so it shows on dark taskbars, docks and the welcome screen.

### Fixed

- Lower idle CPU: the update status is rebuilt only when something changes.
- Requires h0x-core 0.1.3 (faster terminal output).

## [0.1.2] - 2026-11-05

### Fixed

- Security: a remote machine's layout can no longer place panels that start programs on this computer.
- Terminal links refuse Windows network-object paths and URLs with characters Windows Explorer splits on.
- Update signatures are tied to the exact file they sign.
- Update notes now load (releases publish a signed changelog).
- Requires h0x-core 0.1.2, which fixes remote-machine boundaries.

## [0.1.1] - 2026-11-05

### New

- Updates and the core installer are signed (minisign) and checked before they are installed.
- Mods that did not ship with HyperOx ask once before they run.

### Fixed

- Downloads happen only over HTTPS from GitHub's release hosts.
- Terminal links open only web and mail addresses; files are shown in their folder, never launched.
- Mods may only use your home folder, the HyperOx folder and your space folders.
- The control socket and the app home are private to you on Linux and macOS.
- Agent notes drop control characters from file names.
- Screenshots must be .png files.
- Security updates for dependencies (rustls, quinn-proto, zstd, tokio).

## [0.1.0] - 2026-11-04

### New

- First HyperOx release.
