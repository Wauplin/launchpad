#!/usr/bin/env bash
# Runs as a transient systemd user service so it survives the daemon restart.
set -Eeuo pipefail

cd "$(dirname "$0")"
DATA="${LAUNCHPAD_UPDATE_DATA_DIR:-$HOME/.local/share/launchpad}"
STATUS="$DATA/update-status"
LOG="$DATA/update.log"
mkdir -p "$(dirname "$STATUS")"
# Only one updater may write progress or build at a time.
exec 9>"$DATA/update.lock"
if command -v flock >/dev/null; then
  flock -n 9 || exit 0
else
  mkdir "$DATA/update.lockdir" 2>/dev/null || exit 0
fi
exec >"$LOG" 2>&1
stage="Preparing"
status() {
  printf '%s\t%s\t%s\n' "$1" "$stage" "$2" >"$STATUS.tmp"
  mv -f "$STATUS.tmp" "$STATUS"
}
reserved=0
cleanup() {
  rmdir "$DATA/update.lockdir" 2>/dev/null || true
  if [ "$reserved" -eq 1 ]; then
    LAUNCHPAD_PROFILE=prod target/release/launchpad update-install-lock --release || true
  fi
  if [ -n "${work:-}" ]; then rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
trap 'status failed "Check $LOG (failed at line $LINENO)."' ERR

# Resolve GitHub main and require a successful trusted CI run for that exact commit.
# There is deliberately no fallback to a local build or an older successful commit.
stage="Downloading GitHub build"
status running "$stage"
repo=Wauplin/launchpad
revision=$(gh api "repos/$repo/commits/main" --jq .sha)
run=$(gh api "repos/$repo/actions/workflows/ci.yml/runs?branch=main&head_sha=$revision&status=success&per_page=100" --jq '.workflow_runs | map(select(.event == "push" or .event == "workflow_dispatch")) | first | .id // empty')
[ -n "$run" ] || { status failed "GitHub CI has not passed for latest main ($revision). Retry when it finishes."; exit 1; }
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x86_64 ;;
  Darwin-arm64) platform=macos-aarch64 ;;
  Darwin-x86_64) platform=macos-x86_64 ;;
  *) status failed "Unsupported platform."; exit 1 ;;
esac
work=$(mktemp -d)
gh run download "$run" --repo "$repo" --name "launchpad-$platform" --dir "$work"
(cd "$work" && if command -v sha256sum >/dev/null; then sha256sum -c "launchpad-$platform.tar.gz.sha256"; else shasum -a 256 -c "launchpad-$platform.tar.gz.sha256"; fi)
mkdir "$work/release"
tar -xzf "$work/launchpad-$platform.tar.gz" -C "$work/release"
cd "$work/release"
[ "$(cat COMMIT)" = "$revision" ] || { status failed "GitHub artifact commit mismatch."; exit 1; }

if [ "${1:-}" = --download-only ]; then
  stage="Download verified"
  status complete "Verified GitHub build $revision for $platform; installation skipped."
  exit 0
fi

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

stage="Installing and relaunching"
status running "$stage"
packaging/install.sh --prebuilt --show

reserved=0

stage="Complete"
status complete "Launchpad is up to date."
