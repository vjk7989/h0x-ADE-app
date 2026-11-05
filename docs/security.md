# Security and privacy

## Mods have your access

A mod is trusted JavaScript running inside HyperOx. It can run programs with your environment, fetch URLs and read files.
The built-in file API limits writes to the app home, but a mod can run a program to write elsewhere. That is not a security boundary.
Mods can also run programs on connected machines.

There is no permission prompt for each program or network request. Read a mod before installing it, and treat it like a shell script you would run yourself.
Layouts can load mods. Check those references too.

## What stays on disk

The default app home is `~/.config/h0x-ade`, including on macOS and Windows (under your user home).
`XDG_CONFIG_HOME` changes the config root. `H0X_ADE_HOME` selects an app home directly and takes precedence.

This folder holds your settings, keys, layouts, mods, generated agent guide and API types.
It also holds mod data, shared state, window positions, this screen's focus and zoom, update notes and cached media.
Mods may store sensitive data here. HyperOx does not encrypt this folder.
The [installed guide's file map](../defaults/AGENTS.md#the-files) explains the files.

Spaces, tabs and splits are stored in each machine's core document, not in the app home.
Terminals and their output belong to [h0x-core](https://github.com/vjk7989/h0x-ADE-core).
Removing the app home does not stop the core or erase its data.

The app exposes a local control socket in its home so agents can inspect and drive the window.
Treat access to that home as access to the window and its commands.
Remote connections use SSH through the core.

## Updates and signing

Installed releases check GitHub Releases, download an update, and check its SHA-256 checksum against the release manifest before unpacking it.
The app switches to the downloaded version on quit. Source builds do not update themselves.
Core updates are handled separately by the core's updater.

Every release file the app reads (the manifest, the update, its notes, the core installer) has a minisign signature beside it (`<file>.minisig`).
The app checks it against the release public key built into it (key id `29957E064EAB76C4`) and refuses a file whose signature is missing, unreadable or by another key.
The signature must also name the file it was made for (`file:<name>` in its trusted comment), so one signed file cannot be served in place of another.
Downloads go over HTTPS only, to GitHub's release hosts (`github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`); a redirect anywhere else is refused.
An older signed manifest can still be served in place of the newest one; the app then sees no newer version, it never installs an older one.

The release pipeline supports Apple Developer ID signing on macOS and Azure artifact signing on Windows when their credentials are configured.
Without them, packages can be unsigned or ad hoc signed.
On macOS the installer and updater verify certificate-signed bundles and preserve their signatures.
Apple notarization is tested separately; it is not required for publishing a release.
Do not assume every download is notarized or signed.

## What HyperOx sends

HyperOx sends no usage telemetry. Nothing leaves your computer unless you ask for it: update checks (above), network requests made by mods you install, and feedback you choose to send.
Feedback is drafted for you to review; sending opens a prefilled issue on the HyperOx GitHub repository in your browser, and nothing is posted by the app itself.

[Back to HyperOx](../README.md)
