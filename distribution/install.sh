#!/bin/sh
# Installs the HyperOx app, and h0x-core if there's none yet:
#
#   curl -fsSL https://github.com/vjk7989/h0x-ADE-app/releases/latest/download/install.sh | sh
#
# Linux: the app into ~/.local/share/h0x-ade, `h0x-ade` in ~/.local/bin,
# a launcher entry and icon. macOS: HyperOx.app into ~/Applications, keeping
# its release signature intact (ad hoc for unsigned/dev builds). The core, with its
# own installer, into ~/.local/bin/h0x-core only if none is there yet; the app
# keeps it current after that. Running it again updates the app.
# The core is the version this release pins (core_min in its manifest), never
# "latest". The app download must carry the release key's signature (its
# .minisig); h0x-core checks it before anything downloaded is unpacked, as the
# core's own installer does (which, on a first install, trusts TLS to GitHub
# and the checksum, documented there).
# H0X_ADE_RELEASES_URL (app) and H0X_CORE_RELEASES_URL (core) point elsewhere (tests).
set -eu

releases=${H0X_ADE_RELEASES_URL:-https://github.com/vjk7989/h0x-ADE-app/releases}
core_releases=${H0X_CORE_RELEASES_URL:-https://github.com/vjk7989/h0x-ADE-core/releases}
bin="$HOME/.local/bin"
# https only, and the downloads on GitHub; a site set by H0X_ADE_RELEASES_URL
# or H0X_CORE_RELEASES_URL (tests: a local http one) is trusted as it is, unsigned.
proto='=https'
[ -n "${H0X_ADE_RELEASES_URL:-}" ] && proto='=https,http'
core_proto='=https'
[ -n "${H0X_CORE_RELEASES_URL:-}" ] && core_proto='=https,http'
# Said loudly: set in a profile and forgotten, either one turns a check off.
[ -z "${H0X_ADE_RELEASES_URL:-}" ] || printf 'h0x-ade: WARNING: H0X_ADE_RELEASES_URL is set: downloads from that site are NOT signature-checked (unset it for a normal install).\n' >&2
[ -z "${H0X_CORE_RELEASES_URL:-}" ] || printf 'h0x-ade: WARNING: H0X_CORE_RELEASES_URL is set: the core is downloaded from that site and its signature is NOT checked (unset it for a normal install).\n' >&2

say() { printf '%s\n' "$*"; }
fail() { say "h0x-ade: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1"; }
need curl
need tar

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW* | MSYS* | CYGWIN*) fail "this installer is for Linux and macOS; on Windows, run this in PowerShell: irm $releases/latest/download/install.ps1 | iex" ;;
  *) fail "no build for $(uname -s)" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no build for $(uname -m)" ;;
