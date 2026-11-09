#!/usr/bin/env bash
# Builds Launchpad in release mode and installs it for the current user (Linux only): ~/.local/bin/launchpad
# and the systemd user unit, which runs a Home. Then restarts the daemon. Restarting stops running agents, so
# the script asks first when some are running. Open the daemon's address in a browser and install it as an app.
#
# --headless installs a server: it also enables lingering, so the systemd user unit starts at boot and keeps
# running after logout. See docs/remote-access.md.
#
# Usage: packaging/install.sh [--force] [--prebuilt] [--headless]
set -euo pipefail
if [ "$(uname -s)" != Linux ]; then
  echo "packaging/install.sh supports Linux only." >&2
  exit 2
fi

cd "$(dirname "$0")/.."
FORCE=0
PREBUILT=0
HEADLESS=0
for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    --prebuilt) PREBUILT=1 ;;
    --headless) HEADLESS=1 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done
BIN="$HOME/.local/bin"
UNITS="$HOME/.config/systemd/user"

# Source builds select Node; downloaded prebuilt packages need no Node or checkout.
if [ "$PREBUILT" -eq 0 ] && [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then
  set +u
  . "${NVM_DIR:-$HOME/.nvm}/nvm.sh"
  nvm use --silent "$(cat web/.nvmrc)" >/dev/null
  set -u
fi

# 1. Running agents are stopped by the restart.
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
  echo "warning: $running agent session(s) are running in Launchpad. Restarting the daemon interrupts them (they can be resumed)."
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
  cargo build --release -p launchpad
fi

# Sessions may have started while the build ran. Never stop them without an explicit --force.
check_running
if [ "$running" -gt 0 ] && [ "$FORCE" -eq 0 ]; then
  echo "Update paused: $running agent session(s) started during the build. Run the installer with --force when they can be interrupted." >&2
  exit 1
fi

# 3. Install. Copy then rename, so running binaries are replaced safely.
mkdir -p "$BIN" "$UNITS"
cp -r skills "$BIN/"
cp target/release/launchpad "$BIN/.launchpad.new"
chmod 755 "$BIN/.launchpad.new"
mv -f "$BIN/.launchpad.new" "$BIN/launchpad"
install -Dm644 packaging/launchpad.service "$UNITS/launchpad.service"
# The desktop shell is gone: remove what earlier installs added for it (the PWA replaces it).
if pgrep -x launchpad-deskt >/dev/null 2>&1; then pkill -x launchpad-deskt || true; fi
rm -f "$BIN/launchpad-desktop"
for entry in "$HOME/.local/share/applications/launchpad.desktop" "$HOME/.config/autostart/launchpad.desktop"; do
  if grep -q '/launchpad-desktop' "$entry" 2>/dev/null; then rm -f "$entry"; fi
done

# A server has no graphical login: lingering starts the user manager (and the unit) at boot and keeps it
# running after the last SSH session ends.
if [ "$HEADLESS" -eq 1 ]; then
  USER="${USER:-$(id -un)}"
  if [ "$(loginctl show-user "$USER" --property=Linger --value 2>/dev/null || true)" != yes ]; then
    if ! loginctl enable-linger "$USER" 2>/dev/null; then
      echo "warning: could not enable lingering for $USER, so Launchpad stops at logout and does not start at boot." >&2
      echo "Ask an administrator to run: sudo loginctl enable-linger $USER" >&2
    fi
  fi
fi

# 4. Restart the daemon.
systemctl --user daemon-reload
systemctl --user enable --quiet launchpad.service
systemctl --user restart launchpad.service
for _ in $(seq 1 100); do
  curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1 && break
  sleep 0.1
done
if curl -sf http://127.0.0.1:7317/api/health >/dev/null 2>&1; then
  if [ "$HEADLESS" -eq 1 ]; then
    echo "Launchpad installed (headless). The daemon runs on http://127.0.0.1:7317 and starts at boot (logs: journalctl --user -u launchpad -f)."
    echo "Expose it through Tailscale Serve or an identity-aware proxy; see docs/remote-access.md."
  else
    echo "Launchpad installed. The daemon runs on http://127.0.0.1:7317 (logs: journalctl --user -u launchpad -f)."
    echo "Sign a browser in with: $BIN/launchpad url (then install it as an app from the browser menu)."
  fi
else
  echo "The daemon did not start. See: journalctl --user -u launchpad -n 50"
  exit 1
fi
