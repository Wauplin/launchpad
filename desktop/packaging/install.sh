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
# GitHub's REST API is read with a token when one is at hand (GH_TOKEN, GITHUB_TOKEN, else `gh auth token`), sent only
# to api.github.com. When the API refuses (its anonymous limit is 60 requests an hour per address) or is out of reach,
# the asset list comes from the release's web page (releases/expanded_assets/<tag>, with GitHub's digest of each
# asset); a package the page shows no digest for is checked against the published .sha256 alone, and the output says so.
#
# Python is never used: GitHub's release JSON is read with jq when it works, else with the built-in reader (grep and
# awk), each checked on a sample before anything is fetched.
#
# Usage: install.sh [--main | --release] [--dry-run] [--help]
#   --main      the newest build of the main branch (the main-build prerelease; the default)
#   --release   the newest release
#   --dry-run   resolve, download and verify, then print the apt command instead of running it
#
# Environment:
#   GH_TOKEN or GITHUB_TOKEN=<token>     a GitHub token for the API request (else the GitHub CLI's login, if any)
#   LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1    skip the Ubuntu 26.04 check (never the amd64 check; unsupported)
#   LAUNCHPAD_DESKTOP_OS_RELEASE=<file>  read this file instead of /etc/os-release (tests)
#   LAUNCHPAD_DESKTOP_INSTALL_API=<url>  the GitHub API base, for a local fake release server (tests, with --dry-run
#                                        only), which also serves the web fallback under <url>/Wauplin/launchpad/releases/; the asset
#                                        URLs must then start with that base, plain http is allowed, and no token is sent
#
# The whole script is one function called on the last line, so a cut download runs nothing. With `curl … | bash`,
# stdin is the script itself: nothing here reads stdin (commands get </dev/null; sudo asks on the terminal).
set -euo pipefail

REPO="Wauplin/launchpad"
PACKAGE="launchpad-desktop"
ARCH="amd64"
RELEASES="https://github.com/$REPO/releases"
WORK_DIR=""

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

Environment: GH_TOKEN or GITHUB_TOKEN (a GitHub token for its API, else `gh auth token`), LAUNCHPAD_DESKTOP_SKIP_OS_CHECK=1 (skip the Ubuntu 26.04 check, unsupported),
LAUNCHPAD_DESKTOP_OS_RELEASE=<file> and LAUNCHPAD_DESKTOP_INSTALL_API=<url> (tests, --dry-run only).
USAGE
}

# A value of an os-release file (KEY=value, KEY="value" or KEY='value'), without sourcing it.
os_field() {
  sed -n "s/^$1=//p" "$2" | head -n 1 | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'$/\1/"
}