esac
if [ "$os" = macos ] && [ "$arch" = x86_64 ]; then
  # Rosetta reports x86_64 even on Apple Silicon.
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] ||
     [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
    arch=aarch64
  else
    fail "Intel Macs aren't supported; build from source: https://github.com/vjk7989/h0x-ADE-app#build-from-source"
  fi
fi
target="$os-$arch"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL --proto "$proto" --proto-redir "$proto" "$releases/latest/download/latest.json" -o "$tmp/latest.json" || fail "cannot reach $releases"
# Read without jq: the manifest is flat enough for sed once spaces are gone.
flat=$(tr -d '\n ' < "$tmp/latest.json")
entry=$(printf '%s' "$flat" | sed -n "s/.*\"$target\":{\([^}]*\)}.*/\1/p")
url=$(printf '%s' "$entry" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
want=$(printf '%s' "$entry" | sed -n 's/.*"sha256":"\([^"]*\)".*/\1/p')
version=$(printf '%s' "$flat" | sed -n 's/^{"version":"\([^"]*\)".*/\1/p')
core_min=$(printf '%s' "$flat" | sed -n 's/.*"core_min":"\([^"]*\)".*/\1/p')
[ -n "$url" ] && [ -n "$want" ] || fail "no build for $target yet"
if [ -z "${H0X_ADE_RELEASES_URL:-}" ]; then
  case $url in
    https://github.com/* | https://objects.githubusercontent.com/* | https://release-assets.githubusercontent.com/*) ;;
    *) fail "refusing to download $url: not an https link on GitHub" ;;
  esac
fi

say "Installing HyperOx ${version:-} for $target..."
# Fetched exactly as the list gives it, under the release's own file name:
# its signature is bound to that name.
asset=${url%%[?#]*}
asset=${asset##*/}
archive="$tmp/$asset"
# get PROTO URL OUT SITE: https only, redirects too; unless SITE (a test site)
# is set, the URL it ended up at must be on GitHub as well.
get() {
  final=$(curl -fsSL --proto "$1" --proto-redir "$1" -w '%{url_effective}' -o "$3" "$2") || return 1
  [ -n "$4" ] && return 0
  case $final in
    '' | https://github.com/* | https://objects.githubusercontent.com/* | https://release-assets.githubusercontent.com/*) ;;
    *) rm -f "$3"; fail "refusing the download from $final: redirected off GitHub" ;;
  esac
}
get "$proto" "$url" "$archive" "${H0X_ADE_RELEASES_URL:-}" || fail "download failed"
[ "$(sha256 "$archive")" = "$want" ] || fail "download is damaged (checksum mismatch); try again"

# The core: installed only if there's none yet, at the version this release
# pins (never "latest"). One that's already there (yours, or from a package
# manager) is left alone; the app updates it. Its installer is downloaded to a
# file and run, not piped.
core_present() { [ -e "$bin/h0x-core" ] || command -v h0x-core >/dev/null 2>&1; }
ensure_core() {
  core_present && return 0
  printf '%s' "$core_min" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || return 1
  get "$core_proto" "$core_releases/download/v$core_min/install.sh" "$tmp/core-install.sh" "${H0X_CORE_RELEASES_URL:-}" || return 1
  H0X_CORE_VERSION="$core_min" H0X_CORE_INSTALL_DIR="$bin" sh "$tmp/core-install.sh"
}
if [ -z "${H0X_ADE_RELEASES_URL:-}" ]; then
  # The checksum comes from the same place as the file: the signature is what
  # proves the release key made it. h0x-core (0.1.9 or newer) checks it, so
  # nothing from the download is unpacked before that.
  mkdir -p "$bin"
  ensure_core || fail "needs h0x-core $core_min to check the signature, and it did not install"
  if [ -x "$bin/h0x-core" ]; then core="$bin/h0x-core"; else core=$(command -v h0x-core 2>/dev/null) || core=; fi
  [ -n "$core" ] || fail "h0x-core is there but cannot run ($bin/h0x-core): fix or remove it, then run this again"
  get "$proto" "${url%%[?#]*}.minisig" "$archive.minisig" "" || fail "no signature for $asset: ${url%%[?#]*}.minisig"
  "$core" update --from "$archive" --check >/dev/null || fail "$asset is not signed by the HyperOx release key, or h0x-core could not check it (see above; h0x-core 0.1.9 or newer is needed)"
fi
tar -xzf "$archive" -C "$tmp"
mkdir -p "$bin"

if [ "$os" = linux ]; then
  data=${XDG_DATA_HOME:-$HOME/.local/share}
  home="$data/h0x-ade"
  mkdir -p "$data"
  rm -rf "$home.new"
  mv "$tmp/h0x-ade" "$home.new"
  rm -rf "$home"
  mv "$home.new" "$home"
  ln -sfn "$home/h0x-ade" "$bin/h0x-ade"
  mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
  cp "$home/h0x-ade.svg" "$data/icons/hicolor/scalable/apps/h0x-ade.svg"
  cat > "$data/applications/h0x-ade.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=HyperOx
Comment=Terminals and coding agents that keep running
Exec=$bin/h0x-ade
Icon=h0x-ade
Terminal=false
Categories=Development;
StartupWMClass=h0x-ade
EOF
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data/applications" 2>/dev/null || true
  [ -f "$data/icons/hicolor/icon-theme.cache" ] && gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
else
  apps="$HOME/Applications"
  app="$apps/HyperOx.app"
  mkdir -p "$apps"
  # Prepared in the temp folder, so Spotlight never indexes a half-made
  # bundle under a temporary name in Applications.
  # Never change a certificate-signed bundle: macOS permissions follow its
  # signing identity. Only unsigned/ad-hoc development builds need this PATH.
  signed=false
  if signature=$(/usr/bin/codesign -dvv "$tmp/HyperOx.app" 2>&1); then
    if printf '%s\n' "$signature" | grep -q '^Authority=Developer ID Application:'; then
      signed=true
    elif ! printf '%s\n' "$signature" | grep -qx 'Signature=adhoc'; then
      fail "cannot determine app signature: $signature"
    fi
  elif ! printf '%s\n' "$signature" | grep -q ': code object is not signed at all$'; then
    fail "cannot determine app signature: $signature"
  fi
  if [ "$signed" = true ]; then
    /usr/bin/codesign --verify --deep --strict "$tmp/HyperOx.app" || fail "invalid app signature"
  else
    /usr/libexec/PlistBuddy -c "Add :LSEnvironment:PATH string $bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin" "$tmp/HyperOx.app/Contents/Info.plist"
    /usr/bin/codesign --force --sign - "$tmp/HyperOx.app" >/dev/null 2>&1
  fi
  rm -rf "$app"
  mv "$tmp/HyperOx.app" "$app"
  ln -sfn "$app/Contents/MacOS/HyperOx" "$bin/h0x-ade"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app" 2>/dev/null || true
fi

ensure_core || say "h0x-core didn't install; HyperOx will offer it when it opens."

say "HyperOx is installed. Open it from your apps, or run: h0x-ade"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) say "Add $bin to your PATH to use h0x-ade and h0x-core from a terminal." ;;
esac
