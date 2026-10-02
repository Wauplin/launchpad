#!/usr/bin/env bash
# Runs as a transient systemd user service so it survives the daemon restart.
set -Eeuo pipefail

cd "$(dirname "$0")/.."
STATUS="$HOME/.local/share/launchpad/update-status"
LOG="$HOME/.local/share/launchpad/update.log"
mkdir -p "$(dirname "$STATUS")"
exec >"$LOG" 2>&1

stage="Preparing"
status() {
  printf '%s\t%s\t%s\n' "$1" "$stage" "$2" >"$STATUS.tmp"
  mv -f "$STATUS.tmp" "$STATUS"
}
trap 'status failed "Check $LOG (failed at line $LINENO)."' ERR

# The daemon passes its resolved login-shell PATH, and nvm selects the pinned Node version.
if [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then
  set +u
  . "${NVM_DIR:-$HOME/.nvm}/nvm.sh"
  nvm use --silent "$(cat web/.nvmrc)" >/dev/null
  set -u
fi

stage="Installing UI dependencies"
status running "$stage"
(cd web && pnpm install --frozen-lockfile --config.confirmModulesPurge=false)

stage="Checking UI types"
status running "$stage"
(cd web && pnpm typecheck)

stage="Running Rust tests"
status running "$stage"
cargo test

stage="Building UI"
status running "$stage"
(cd web && pnpm build)

stage="Building release binaries"
status running "$stage"
cargo build --release -p launchpad -p launchpad-desktop

stage="Installing and relaunching"
status running "$stage"
packaging/install.sh --prebuilt --show

stage="Complete"
status complete "Launchpad is up to date."
