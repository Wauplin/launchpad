#!/usr/bin/env bash
# Update the current user's app icon without stopping the desktop app or daemon.
set -euo pipefail
cd "$(dirname "$0")/.."
icon_dir="$HOME/.local/share/icons/hicolor"
for size in 16 24 32 48 64 96 128 256 512 1024; do
  install -Dm644 "packaging/icons/${size}x${size}.png" "$icon_dir/${size}x${size}/apps/launchpad.png"
done
install -Dm644 packaging/launchpad.svg "$icon_dir/scalable/apps/launchpad.svg"
if command -v gtk-update-icon-cache >/dev/null; then
  gtk-update-icon-cache -q -t -f "$icon_dir"
fi
echo "Launchpad icons updated."
