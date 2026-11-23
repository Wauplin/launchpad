#!/usr/bin/env bash
# Installs the Launchpad desktop companion (launchpad-desktop) on Ubuntu 26.04 amd64:
#
#   curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/desktop/packaging/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/desktop/packaging/install.sh | bash -s -- --release
#
# It resolves the package like the companion's own updater (desktop/src/update.rs): the newest
# launchpad-desktop_<version>_amd64.deb of the `main-build` prerelease (default) or of the latest release (--release)
# that has its .sha256, downloaded only from Wauplin/launchpad's release URLs into a private folder. Before apt runs,
# the file's SHA-256 must equal both GitHub's digest of the asset and the published .sha256, and `dpkg-deb -f` must
# name package launchpad-desktop, architecture amd64 and the resolved version. Then `sudo apt-get install` installs it
# (apt also installs GTK 4 and the other libraries it needs). Nothing else on the machine changes.
#
# Usage: install.sh [--main | --release] [--dry-run] [--help]
#   --main      the newest build of the main branch (the main-build prerelease; the default)
#   --release   the newest release
#   --dry-run   resolve, download and verify, then print the apt command instead of running it
#
# Environment:
#   LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1    skip the Ubuntu 26.04 check (never the amd64 check; unsupported)
#   LAUNCHPAD_DESKTOP_OS_RELEASE=<file>  read this file instead of /etc/os-release (tests)
#   LAUNCHPAD_DESKTOP_INSTALL_API=<url>  the GitHub API base, for a local fake release server (tests); the asset URLs
#                                        must then start with that base, and plain http is allowed
#
# The whole script is one function called on the last line, so a cut download runs nothing. With `curl … | bash`,
# stdin is the script itself: nothing here reads stdin (commands get </dev/null; sudo asks on the terminal).
set -euo pipefail

REPO="Wauplin/launchpad"
PACKAGE="launchpad-desktop"
ARCH="amd64"
RELEASES="https://github.com/$REPO/releases"

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$*"; }
note() { printf '  \033[33m!\033[0m %s\n' "$*"; }
fail() { printf '\n\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
Installs the Launchpad desktop companion on Ubuntu 26.04 amd64.

Usage: install.sh [--main | --release] [--dry-run] [--help]
  --main      the newest build of the main branch (the main-build prerelease; the default)
  --release   the newest release
  --dry-run   resolve, download and verify, then print the apt command instead of running it

Environment: LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1 (skip the Ubuntu 26.04 check, unsupported),
LAUNCHPAD_DESKTOP_OS_RELEASE=<file> and LAUNCHPAD_DESKTOP_INSTALL_API=<url> (tests).
USAGE
}

# A value of an os-release file (KEY=value, KEY="value" or KEY='value'), without sourcing it.
os_field() {
  sed -n "s/^$1=//p" "$2" | head -n 1 | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'$/\1/"
}

# The assets of a release JSON as lines `name<TAB>browser_download_url<TAB>digest` (digest empty when GitHub gives
# none). Assets with a control character in a field are left out. Exit 3 for a body that is not a release.
list_assets() {
  python3 -I -c '
import json, sys
try:
    with open(sys.argv[1], encoding="utf-8") as f:
        release = json.load(f)
except (OSError, ValueError):
    sys.exit(3)
assets = release.get("assets") if isinstance(release, dict) else None
if not isinstance(assets, list):
    sys.exit(3)
for a in assets:
    if not isinstance(a, dict):
        continue
    fields = [a.get("name"), a.get("browser_download_url"), a.get("digest") or ""]
    if not all(isinstance(v, str) for v in fields):
        continue
    if any(ord(c) < 32 or ord(c) == 127 for v in fields for c in v):
        continue
    print("\t".join(fields))
' "$1" </dev/null
}

# 64 lowercase hex digits, or nothing.
hex64() {
  local s
  s=$(printf '%s' "$1" | tr 'A-F' 'a-f')
  [[ $s =~ ^[0-9a-f]{64}$ ]] && printf '%s' "$s"
}

main() {
  local channel=main dry_run=0 arg
  for arg in "$@"; do
    case "$arg" in
      --main) channel=main ;;
      --release) channel=release ;;
      --dry-run) dry_run=1 ;;
      -h | --help) usage; exit 0 ;;
      *) echo "Unknown option: $arg (use --main, --release, --dry-run or --help)" >&2; exit 2 ;;
    esac
  done

  bold "1/4 · Checking this machine"
  [ "$(uname -s)" = Linux ] || fail "The desktop companion needs Ubuntu 26.04 on amd64. This machine runs $(uname -s) $(uname -m)."
  local arch
  if command -v dpkg >/dev/null; then arch=$(dpkg --print-architecture </dev/null); else arch=$(uname -m); fi
  [ "$arch" = amd64 ] || [ "$arch" = x86_64 ] || fail "The desktop companion is built for amd64 only. This machine is $arch."
  ok "amd64"
  local os_release="${LAUNCHPAD_DESKTOP_OS_RELEASE:-/etc/os-release}" os_id="" os_version="" os_name=""
  if [ -r "$os_release" ]; then
    os_id=$(os_field ID "$os_release")
    os_version=$(os_field VERSION_ID "$os_release")
    os_name=$(os_field PRETTY_NAME "$os_release")
  fi
  if [ "$os_id" = ubuntu ] && [ "$os_version" = 26.04 ]; then
    ok "Ubuntu 26.04"
  elif [ "${LAUNCHPAD_DESKTOP_SKIP_OS_CHECK:-}" = 1 ]; then
    note "${os_name:-This system} is not Ubuntu 26.04: going on because LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1 (unsupported)"
  else
    fail "The desktop companion supports Ubuntu 26.04 only. This machine runs ${os_name:-an unknown system (no $os_release)}.
