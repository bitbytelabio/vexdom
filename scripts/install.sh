#!/bin/sh
# Installs Vexdom Bridge (the `vexdom` binary and the unpacked Chrome extension beside it) from a
# GitHub release, and on macOS the desktop app `Vexdom.app` (into /Applications when writable,
# else ~/Applications):
#
#   curl -fsSL https://github.com/nguyenthdat/vexdom/releases/latest/download/install.sh | sh
#
# Each download is checked against its `.sha256`. macOS (arm64, x64) and Linux with glibc (x64,
# arm64). Environment:
#   VEXDOM_VERSION       a version (`0.9.2`) instead of the latest release
#   VEXDOM_INSTALL_DIR   where to install (default ~/.local/share/vexdom)
#   VEXDOM_ARCHIVE       a local release archive to install instead of downloading one (offline;
#                        installs no Vexdom.app)
#   VEXDOM_NO_APP=1      on macOS, skip Vexdom.app
set -eu

REPO=nguyenthdat/vexdom
DIR=${VEXDOM_INSTALL_DIR:-$HOME/.local/share/vexdom}

fail() {
  echo "vexdom install: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "needs $1"
}

target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os/$arch" in
    Darwin/arm64) echo aarch64-apple-darwin ;;
    Darwin/x86_64) echo x86_64-apple-darwin ;;
    Linux/x86_64 | Linux/amd64 | Linux/aarch64 | Linux/arm64)
      if ldd --version 2>&1 | grep -qi musl; then
        fail "unsupported platform: $os/$arch with musl (releases are built for glibc)"
      fi
      case "$arch" in
        x86_64 | amd64) echo x86_64-unknown-linux-gnu ;;
        *) echo aarch64-unknown-linux-gnu ;;
      esac
      ;;
    *) fail "unsupported platform: $os/$arch" ;;
  esac
}

latest_version() {
  url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
    fail "cannot reach GitHub to find the latest release"
  tag=${url##*/}
  case "$tag" in
    v*) echo "${tag#v}" ;;
    *) fail "no release found at https://github.com/$REPO/releases" ;;
  esac
}

verify() {
  # $1 = archive, $2 = its .sha256 (`<hash>  <name>`).
  expected=$(cut -d' ' -f1 <"$2")
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$1" | cut -d' ' -f1)
  else
    need shasum
    actual=$(shasum -a 256 "$1" | cut -d' ' -f1)
  fi
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $(basename "$1")"
}

# Replaces bin/ and extension/ in $DIR with the archive's, leaving anything else there alone.
install_archive() {
  archive=$1
  staging="$DIR.new"
  rm -rf "$staging"
  mkdir -p "$staging"
  tar xzf "$archive" -C "$staging"
  root=$(find "$staging" -mindepth 1 -maxdepth 1 -type d | head -n 1)
  [ -x "$root/bin/vexdom" ] || fail "$(basename "$archive") holds no bin/vexdom"
  [ -f "$root/extension/chrome-mv3/manifest.json" ] || fail "$(basename "$archive") holds no extension/chrome-mv3"
  mkdir -p "$DIR"
  rm -rf "${DIR:?}/bin" "${DIR:?}/extension"
  mv "$root/bin" "$DIR/bin"
  mv "$root/extension" "$DIR/extension"
  for file in LICENSE NOTICE; do
    [ -f "$root/$file" ] && mv "$root/$file" "$DIR/$file"
  done
  rm -rf "$staging"
}

# Puts the release zip's Vexdom.app in /Applications when writable, else ~/Applications. The app is
# unpacked beside its destination first, so a failure leaves an existing Vexdom.app as it was.
install_app() {
  zip=$1
  need ditto
  apps=/Applications
  [ -w "$apps" ] || apps="$HOME/Applications"
  mkdir -p "$apps" || fail "cannot create $apps"
  unpacked=$(mktemp -d "$apps/.Vexdom.XXXXXX") || fail "cannot write to $apps"
  if ! ditto -x -k "$zip" "$unpacked" || [ ! -d "$unpacked/Vexdom.app" ]; then
    rm -rf "$unpacked"
    fail "$(basename "$zip") holds no Vexdom.app"
  fi
  if [ -e "$apps/Vexdom.app" ]; then
    mv "$apps/Vexdom.app" "$unpacked/Vexdom.app.previous" || {
      rm -rf "$unpacked"
      fail "cannot replace $apps/Vexdom.app"
    }
  fi
  if ! mv "$unpacked/Vexdom.app" "$apps/Vexdom.app"; then
    [ -e "$unpacked/Vexdom.app.previous" ] && mv "$unpacked/Vexdom.app.previous" "$apps/Vexdom.app"
    rm -rf "$unpacked"
    fail "cannot move Vexdom.app into $apps"
  fi
  rm -rf "$unpacked"
  xattr -dr com.apple.quarantine "$apps/Vexdom.app" 2>/dev/null || true
  echo "Installed Vexdom.app in $apps"
}

main() {
  need tar
  if [ -n "${VEXDOM_ARCHIVE:-}" ]; then
    [ -f "$VEXDOM_ARCHIVE" ] || fail "no archive at $VEXDOM_ARCHIVE"
    install_archive "$VEXDOM_ARCHIVE"
  else
    need curl
    version=${VEXDOM_VERSION:-$(latest_version)}
    version=${version#v}
    triple=$(target)
    name="vexdom-$version-$triple.tar.gz"
    base="https://github.com/$REPO/releases/download/v$version"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    echo "Downloading $name"
    curl -fsSL -o "$work/$name" "$base/$name" || fail "cannot download $base/$name"
    curl -fsSL -o "$work/$name.sha256" "$base/$name.sha256" || fail "cannot download $base/$name.sha256"
    verify "$work/$name" "$work/$name.sha256"
    install_archive "$work/$name"
    if [ "$(uname -s)" = Darwin ] && [ "${VEXDOM_NO_APP:-}" != 1 ]; then
      app="Vexdom-$version-$triple.zip"
      echo "Downloading $app"
      curl -fsSL -o "$work/$app" "$base/$app" || fail "cannot download $base/$app"
      curl -fsSL -o "$work/$app.sha256" "$base/$app.sha256" || fail "cannot download $base/$app.sha256"
      verify "$work/$app" "$work/$app.sha256"
      install_app "$work/$app"
    fi
  fi

  echo "Installed $("$DIR/bin/vexdom" --version 2>/dev/null || echo vexdom) in $DIR"
  case ":$PATH:" in
    *":$DIR/bin:"*) ;;
    *) echo "Add to PATH: export PATH=\"$DIR/bin:\$PATH\"" ;;
  esac
  echo "Next: vexdom setup"
}

main "$@"
