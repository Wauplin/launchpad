#!/usr/bin/env bash
# Install or update a Launchpad Home Space on Hugging Face with the hf CLI.
#
#   curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/deploy/hf-home/deploy.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/deploy/hf-home/deploy.sh | bash -s -- --main
#
# Options: see --help. With --main, the Space uses CI's package of the newest main commit (main-build prerelease).
# Nothing is compiled on the Space builder.
set -euo pipefail

REPO="Wauplin/launchpad"
SPACE="" BUCKET="" RELEASE="" MODE="release" FORCE=0 WAIT=1

usage() {
  cat <<'USAGE'
Install or update a Launchpad Home Space on Hugging Face.

Options:
  --space <[namespace/]name>  Space to create or update (default: <your user>/launchpad-home)
  --bucket <namespace/name>   Backup bucket (default: <Space ID>-bucket)
  --release <tag>             Deploy this GitHub release (default: the latest release)
  --main                      Deploy the newest main commit that CI has packaged (main-build prerelease)
  --force                     Replace a Space Dockerfile that this script did not write
  --no-wait                   Do not wait for the build and the first start
USAGE
}
die() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }
step() { printf '\033[1m==> %s\033[0m\n' "$*"; }
ok() { printf '    %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --space) SPACE="${2:?--space needs a value}"; shift 2 ;;
    --bucket) BUCKET="${2:?--bucket needs a value}"; shift 2 ;;
    --release) RELEASE="${2:?--release needs a tag}"; shift 2 ;;
    --main) MODE="main"; shift ;;
    --force) FORCE=1; shift ;;
    --no-wait) WAIT=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
done
[ "$MODE" = main ] && [ -n "$RELEASE" ] && die "--release and --main cannot be used together"

# --- Checks: tools and Hugging Face sign-in ---------------------------------
step "Checking tools"
for tool in curl sed python3; do
  command -v "$tool" >/dev/null || die "'$tool' is required but not installed"
done
command -v hf >/dev/null || die "the Hugging Face CLI is not installed. Install it with: curl -LsSf https://hf.co/cli/install.sh | bash"
hf buckets --help >/dev/null 2>&1 && hf spaces volumes --help >/dev/null 2>&1 \
  || die "your hf CLI is too old (buckets and Space volumes are needed). Update it with: curl -LsSf https://hf.co/cli/install.sh | bash"
ok "hf $(hf --version 2>/dev/null | head -n 1)"

# json <dotted.path>: print one field of the JSON document on stdin ("" when missing).
json() { python3 -c 'import json,sys
d=json.load(sys.stdin)
for k in sys.argv[1].split("."):
    d = d[int(k)] if isinstance(d, list) else (d or {}).get(k)
print("" if d is None else (str(d).lower() if isinstance(d, bool) else d))' "$1"; }

step "Checking the Hugging Face sign-in"
WHOAMI="$(hf auth whoami --format json 2>/dev/null)" || die "you are not signed in to Hugging Face. Run: hf auth login"
USER_NAME="$(printf '%s' "$WHOAMI" | json user)"
[ -n "$USER_NAME" ] || die "you are not signed in to Hugging Face. Run: hf auth login"
TOKEN="$(hf auth token 2>/dev/null | tail -n 1)"
ok "signed in as $USER_NAME"

[ -n "$SPACE" ] || SPACE="launchpad-home"
case "$SPACE" in */*) ;; *) SPACE="$USER_NAME/$SPACE" ;; esac
[ -n "$BUCKET" ] || BUCKET="$SPACE-bucket"
case "$BUCKET" in */*) ;; *) die "--bucket must be <namespace>/<name>" ;; esac