# Readers of GitHub's release JSON (on stdin). Both print the assets as lines `name<TAB>browser_download_url<TAB>digest`
# (digest empty when GitHub gives none) and leave out assets whose name or URL is not a string, or whose fields hold a
# control character. Exit 3 for a body that is not a release (an object with an `assets` array). Neither needs Python.
assets_jq() {
  # --slurp: exactly one JSON value (an empty body or a second value is not a release).
  jq -rs '
    if length == 1 and (.[0] | type) == "object" and (.[0].assets | type) == "array" then
      .[0].assets[] | objects | [.name, .browser_download_url, (.digest // "")]
      | select(all(.[]; type == "string" and (explode | all(. >= 32 and . != 127))))
      | join("\t")
    else error("not a release") end' 2>/dev/null || return 3
}

# The built-in reader, for machines without a working jq: grep cuts the JSON into tokens (strings, punctuation,
# literals), awk checks the grammar with a stack and keeps the three fields of each object of the top-level `assets`
# array. Fields with an escape other than \" \\ \/ are left out like control characters (asset names and URLs never
# need one); `digest` null or false reads as none, like jq's `//`.
assets_builtin() {
  LC_ALL=C grep -aoE '"([^"\\]|\\.)*"|[][{}:,]|[^][{}:,"[:space:]]+' | LC_ALL=C awk '
    BEGIN {
      depth = 0; want = "V"; seen = 0; failed = 0; BAD = sprintf("%c", 1)
      for (i = 1; i < 32; i++) ctl = ctl sprintf("%c", i)
      ctl = "[" ctl sprintf("%c", 127) "]"
    }
    function fail_() { failed = 1; exit }
    function done_value() { want = (depth == 0) ? "END" : "E" }
    function decode(t,    s, out, i, c) {
      s = substr(t, 2, length(t) - 2)
      if (s ~ ctl) return BAD
      if (index(s, "\\") == 0) return s
      out = ""
      for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (c == "\\") {
          c = substr(s, ++i, 1)
          if (c != "\"" && c != "\\" && c != "/") return BAD
        }
        out = out c
      }
      return out
    }
    function in_asset() { return depth == 3 && ctx[3] == "asset" }
    function non_string(k) {
      if (k == "\"name\"") sn = "bad"
      else if (k == "\"browser_download_url\"") su = "bad"
      else if (k == "\"digest\"") sd = "bad"
    }
    function set_field(k, t,    v, st) {
      v = decode(t); st = (v == BAD) ? "bad" : "ok"
      if (k == "\"name\"") { name = v; sn = st }
      else if (k == "\"browser_download_url\"") { url = v; su = st }
      else if (k == "\"digest\"") { digest = v; sd = st }
    }
    function open_(kind) {
      if (want != "V" && want != "A") fail_()
      if (in_asset()) non_string(key[3])
      depth++; type[depth] = kind; ctx[depth] = ""; key[depth] = ""
      if (depth == 1 && kind == "o") ctx[1] = "root"
      else if (depth == 2 && kind == "a" && ctx[1] == "root" && key[1] == "\"assets\"") { ctx[2] = "assets"; seen = 1 }
      else if (depth == 3 && kind == "o" && ctx[2] == "assets") { ctx[3] = "asset"; sn = su = sd = ""; digest = "" }
      want = (kind == "o") ? "K" : "A"
    }
    $0 == "{" { open_("o"); next }
    $0 == "[" { open_("a"); next }
    $0 == "}" {
      if (type[depth] != "o" || (want != "K" && want != "E")) fail_()
      if (in_asset() && sn == "ok" && su == "ok" && sd != "bad") print name "\t" url "\t" (sd == "ok" ? digest : "")
      depth--; done_value(); next
    }
    $0 == "]" {
      if (type[depth] != "a" || (want != "A" && want != "E")) fail_()
      depth--; done_value(); next
    }
    $0 == ":" { if (want != "C") fail_(); want = "V"; next }
    $0 == "," { if (want != "E") fail_(); want = (type[depth] == "o") ? "N" : "V"; next }
    /^"/ {
      if (want == "K" || want == "N") { key[depth] = $0; want = "C"; next }
      if (want != "V" && want != "A") fail_()
      if (in_asset()) set_field(key[3], $0)
      done_value(); next
    }
    {
      if (want != "V" && want != "A") fail_()
      if ($0 !~ /^(-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?|true|false|null)$/) fail_()
      if (in_asset()) {
        if (key[3] == "\"digest\"" && ($0 == "null" || $0 == "false")) { digest = ""; sd = "" }
        else non_string(key[3])
      }
      done_value()
    }
    END { if (failed || want != "END" || !seen) exit 3 }'
}

