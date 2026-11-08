# Changelog

## [Unreleased]

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
