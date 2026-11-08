# HyperOx

HyperOx is a desktop app for terminals and coding agents. Spaces hold your projects, tabs hold the agents and shells you run in them, panes split a tab. A sidebar shows which agents need you. Terminals keep running when you close the window: they live in the h0x-core runtime, a small background program, and are back where you left them when you open the window again.

## Install

HyperOx is in Beta: expect rough edges and breaking changes between releases.

### Linux and macOS

```sh
curl -fsSL https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/install.sh | sh
```

Linux needs a desktop session and a Vulkan-capable GPU driver; `h0x-ade` goes into `~/.local/bin`, so make sure that is on your PATH. macOS builds are for Apple Silicon; the installer puts `HyperOx.app` in `~/Applications`.

### Windows

```powershell
irm https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/install.ps1 | iex
```

The app lands in `%LOCALAPPDATA%\Programs\h0x-ade` and is added to your PATH. No admin needed; open a new terminal afterwards.

On every platform the installer also installs `h0x-core`, the runtime that runs your terminals, if there is none yet. The app's own files live in `~/.config/h0x-ade`.

### Manual download

Pick the file for your platform from the [latest release](https://github.com/vjk7989/h0x-ADE-app/releases/latest):

| Platform | Asset |
|---|---|
| Linux x86_64 / aarch64 | `h0x-ade-linux-x86_64.tar.gz` / `h0x-ade-linux-aarch64.tar.gz` |
| macOS Apple Silicon | `h0x-ade-macos-aarch64.tar.gz` |
| Windows x86_64 | `h0x-ade-windows-x86_64.tar.gz` |

Prefer the installers: they also place the launcher, menu entry and `h0x-core`.

### Verify a download

Every release file has a [minisign](https://jedisct1.github.io/minisign/) signature beside it (`<file>.minisig`, same folder). The public key is:

```text
RWTEdqtOBn6VKQ/x9x3qWk/gIWUb/I1OlZXsTlHyrkp9Yq30qBki+tdt
```

```sh
base=https://github.com/vjk7989/h0x-ADE-app/releases/latest/download
curl -fsSLO $base/h0x-ade-linux-x86_64.tar.gz
curl -fsSLO $base/h0x-ade-linux-x86_64.tar.gz.minisig
minisign -Vm h0x-ade-linux-x86_64.tar.gz -P RWTEdqtOBn6VKQ/x9x3qWk/gIWUb/I1OlZXsTlHyrkp9Yq30qBki+tdt
```

The installers and the in-app updater check these signatures themselves.

## Get started

Run `h0x-ade` (or open HyperOx from the Start menu or Applications folder). The app installs `h0x-core` on first run if it is missing. Add a space for a project folder, then open a tab and start your agent or a shell in it.

## Find your work

- **Spaces** are projects: a folder, on this machine or another one. Switch between them in the sidebar.
- **Tabs** inside a space are the things you have running there: an agent, a shell, a dev server. Name them.
- **Panes** split a tab. Open a terminal beside the agent that is working for you.

The agents sidebar lists every agent with its state: **Working**, **Blocked** (it is waiting on you), **Done** (it finished while you were elsewhere) or **Idle**. Blocked and Done play a sound and show a toast; peek at an agent without leaving your current tab, or jump straight to it.

Every terminal has a short address, `t7` for one on this machine and `vps/t7` for one on a machine called `vps`. Agents and scripts use these addresses to talk to each other through `h0x-core`.

## Edit files

Open a file from the **Files** panel and type: it saves itself a second after you stop, and right away when you switch files, close the panel or press `Ctrl+S` (`Cmd+S` on macOS). A dot before the name means there are unsaved edits. Line endings stay as the file had them.

If an agent changes the file while you have unsaved edits, HyperOx stops saving that file and asks: **Reload** takes the version on disk, **Keep mine** writes yours over it. A file you have not touched just reloads.

After you save, idle agents in the same space get a one-line note naming the files you edited, so they re-read them before changing anything. Busy agents get it once they are idle. Files on another machine open read-only for now.

## Work on another machine

Open **Settings → Machines** and pick a host from your SSH config. HyperOx connects over SSH, installs `h0x-core` there if it is missing, and lists that machine in the sidebar with its own spaces. Agents on a server keep running while your laptop sleeps.

## Make it yours

- **Command palette**: `Ctrl+Shift+P` (`Cmd+Shift+P` on macOS) lists every command with its arguments.
- **Settings**: look, sounds, machines, updates, and the raw `settings.json` in your app home.
- **Layouts**: HyperOx (the default), Terminals (just terminals), and HyperOx 98 (a whole desktop in one mod). Clone one and change anything.
- **Mods** are small JavaScript programs that draw panels, overlays and windows. The shipped ones are examples; your agent can write new ones. `AGENTS.md` in your app home is the guide, written for agents and people alike.
- **Keys**: `keys.json` in your app home rebinds anything; see [the keys section](defaults/AGENTS.md#keys-keysjson) of the mod guide for the format, including JSON objects for named, typed command arguments.

## Updates and privacy

Installed releases check GitHub Releases for new versions and tell you in the corner; the app switches on quit, and `h0x-core` updates without stopping your terminals. Source builds do not update themselves.

HyperOx sends no usage telemetry and nothing else unless you ask it to: update checks, network requests made by mods you install, and feedback you review before sending. See [security and privacy](docs/security.md).

## For agents

An agent working inside HyperOx can drive the window from a shell:

```sh
h0x-ade --skill      # the manual: how to inspect and change the window
h0x-ade ls           # spaces, their tabs and terminals, including pane IDs
h0x-ade status       # what is on screen, and any errors
h0x-ade commands     # every command, its arguments, what it does
h0x-ade do tab.new --name "fix login" --argv '["claude"]'
```

Starting, reading and waiting on terminals is `h0x-core`'s job: `h0x-core --skill` prints its manual.

## Remote control

`h0x-core` can let a paired chat account see and drive your terminals while you are away. See [Remote control (bridge)](https://github.com/vjk7989/h0x-ADE-core/blob/main/docs/bridge.md).

## Build from source

Rust 1.95 and Zig 0.16.0 (for cross builds and the C dependencies).

```sh
git clone --recurse-submodules https://github.com/vjk7989/h0x-ADE-app
cd h0x-ADE-app
cargo build --locked
cargo test --locked
```

The app needs a running `h0x-core` of the matching generation; `distribution/core-version` names the release it expects. Mods are plain files under `defaults/`: edit, save, and the running window reloads them.

## Part of HyperOx

- [h0x-core](https://github.com/vjk7989/h0x-ADE-core): the runtime that runs terminals and agents and keeps them alive.
- [h0x](https://github.com/vjk7989/h0x-ADE-Runtime): the terminal multiplexer, for the same terminals inside a TUI.
- This repository: the desktop app.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE) for attributions.