# --- What to deploy -----------------------------------------------------------
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/upload"
if [ "$MODE" = release ]; then
  step "Checking the release"
  if [ -z "$RELEASE" ]; then
    latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" || die "cannot reach GitHub"
    RELEASE="${latest##*/tag/}"
    { [ "$RELEASE" != "$latest" ] && [ -n "$RELEASE" ]; } || die "no release found on https://github.com/$REPO/releases"
  fi
  base="https://github.com/$REPO/releases/download/$RELEASE"
  curl -fsSL "$base/launchpad-linux-x86_64.tar.gz.sha256" -o "$WORK/sha256" || die "release $RELEASE has no Linux package (launchpad-linux-x86_64.tar.gz.sha256)"
  grep -Eq '^[0-9a-f]{64} ' "$WORK/sha256" || die "release $RELEASE has an unreadable checksum file"
  curl -fsSLI -o /dev/null "$base/launchpad-linux-x86_64.tar.gz" || die "release $RELEASE has no launchpad-linux-x86_64.tar.gz"
  # The release's own Dockerfile.release when it has one, else the current one.
  curl -fsSL "https://raw.githubusercontent.com/$REPO/$RELEASE/deploy/hf-home/Dockerfile.release" -o "$WORK/upload/Dockerfile" 2>/dev/null \
    || curl -fsSL "https://raw.githubusercontent.com/$REPO/main/deploy/hf-home/Dockerfile.release" -o "$WORK/upload/Dockerfile" \
    || die "cannot download Dockerfile.release"
  grep -q '^ARG LAUNCHPAD_RELEASE=' "$WORK/upload/Dockerfile" || die "Dockerfile.release has no LAUNCHPAD_RELEASE line"
  sed -i.bak "s#^ARG LAUNCHPAD_RELEASE=.*#ARG LAUNCHPAD_RELEASE=$RELEASE#" "$WORK/upload/Dockerfile" && rm -f "$WORK/upload/Dockerfile.bak"
  WHAT="release $RELEASE"
  NOTE="Built from the GitHub release named by \`LAUNCHPAD_RELEASE\` in the Dockerfile. To update, set it to a newer release tag, or run the deploy script again."
  ok "$WHAT (package sha256 $(cut -c1-12 "$WORK/sha256")…)"