# The fallback reader: the asset list of GitHub's release web page (releases/expanded_assets/<tag>, HTML on stdin),
# as the same lines `name<TAB>url<TAB>digest`. Only links to this repository's downloads of that tag count (names of
# letters, digits and . _ ~ + -), with the digest GitHub shows next to each (`Copy to clipboard digest for <name>`,
# value `sha256:<hex>`; empty when none). Exit 3 when the page names no download.
assets_web() {
  local origin=$1 tag=${2//./\\.} prefix
  prefix="/$REPO/releases/download/$tag/"
  LC_ALL=C grep -aoE "aria-label=\"Copy to clipboard digest for [A-Za-z0-9._~+-]+\"[^>]*value=\"sha256:[0-9a-fA-F]{64}\"|href=\"${prefix}[A-Za-z0-9._~+-]+\"" \
    | LC_ALL=C awk -v origin="$origin" '
      /^aria-label=/ {
        name = $0; sub(/^aria-label="Copy to clipboard digest for /, "", name); sub(/".*/, "", name)
        d = $0; sub(/.*value="/, "", d); sub(/"$/, "", d)
        digest[name] = d; next
      }
      {
        path = substr($0, 7, length($0) - 7); name = path; sub(/.*\//, "", name)
        if (!(name in url)) { url[name] = origin path; order[++n] = name }
      }
      END {
        if (n == 0) exit 3
        for (i = 1; i <= n; i++) print order[i] "\t" url[order[i]] "\t" (order[i] in digest ? digest[order[i]] : "")
      }'
}

# The reader this machine can use: jq when it works, else the built-in one, each checked on a sample first (so a
# broken tool is reported as such before anything is fetched, never as a bad answer from GitHub). Prints its name.
pick_reader() {
  local sample expected
  sample='{"name":"x","body":"a \"quoted\" line\nwith café","n":[1,-2.5e3,true,null,{}],"assets":[
    {"name":"a_1.0_amd64.deb","browser_download_url":"https:\/\/example.invalid\/a.deb","digest":"sha256:00","size":12,"uploader":{"name":"u"}},
    {"name":"a.sha256","browser_download_url":"https://example.invalid/a.sha256","digest":null},
    {"name":7,"browser_download_url":"https://example.invalid/b"}],"tail":[]}'
  expected=$'a_1.0_amd64.deb\thttps://example.invalid/a.deb\tsha256:00\na.sha256\thttps://example.invalid/a.sha256\t'
  if command -v jq >/dev/null && [ "$(assets_jq <<<"$sample")" = "$expected" ]; then
    echo jq
  elif command -v grep >/dev/null && command -v awk >/dev/null && [ "$(assets_builtin <<<"$sample")" = "$expected" ]; then
    echo builtin
  else
    return 1
  fi
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
  for tool in curl sha256sum dpkg dpkg-deb dpkg-query apt-get; do
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
  ok "curl, sha256sum, dpkg-deb, apt-get${sudo[*]:+, sudo}"
  # GitHub's release data is JSON: read it with jq when it works, else with the built-in reader (grep and awk).
  # Python is never used (a broken python3 must not stop the install or be mistaken for a bad answer from GitHub).
  local reader
  reader=$(pick_reader) || fail "This installer reads GitHub's release data with jq, or with grep and awk, and none of them works here. Install jq first (sudo apt install jq), or check grep and awk."
  if [ "$reader" = jq ]; then ok "jq to read GitHub's release data"; else ok "built-in reader (grep, awk) for GitHub's release data"; fi

  # The API base and where downloads may come from. A fake release server (tests) may use http, and only its own
  # URLs are accepted then; it never leads to an install (--dry-run only), so a forgotten variable installs nothing.
  local api="https://api.github.com" web="https://github.com" allowed="$RELEASES/download/" protocols="=https"
  if [ -n "${LAUNCHPAD_DESKTOP_INSTALL_API:-}" ]; then
    [ "$dry_run" -eq 1 ] || fail "LAUNCHPAD_DESKTOP_INSTALL_API is set (a fake release server for tests): it works only with --dry-run. Unset it to install from GitHub."
    api="${LAUNCHPAD_DESKTOP_INSTALL_API%/}"
    web=$api
    allowed="$api/"
    protocols="=http,https"
    note "Using the release server $api (LAUNCHPAD_DESKTOP_INSTALL_API), not GitHub"
  fi
  local curl_opts=(--silent --show-error --location --proto "$protocols" --proto-redir "$protocols" --connect-timeout 15 --retry 2)

  # The private folder goes on every exit: the EXIT trap, and an exit on Ctrl+C, a closed terminal or kill (bash
  # runs no EXIT trap when a signal ends it).
  trap 'if [ -n "$WORK_DIR" ]; then rm -rf -- "$WORK_DIR"; fi' EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
  WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/launchpad-desktop-install.XXXXXXXX")
  chmod 700 "$WORK_DIR"
  local dir=$WORK_DIR

  local path label page
  if [ "$channel" = main ]; then
    path="releases/tags/main-build" label="the main-build prerelease" page="$RELEASES/tag/main-build"
  else
    path="releases/latest" label="the latest release" page="$RELEASES/latest"
  fi
  bold "2/4 · Finding the package in $label of $REPO"
  # GitHub's REST API answers 60 anonymous requests an hour per address (shared by everything behind one NAT), so
  # the request carries a token when one is at hand: GH_TOKEN, GITHUB_TOKEN, else the GitHub CLI's login. The token
  # goes only to api.github.com (never to downloads or a fake release server), from a 0600 file (never in the process
  # list), and is never printed.
  local auth=() token="" token_from=""
  if [ "$api" = "https://api.github.com" ]; then
    if [ -n "${GH_TOKEN:-}" ]; then token=$GH_TOKEN token_from=GH_TOKEN
    elif [ -n "${GITHUB_TOKEN:-}" ]; then token=$GITHUB_TOKEN token_from=GITHUB_TOKEN
    elif command -v gh >/dev/null && token=$(gh auth token --hostname github.com 2>/dev/null </dev/null) && [ -n "$token" ]; then
      token_from="the GitHub CLI's login"
    else token=""
    fi
    if [ -n "$token" ] && ! [[ $token =~ ^[A-Za-z0-9_.-]+$ ]]; then
      note "The token from $token_from does not look like a GitHub token: not using it"
      token=""
    fi
    if [ -n "$token" ]; then
      (umask 077 && printf 'Authorization: Bearer %s\n' "$token" >"$dir/auth")
      auth=(-H "@$dir/auth")
      ok "Reading GitHub's API with the token of $token_from"
    fi
    token=""
  fi
  local status assets="" source="" rc=0
  status=$(curl "${curl_opts[@]}" --max-filesize 20M "${auth[@]}" -H "Accept: application/vnd.github+json" -o "$dir/release.json" -w '%{http_code}' "$api/repos/$REPO/$path" </dev/null) || status=000
  rm -f -- "$dir/auth"
  case "$status" in
    200)
      assets=$("assets_$reader" <"$dir/release.json") || rc=$?
      case "$rc" in
        0) source=api ;;
        3) fail "GitHub's answer for $label is not a release. Try again later, or see $page" ;;
        *) fail "Reading GitHub's answer with the $reader reader failed (exit $rc). Not a GitHub problem: see $page to download the package by hand." ;;
      esac
      ;;
    404) fail "Found no ${label#the } of $REPO. See $RELEASES" ;;
    000) note "Cannot reach $api: reading the release's web page instead" ;;
    401) note "GitHub's API refused the token of $token_from (HTTP 401): reading the release's web page instead" ;;
    403 | 429) note "GitHub's API refused the request (HTTP $status), probably its rate limit${auth[*]:+ for this token}${auth[*]:- for anonymous requests}: reading the release's web page instead" ;;
    *) note "GitHub's API answered HTTP $status: reading the release's web page instead" ;;
  esac

  # The fallback, outside the REST API's limit: the release's asset list as GitHub's web page shows it
  # (releases/expanded_assets/<tag>), with each asset's download link and GitHub's SHA-256 digest.
  if [ -z "$source" ]; then
    local tag=main-build hint="Set GH_TOKEN to a GitHub token (any token, no scope needed), or log in with \`gh auth login\`, and run this again; or download the package by hand from $page"
    local web_opts=(--silent --show-error --proto "$protocols" --connect-timeout 15 --retry 2)
    if [ "$channel" = release ]; then
      # releases/latest redirects to releases/tag/<tag>: read the tag from the redirect, without following it.
      local redirect
      redirect=$(curl "${web_opts[@]}" -o /dev/null -w '%{redirect_url}' "$web/$REPO/releases/latest" </dev/null) \
        || fail "Cannot reach $web to find $label. Check the network and try again."
      tag=${redirect##*/releases/tag/}
      [[ $redirect == */releases/tag/* && $tag =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] \
        || fail "GitHub's web page did not name $label (${redirect:-no redirect}). $hint"
    fi
    status=$(curl "${web_opts[@]}" --location --proto-redir "$protocols" --max-filesize 20M -o "$dir/assets.html" -w '%{http_code}' \
      "$web/$REPO/releases/expanded_assets/$tag" </dev/null) || fail "Cannot reach $web to read $label. Check the network and try again."
    case "$status" in
      200) ;;
      404) fail "Found no ${label#the } of $REPO. See $RELEASES" ;;
      *) fail "GitHub's web page answered HTTP $status for $label. $hint" ;;
    esac
    assets=$(assets_web "$web" "$tag" <"$dir/assets.html") || fail "GitHub's web page for $label lists no downloads. $hint"
    source=web
    ok "Read the asset list from $web/$REPO/releases/tag/$tag"
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
  if [ -z "$github_digest" ]; then
    [ "$source" = web ] || fail "GitHub gives no SHA-256 digest for $best, so it cannot be verified. See $page"
    # The web page is GitHub's HTML, which may change: without its digest the published .sha256 alone must match.
    note "GitHub's web page shows no digest for $best: verifying against the published .sha256 only"
  fi

  bold "3/4 · Downloading and verifying"
  local deb="$dir/$best"
  curl "${curl_opts[@]}" --fail --max-filesize 200M -o "$deb" "$deb_url" </dev/null || fail "Downloading $best failed. Try again later."
  curl "${curl_opts[@]}" --fail --max-filesize 4K -o "$dir/package.sha256" "$sha_url" </dev/null || fail "Downloading $best.sha256 failed. Try again later."
  local actual published line file_name
  actual=$(sha256sum "$deb" | cut -d ' ' -f 1)
  # Like update::parse_sha256_file: the first non-empty line, `<hex>  <name>` or `<hex> *<name>`; a name, when
  # given, must be the package's: the asset's name, or the file name build-deb.sh wrote (with the version's `~`).
  line=$(grep -m 1 -v '^[[:space:]]*$' "$dir/package.sha256" || true)
  line=${line%$'\r'}
  read -r published file_name _ <<<"$line" || true
  published=$(hex64 "${published:-}") || true
  file_name=${file_name#\*}
  if [ -z "$published" ] || { [ -n "$file_name" ] && [ "$file_name" != "$best" ] && [ "$file_name" != "${PACKAGE}_${best_version}_${ARCH}.deb" ]; }; then
    fail "$best.sha256 is not a checksum line for $best. Not installing."
  fi
  [ -z "$github_digest" ] || [ "$actual" = "$github_digest" ] || fail "The download does not match GitHub's digest of $best. Not installing."
  [ "$actual" = "$published" ] || fail "The download does not match the published $best.sha256. Not installing."
  if [ -n "$github_digest" ]; then ok "SHA-256 matches GitHub's digest and the published .sha256"; else ok "SHA-256 matches the published .sha256"; fi
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
