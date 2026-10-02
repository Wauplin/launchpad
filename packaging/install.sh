#!/usr/bin/env bash
# Builds Launchpad in release mode and installs it for the current user:
#   ~/.local/bin/{launchpad,launchpad-desktop}, the systemd user unit, the .desktop entry, the icons, and an
#   autostart entry that keeps the window loaded in the background (so the dock opens it instantly).
# Then restarts the daemon and the background window. Restarting stops running agents, so the script asks
# first when some are running.
#
# Usage: packaging/install.sh [--force]
set -euo pipefail

cd "$(dirname "$0")/.."
FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1
BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor"
UNITS="$HOME/.config/systemd/user"
AUTOSTART="$HOME/.config/autostart"

# Node version from web/.nvmrc, when nvm is installed.
if [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then
  set +u
  . "${NVM_DIR:-$HOME/.nvm}/nvm.sh"
  nvm use --silent "$(cat web/.nvmrc)" >/dev/null
  set -u
fi

# 1. Running agents are stopped by the restart.
running=0
if curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1 && [ -x "$BIN/launchpad" ]; then
  running=$("$BIN/launchpad" status 2>/dev/null | python3 -c 'import json,sys; s=json.load(sys.stdin)["sessions"]; print(s["running"] + s["pending"])' || echo 0)
fi
if [ "$running" -gt 0 ] && [ "$FORCE" -eq 0 ]; then
  echo "warning: $running agent session(s) are open in Launchpad. Restarting the daemon stops them (they can be resumed)."
  if [ -t 0 ]; then
    read -r -p "Restart anyway? [y/N] " answer
    [ "$answer" = "y" ] || [ "$answer" = "Y" ] || { echo "Cancelled."; exit 1; }
  else
    echo "Run again with --force to restart anyway."
    exit 1
  fi
fi

# 2. Build.
pnpm -C web install --frozen-lockfile --config.confirmModulesPurge=false
pnpm -C web build
cargo build --release -p launchpad -p launchpad-desktop

# 3. Install. Copy then rename, so running binaries are replaced safely.
mkdir -p "$BIN" "$APPS" "$UNITS" "$AUTOSTART"
# The running shell (if any) keeps the old binary: stop it, it is started again below.
if pgrep -x launchpad-deskt >/dev/null 2>&1; then
  "$BIN/launchpad-desktop" --quit >/dev/null 2>&1 || true
  sleep 0.5
fi
for b in launchpad launchpad-desktop; do
  cp "target/release/$b" "$BIN/.$b.new"
  chmod 755 "$BIN/.$b.new"
  mv -f "$BIN/.$b.new" "$BIN/$b"
done
install -Dm644 packaging/launchpad.service "$UNITS/launchpad.service"
sed "s|@BIN@|$BIN|g" packaging/launchpad.desktop > "$APPS/launchpad.desktop"
sed "s|@BIN@|$BIN|g" packaging/launchpad-autostart.desktop > "$AUTOSTART/launchpad.desktop"
install -Dm644 packaging/launchpad.svg "$ICONS/scalable/apps/launchpad.svg"
install -Dm644 desktop/src-tauri/icons/32x32.png "$ICONS/32x32/apps/launchpad.png"
install -Dm644 desktop/src-tauri/icons/128x128.png "$ICONS/128x128/apps/launchpad.png"
install -Dm644 desktop/src-tauri/icons/128x128@2x.png "$ICONS/256x256/apps/launchpad.png"
install -Dm644 desktop/src-tauri/icons/icon.png "$ICONS/512x512/apps/launchpad.png"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$ICONS" || true

# 4. Restart the daemon.
systemctl --user daemon-reload
systemctl --user enable --quiet launchpad.service
systemctl --user restart launchpad.service
for _ in $(seq 1 100); do
  curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1 && break
  sleep 0.1
done
if curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1; then
  if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    setsid "$BIN/launchpad-desktop" --hidden >/dev/null 2>&1 < /dev/null &
  fi
  echo "Launchpad installed. The daemon runs on http://127.0.0.1:7317 (logs: journalctl --user -u launchpad -f)."
  echo "Open it from the applications grid and pin it to the dock (right click > Add to Favorites)."
else
  echo "The daemon did not start. See: journalctl --user -u launchpad -n 50"
  exit 1
fi
