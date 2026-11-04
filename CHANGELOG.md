# Changelog

## [Unreleased]

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
