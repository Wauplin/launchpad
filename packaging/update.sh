#!/usr/bin/env bash
# Runs as a transient systemd user service so it survives the daemon restart.
set -Eeuo pipefail

cd "$(dirname "$0")/.."
STATUS="$HOME/.local/share/launchpad/update-status"
LOG="$HOME/.local/share/launchpad/update.log"
mkdir -p "$(dirname "$STATUS")"
# Only one updater may write progress or build at a time.
exec 9>"$HOME/.local/share/launchpad/update.lock"
flock -n 9 || exit 0
exec >"$LOG" 2>&1
stage="Preparing"
status() {
  printf '%s\t%s\t%s\n' "$1" "$stage" "$2" >"$STATUS.tmp"
  mv -f "$STATUS.tmp" "$STATUS"
}
reserved=0
cleanup() {
  if [ "$reserved" -eq 1 ]; then
    LAUNCHPAD_PROFILE=prod target/release/launchpad update-install-lock --release || true
  fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
trap 'status failed "Check $LOG (failed at line $LINENO)."' ERR

# Never build unsaved work or a feature branch. The update source is local main.
stage="Waiting for committed main"
status running "$stage"
while [ "$(git branch --show-current)" != main ] || [ -n "$(git status --porcelain)" ]; do sleep 5; done
revision=$(git rev-parse HEAD)

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

# Refuse a mixed build if the checkout changed while compiling.
check_revision() {
  if [ "$(git rev-parse HEAD)" != "$revision" ] || [ -n "$(git status --porcelain)" ]; then
    status failed "The checkout changed during the update. Retry after main is committed."
    exit 1
  fi
}
check_revision

stage="Waiting for agents to stop"
status running "$stage"
while true; do
  if LAUNCHPAD_PROFILE=prod target/release/launchpad update-install-lock; then
    reserved=1
    break
  else
    result=$?
    # Only a busy daemon is retried. Authentication or daemon errors fail safely.
    [ "$result" -eq 3 ] || { status failed "Could not reserve the idle daemon for installation. Check $LOG."; exit "$result"; }
  fi
  sleep 2
done

check_revision

stage="Installing and relaunching"
status running "$stage"
packaging/install.sh --prebuilt --show

reserved=0

stage="Complete"
status complete "Launchpad is up to date."
