#!/usr/bin/env bash
# Builds launchpad-desktop_<version>_amd64.deb (and its .sha256) from a release binary of desktop/.
# CI runs it in an ubuntu:26.04 container, where dpkg-shlibdeps computes Depends from the installed libraries.
#
#   desktop/packaging/build-deb.sh <version> [--binary <path>] [--out <dir>] [--depends "<Depends line>"]
#
# <version>: YYYY.MM.DD.N for a release (the tag without its v), or YYYY.MM.DD.0~main.<HHMMSS>.g<commit> for a main
# build (sorts below that day's releases and names the commit). --depends replaces dpkg-shlibdeps, for a build made
# against a local sysroot whose libraries dpkg does not know (never in CI).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
version=${1:?usage: build-deb.sh <version> [--binary <path>] [--out <dir>] [--depends <line>]}
shift
binary="${CARGO_TARGET_DIR:-$root/desktop/target}/release/launchpad-desktop"
out="$root/desktop/dist"
depends=""
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) binary=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --depends) depends=$2; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
if ! printf '%s' "$version" | grep -Eq '^[0-9]+(\.[0-9]+)*(~[a-z]+(\.[0-9a-z]+)*)?$'; then
  echo "not a package version: $version" >&2
  exit 2
fi
[ -x "$binary" ] || { echo "no binary at $binary (cargo build --release --locked)" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
pkg="$stage/pkg"
install -Dm755 "$binary" "$pkg/usr/bin/launchpad-desktop"
install -Dm644 "$here/dev.launchpad.Desktop.desktop" "$pkg/usr/share/applications/dev.launchpad.Desktop.desktop"
# Icons: the sizes packaging/generate-icons.py already made from packaging/icons/source.png.
for size in 16 24 32 48 64 96 128 256 512; do
  install -Dm644 "$root/packaging/icons/${size}x${size}.png" "$pkg/usr/share/icons/hicolor/${size}x${size}/apps/dev.launchpad.Desktop.png"
done

if [ -z "$depends" ]; then
  # dpkg-shlibdeps reads debian/control of a source tree: a minimal one in the staging folder.
  mkdir -p "$stage/src/debian"
  printf 'Source: launchpad-desktop\n\nPackage: launchpad-desktop\nArchitecture: amd64\n' > "$stage/src/debian/control"
  depends=$(cd "$stage/src" && dpkg-shlibdeps -O "$pkg/usr/bin/launchpad-desktop" | sed -n 's/^shlibs:Depends=//p')
  [ -n "$depends" ] || { echo "dpkg-shlibdeps found no dependencies" >&2; exit 1; }
fi

size=$(du -sk "$pkg" | cut -f1)
mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: launchpad-desktop
Version: $version
Architecture: amd64
Maintainer: Launchpad <noreply@github.com>
Installed-Size: $size
Depends: $depends
Recommends: gnome-keyring, xdg-desktop-portal-gnome, packagekit
Section: utils
Priority: optional
Homepage: https://github.com/Wauplin/launchpad
Description: Launchpad companion for the GNOME desktop
 A global spotlight to ask Launchpad's bots, go to any agent, chat or folio
 and quick launch agents, a top-bar menu with what needs you, and desktop
 notifications. The full Launchpad UI stays in the browser.
CONTROL

mkdir -p "$out"
deb="$out/launchpad-desktop_${version}_amd64.deb"
dpkg-deb --root-owner-group --build "$pkg" "$deb" >/dev/null
(cd "$out" && sha256sum "$(basename "$deb")" > "$(basename "$deb").sha256")
echo "$deb"
