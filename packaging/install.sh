#!/usr/bin/env bash
# Builds Launchpad in release mode and installs it for the current user:
#   ~/.local/bin/{launchpad,launchpad-desktop}, the systemd user unit, the .desktop entry, the icons, and an
#   autostart entry that keeps the window loaded in the background (so the dock opens it instantly).
# Then restarts the daemon and the background window. Older sessions without workers can be interrupted, so the script asks
# first when some are running.
#
# Usage: packaging/install.sh [--force] [--prebuilt] [--show]
set -euo pipefail

cd "$(dirname "$0")/.."
FORCE=0
PREBUILT=0
SHOW=0
for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    --prebuilt) PREBUILT=1 ;;
    --show) SHOW=1 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done
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

# 1. Keep an idle installation policy even though session workers survive the restart.
check_running() {
  running=0
  if curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1 && [ -x "$BIN/launchpad" ]; then
    if ! running=$("$BIN/launchpad" status 2>/dev/null | python3 -c 'import json,sys; s=json.load(sys.stdin)["sessions"]; print(s["running"])'); then
      echo "Cannot read running sessions; leaving the daemon running." >&2
      exit 1
    fi
  fi
}
check_running
if [ "$running" -gt 0 ] && [ "$FORCE" -eq 0 ]; then
  echo "warning: $running agent session(s) are running in Launchpad. Session workers survive the restart; older sessions without workers may be interrupted."
  if [ -t 0 ]; then
    read -r -p "Restart anyway? [y/N] " answer
    [ "$answer" = "y" ] || [ "$answer" = "Y" ] || { echo "Cancelled."; exit 1; }
  else
    echo "Run again with --force to restart anyway."
    exit 1
  fi
fi

# 2. Build.
# From web/: Corepack picks the pnpm version of the nearest package.json, not the one of `-C web`.
if [ "$PREBUILT" -eq 0 ]; then
  (cd web && pnpm install --frozen-lockfile --config.confirmModulesPurge=false && pnpm build)
  cargo build --release -p launchpad -p launchpad-desktop
fi

# Sessions may have started while the build ran. Never stop them without an explicit --force.
check_running
if [ "$running" -gt 0 ] && [ "$FORCE" -eq 0 ]; then
  echo "Update paused: $running agent session(s) started during the build. Run the installer with --force when installation is safe." >&2
  exit 1
fi

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
    if [ "$SHOW" -eq 1 ]; then
      # The updater itself runs in a transient unit. A separate unit keeps the new window alive.
      window_env=()
      for key in DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS; do
        if [ -n "${!key:-}" ]; then window_env+=("--setenv=$key=${!key}"); fi
      done
      systemd-run --user --collect "--unit=launchpad-desktop-update-$(date +%s)" "${window_env[@]}" "$BIN/launchpad-desktop"
    else
      setsid "$BIN/launchpad-desktop" --hidden >/dev/null 2>&1 < /dev/null &
    fi
  fi
  echo "Launchpad installed. The daemon runs on http://127.0.0.1:7317 (logs: journalctl --user -u launchpad -f)."
  echo "Open it from the applications grid and pin it to the dock (right click > Add to Favorites)."
else
  echo "The daemon did not start. See: journalctl --user -u launchpad -n 50"
  exit 1
fi
