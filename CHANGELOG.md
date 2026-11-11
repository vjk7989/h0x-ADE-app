# Changelog

## [Unreleased]

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