else
  step "Checking main"
  # CI publishes the package of each main commit to the main-build prerelease (.github/workflows/build.yml),
  # about 15 minutes after the push, and keeps the 20 newest: take the newest main commit that has one.
  curl -fsSL "https://api.github.com/repos/$REPO/commits?sha=main&per_page=30" -o "$WORK/commits.json" || die "cannot list the commits of main"
  curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/main-build" -o "$WORK/main-build.json" || die "cannot read the main-build prerelease"
  read -r COMMIT BEHIND < <(python3 -c 'import json,sys
assets = {a["name"] for a in json.load(open(sys.argv[2])).get("assets", [])}
for i, c in enumerate(json.load(open(sys.argv[1]))):
    if "launchpad-linux-x86_64-%s.tar.gz.sha256" % c["sha"] in assets:
        print(c["sha"], i); break' "$WORK/commits.json" "$WORK/main-build.json") || true
  [[ "${COMMIT:-}" =~ ^[0-9a-f]{40}$ ]] || die "no recent main commit has a CI package yet (see https://github.com/$REPO/releases/tag/main-build)"
  PACKAGE="launchpad-linux-x86_64-$COMMIT"
  curl -fsSL "https://github.com/$REPO/releases/download/main-build/$PACKAGE.tar.gz.sha256" -o "$WORK/sha256" || die "cannot download the checksum of $PACKAGE"
  grep -Eq '^[0-9a-f]{64} ' "$WORK/sha256" || die "the CI package of $COMMIT has an unreadable checksum file"
  curl -fsSL "https://raw.githubusercontent.com/$REPO/$COMMIT/deploy/hf-home/Dockerfile.release" -o "$WORK/upload/Dockerfile" \
    || die "cannot download Dockerfile.release of $COMMIT"
  grep -q '^ARG LAUNCHPAD_PACKAGE=' "$WORK/upload/Dockerfile" || die "Dockerfile.release of $COMMIT has no LAUNCHPAD_PACKAGE line"
  sed -i.bak -e "s#^ARG LAUNCHPAD_RELEASE=.*#ARG LAUNCHPAD_RELEASE=main-build#" -e "s#^ARG LAUNCHPAD_PACKAGE=.*#ARG LAUNCHPAD_PACKAGE=$PACKAGE#" "$WORK/upload/Dockerfile" \
    && rm -f "$WORK/upload/Dockerfile.bak"
  WHAT="main $COMMIT"
  NOTE="Built from CI's package of main commit \`$COMMIT\`. Run the deploy script again with \`--main\` to update."
  ok "$WHAT (package sha256 $(cut -c1-12 "$WORK/sha256")…)"
  [ "$BEHIND" = 0 ] || ok "note: the $BEHIND newest main commit(s) have no CI package yet (CI publishes one about 15 minutes after each push)"
fi
cat > "$WORK/upload/README.md" <<EOF
---
title: Launchpad Home
emoji: 🏠
colorFrom: blue
colorTo: green
sdk: docker
app_port: 7860
header: mini
tags:
- launchpad-home
---

[Launchpad](https://github.com/$REPO) Home. $NOTE
EOF

# --- Bucket -------------------------------------------------------------------
step "Backup bucket $BUCKET"
if info="$(hf buckets info "$BUCKET" --format json 2>/dev/null)" && [ -n "$(printf '%s' "$info" | json id 2>/dev/null)" ]; then
  [ "$(printf '%s' "$info" | json private)" = true ] || die "bucket $BUCKET is public. Home stores its backups there: make it private (hf buckets settings $BUCKET --private) and run again"
  ok "exists, private"
else
  hf buckets create "$BUCKET" --private >/dev/null || die "cannot create bucket $BUCKET"
  ok "created (private)"
fi

# --- Space --------------------------------------------------------------------
step "Space $SPACE"
VOLUME="hf://buckets/$BUCKET:/backups"
if info="$(hf spaces info "$SPACE" --format json 2>/dev/null)" && [ -n "$(printf '%s' "$info" | json id 2>/dev/null)" ]; then
  [ "$(printf '%s' "$info" | json private)" = true ] || die "Space $SPACE is public. Make it private in its settings and run again"
  [ "$(printf '%s' "$info" | json card_data.sdk)" = docker ] || die "Space $SPACE is not a Docker Space"
  if hf download "$SPACE" Dockerfile --repo-type space --local-dir "$WORK/current" >/dev/null 2>&1; then
    grep -Eq '^ARG LAUNCHPAD_(RELEASE|REVISION)' "$WORK/current/Dockerfile" || [ "$FORCE" = 1 ] \
      || die "the Dockerfile of $SPACE was not written by this script (edited by hand?). Run again with --force to replace it"
  fi
  mounted="$(hf spaces volumes ls "$SPACE" --format json 2>/dev/null | python3 -c 'import json,sys
try: vols = json.load(sys.stdin) or []
except ValueError: vols = []
print(" ".join("%s:%s:%s:%s" % (v.get("type"), v.get("source"), v.get("mount_path"), v.get("read_only")) for v in vols))')"
  if [ -z "$mounted" ]; then
    hf spaces volumes set "$SPACE" -v "$VOLUME" >/dev/null || die "cannot mount $BUCKET at /backups"
    ok "exists, private; mounted $BUCKET at /backups"
  elif [ "$mounted" = "bucket:$BUCKET:/backups:False" ]; then
    ok "exists, private, $BUCKET mounted read-write at /backups"
  else
    die "Space $SPACE has other volumes ($mounted). Home needs exactly $BUCKET mounted read-write at /backups"
  fi
else
  hf repos create "$SPACE" --type space --sdk docker --private --flavor cpu-basic -v "$VOLUME" >/dev/null || die "cannot create Space $SPACE"
  ok "created (private, cpu-basic, $BUCKET at /backups)"
fi
if [ "$BUCKET" != "$SPACE-bucket" ]; then
  hf spaces variables add "$SPACE" -e "LP_HOME_BUCKET=$BUCKET" >/dev/null 2>&1 \
    || ok "note: set the Space variable LP_HOME_BUCKET=$BUCKET in the Space settings"
fi

# --- Upload and wait ----------------------------------------------------------
step "Uploading $WHAT"
hf upload "$SPACE" "$WORK/upload" . --repo-type space --delete '*' --commit-message "Deploy Launchpad Home: $WHAT" >"$WORK/upload.log" 2>&1 \
  || { cat "$WORK/upload.log" >&2; die "upload failed"; }
info="$(hf spaces info "$SPACE" --format json)"
TARGET="$(printf '%s' "$info" | json sha)"
HOST="$(printf '%s' "$info" | json host)"
ok "Space commit $TARGET"
if [ "$WAIT" = 0 ]; then
  step "Building in the background: https://huggingface.co/spaces/$SPACE"
  exit 0
fi

step "Waiting for the build and the first start (a few minutes)"
# elapsed <seconds>: "42s" or "3m 05s".
elapsed() { if [ "$1" -ge 60 ]; then printf '%dm %02ds' $(( $1 / 60 )) $(( $1 % 60 )); else printf '%ds' "$1"; fi; }
# On a terminal, one line per stage counts up in place; elsewhere, each stage prints once when it ends.
TTY=0; [ -t 1 ] && TTY=1
start=$(date +%s); deadline=$(( start + 3600 )); last="" since=$start polled=0 stage=""
while :; do
  now=$(date +%s)
  if [ $(( now - polled )) -ge 10 ]; then
    polled=$now
    info="$(hf spaces info "$SPACE" --format json 2>/dev/null || echo '{}')"
    stage="$(printf '%s' "$info" | json runtime.stage 2>/dev/null || true)"
    sha="$(printf '%s' "$info" | json runtime.raw.sha 2>/dev/null || true)"
    [ "$sha" = "$TARGET" ] || stage="waiting for the new commit"
    if [ "$TTY" = 1 ]; then printf '\r\033[K'; fi
    if [ "$stage" != "$last" ]; then
      [ -z "$last" ] || ok "$last in $(elapsed $(( now - since )))"
      last="$stage" since=$now
    fi
    case "$stage" in
      RUNNING) ok "RUNNING after $(elapsed $(( now - start )))"; break ;;
      BUILD_ERROR|RUNTIME_ERROR|CONFIG_ERROR|NO_APP_FILE|PAUSED|STOPPED|DELETING)
        die "the Space stopped in $stage after $(elapsed $(( now - start ))). Read the logs with: hf spaces logs $SPACE --build   (or without --build for the run logs)" ;;
    esac
    [ "$now" -lt "$deadline" ] || die "the Space is not running after one hour (stage $stage). See https://huggingface.co/spaces/$SPACE"
  fi
  if [ "$TTY" = 1 ]; then printf '\r\033[K    %s for %s (total %s)' "$last" "$(elapsed $(( now - since )))" "$(elapsed $(( now - start )))"; sleep 1; else sleep 10; fi
done

step "Checking Home"
code=000
for _ in $(seq 1 12); do
  code="$(curl -s -o "$WORK/status" -w '%{http_code}' -H "Authorization: Bearer $TOKEN" "$HOST/_home/status" || true)"
  [ "$code" = 200 ] && break
  sleep 10
done
[ "$code" = 200 ] || die "Home does not answer at $HOST/_home/status (HTTP $code). Read the logs with: hf spaces logs $SPACE"
if [ "$(json complete < "$WORK/status" 2>/dev/null)" = true ]; then
  ok "Home is up and set up"
else
  ok "Home is up; open it to finish the setup steps"
fi

printf '\n\033[32mLaunchpad Home is ready\033[0m after %s: %s\n' "$(elapsed $(( $(date +%s) - start )))" "$HOST"
printf 'Space: https://huggingface.co/spaces/%s\n' "$SPACE"
