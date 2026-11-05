# Changelog

## [Unreleased]

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