Set LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1 to try anyway (unsupported)."
  fi
  local tool
  for tool in curl sha256sum python3 dpkg dpkg-deb dpkg-query apt-get; do
    command -v "$tool" >/dev/null || fail "This installer needs $tool, which is missing. Install it first, for example: sudo apt install $tool"
  done
  local sudo=()
  if [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null; then
      sudo=(sudo)
    elif [ "$dry_run" -eq 0 ]; then
      fail "Installing a package needs sudo, which is missing. Run this as root, or install sudo first."
    fi
  fi
  ok "curl, sha256sum, python3, dpkg-deb, apt-get${sudo[*]:+, sudo}"

  # The API base and where downloads may come from. A fake release server (tests) may use http, and only its own
  # URLs are accepted then.
  local api="https://api.github.com" allowed="$RELEASES/download/" protocols="=https"
  if [ -n "${LAUNCHPAD_DESKTOP_INSTALL_API:-}" ]; then
    api="${LAUNCHPAD_DESKTOP_INSTALL_API%/}"
    allowed="$api/"
    protocols="=http,https"
    note "Using the release server $api (LAUNCHPAD_DESKTOP_INSTALL_API), not GitHub"
  fi
  local curl_opts=(--silent --show-error --location --proto "$protocols" --proto-redir "$protocols" --connect-timeout 15 --retry 2)

  local dir
  dir=$(mktemp -d "${TMPDIR:-/tmp}/launchpad-desktop-install.XXXXXXXX")
  chmod 700 "$dir"
  # shellcheck disable=SC2064 # $dir is fixed now.
  trap "rm -rf '$dir'" EXIT

  local path label page
  if [ "$channel" = main ]; then
    path="releases/tags/main-build" label="the main-build prerelease" page="$RELEASES/tag/main-build"
  else
    path="releases/latest" label="the latest release" page="$RELEASES/latest"
  fi
  bold "2/4 · Finding the package in $label of $REPO"
  local status
  status=$(curl "${curl_opts[@]}" --max-filesize 20M -H "Accept: application/vnd.github+json" -o "$dir/release.json" -w '%{http_code}' "$api/repos/$REPO/$path" </dev/null) \
    || fail "Cannot reach $api to read $label. Check the network and try again."
  case "$status" in
    200) ;;
    404) fail "Found no ${label#the } of $REPO. See $RELEASES" ;;
    403 | 429) fail "GitHub refused the request (HTTP $status), probably its rate limit for anonymous requests. Try again later, or download the package by hand from $page" ;;
    *) fail "GitHub answered HTTP $status for $label. Try again later, or see $page" ;;
  esac
  local assets
  if ! assets=$(list_assets "$dir/release.json"); then
    fail "GitHub's answer for $label is not a release. Try again later, or see $page"
  fi

  # Like update::select: launchpad-desktop_<version>_amd64.deb with a strict version (letters, digits, . ~ +, starting
  # with a digit, as Debian versions do), only with its <name>.sha256 asset, the newest by Debian version order.
  local -A url_of=() digest_of=()
  local name url digest
  while IFS=$'\t' read -r name url digest; do
    [ -n "$name" ] || continue
    url_of[$name]=$url
    digest_of[$name]=$digest
  done <<<"$assets"
  local best="" best_version="" without_sha=() version
  for name in "${!url_of[@]}"; do
    case "$name" in "$PACKAGE"_*_"$ARCH".deb) ;; *) continue ;; esac
    version=${name#"$PACKAGE"_}
    version=${version%_"$ARCH".deb}
    [[ $version =~ ^[0-9][A-Za-z0-9.~+]*$ ]] || continue
    # GitHub renames `~` to `.` in uploaded asset names, so a main build's asset reads 2026.11.23.0.main.165830.g614bccb
    # while the package's Version (and the name inside its .sha256) is 2026.11.23.0~main.165830.g614bccb (the version
    # format of build.yml). Take the package's version back, so the order and the checks below use it.
    if [[ $version =~ ^([0-9]{4}\.[0-9]{2}\.[0-9]{2}\.[0-9]+)\.main\.([0-9]{6}\.g[0-9a-f]+)$ ]]; then
      version="${BASH_REMATCH[1]}~main.${BASH_REMATCH[2]}"
    fi
    if [ -z "${url_of[$name.sha256]+set}" ]; then
      without_sha+=("$name")
      continue
    fi
    if [ -z "$best" ] || dpkg --compare-versions "$version" gt "$best_version" </dev/null; then
      best=$name best_version=$version
    fi
  done
  if [ -z "$best" ]; then
    if [ "${#without_sha[@]}" -gt 0 ]; then
      fail "The ${label#the } has ${without_sha[*]} without a .sha256 file, so it cannot be verified. Try again later (the upload may not be finished), or see $page"
    fi
    fail "The ${label#the } has no ${PACKAGE}_<version>_${ARCH}.deb. The package may still be building; see $page"
  fi
  ok "$best_version"

  local deb_url="${url_of[$best]}" sha_url="${url_of[$best.sha256]}" github_digest
  case "$deb_url" in "$allowed"*) ;; *) fail "The release names a download outside $REPO's releases ($deb_url): refusing it." ;; esac
  case "$sha_url" in "$allowed"*) ;; *) fail "The release names a download outside $REPO's releases ($sha_url): refusing it." ;; esac
  github_digest=$(hex64 "${digest_of[$best]#sha256:}") || true
  case "${digest_of[$best]}" in sha256:*) ;; *) github_digest="" ;; esac
  [ -n "$github_digest" ] || fail "GitHub gives no SHA-256 digest for $best, so it cannot be verified. See $page"

  bold "3/4 · Downloading and verifying"
  local deb="$dir/$best"
  curl "${curl_opts[@]}" --fail --max-filesize 200M -o "$deb" "$deb_url" </dev/null || fail "Downloading $best failed. Try again later."
  curl "${curl_opts[@]}" --fail --max-filesize 4K -o "$dir/package.sha256" "$sha_url" </dev/null || fail "Downloading $best.sha256 failed. Try again later."
  local actual published line file_name
  actual=$(sha256sum "$deb" | cut -d ' ' -f 1)
  # Like update::parse_sha256_file: the first non-empty line, `<hex>  <name>` or `<hex> *<name>`; a name, when
  # given, must be the package's: the asset's name, or the file name build-deb.sh wrote (with the version's `~`).
  line=$(grep -m 1 -v '^[[:space:]]*$' "$dir/package.sha256" || true)
  read -r published file_name _ <<<"$line" || true
  published=$(hex64 "${published:-}") || true
  file_name=${file_name#\*}
  if [ -z "$published" ] || { [ -n "$file_name" ] && [ "$file_name" != "$best" ] && [ "$file_name" != "${PACKAGE}_${best_version}_${ARCH}.deb" ]; }; then
    fail "$best.sha256 is not a checksum line for $best. Not installing."
  fi
  [ "$actual" = "$github_digest" ] || fail "The download does not match GitHub's digest of $best. Not installing."
  [ "$actual" = "$published" ] || fail "The download does not match the published $best.sha256. Not installing."
  ok "SHA-256 matches GitHub's digest and the published .sha256"
  local fields field want got
  fields=$(dpkg-deb -f "$deb" Package Architecture Version </dev/null 2>&1) || fail "dpkg-deb refused the file: $fields. Not installing."
  for field in Package Architecture Version; do
    case "$field" in Package) want=$PACKAGE ;; Architecture) want=$ARCH ;; Version) want=$best_version ;; esac
    got=$(printf '%s\n' "$fields" | sed -n "s/^$field: //p" | head -n 1)
    [ "$got" = "$want" ] || fail "The package's $field is \"${got:-missing}\", expected $want. Not installing."
  done
  ok "package $PACKAGE $best_version for $ARCH"

  bold "4/4 · Installing $PACKAGE $best_version"
  local installed
  installed=$(dpkg-query -W -f '${Status} ${Version}' "$PACKAGE" 2>/dev/null </dev/null | sed -n 's/^install ok installed //p') || true
  if [ -n "$installed" ] && [ "$installed" = "$best_version" ]; then
    ok "$PACKAGE $installed is already installed"
    exit 0
  fi
  if [ -n "$installed" ] && dpkg --compare-versions "$installed" gt "$best_version" </dev/null; then
    fail "$PACKAGE $installed is installed, newer than $best_version in $label. Remove it first (sudo apt remove $PACKAGE) to go back."
  fi
  # apt-get reads a local package as the user _apt; in a 0700 folder it warns and reads it as root instead. The file
  # is verified and nobody else can write it, so let _apt reach it: the folder only traversable, the file readable.
  chmod 711 "$dir"
  chmod 644 "$deb"
  # -y: running this installer is the confirmation (with `| bash`, apt could not ask on stdin anyway).
  local cmd=("${sudo[@]}" apt-get install -y "$deb")
  if [ "$dry_run" -eq 1 ]; then
    ok "Dry run: verified, not installed. The install command would be:"
    printf '    %s\n' "${cmd[*]}"
    echo "  (that folder is removed now; run again without --dry-run to install)"
    exit 0
  fi
  "${cmd[@]}" </dev/null || fail "apt-get could not install $best. Nothing else was changed."
  echo
  bold "Installed $PACKAGE $best_version ($channel channel)."
  echo "Open Launchpad from the app grid (or run launchpad-desktop) and link it to your Home."
  echo "Later versions: the companion's menu offers each update; or run this command again."
}

main "$@"
