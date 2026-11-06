#!/usr/bin/env bash
# Installs a Launchpad runner for the Home at @HOME_URL@ and pairs it.
# Served by Home: it downloads the exact build Home runs, so both always match.
# It changes nothing outside ~/.local/lib/launchpad-runner, ~/.local/bin/launchpad-runner,
# ~/.config/launchpad-runner, the user service launchpad-runner.service and systemd lingering for
# your user (so the runner keeps running after you log out). It never uses sudo.
set -euo pipefail

HOME_URL="@HOME_URL@"
COMMIT="@COMMIT@"
SHA256="@SHA256@"
NAME="${LAUNCHPAD_RUNNER_NAME:-$(hostname -s 2>/dev/null || hostname)}"
DIR="$HOME/.local/lib/launchpad-runner"
BIN="$DIR/launchpad"
CONFIG="$HOME/.config/launchpad-runner"
UNIT="$HOME/.config/systemd/user/launchpad-runner.service"

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$*"; }
note() { printf '  \033[33m!\033[0m %s\n' "$*"; }
fail() { printf '\n\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

bold "1/4 · Checking this machine"
BURST="Use Burst instead: on Home's setup page, choose Continue with Burst. Burst runs agents in a Hugging Face Job, with nothing to install."
[ "$(uname -s)" = Linux ] || fail "Runners need Linux x86_64. This machine runs $(uname -s) $(uname -m).
$BURST"
[ "$(uname -m)" = x86_64 ] || fail "Runners need Linux x86_64. This machine is Linux $(uname -m).
$BURST"
ok "Linux x86_64"
glibc="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
if [ -n "$glibc" ] && [ "$(printf '%s\n2.34\n' "$glibc" | sort -V | head -1)" != 2.34 ]; then
  fail "This machine has glibc $glibc. The runner needs glibc 2.34 or newer (Ubuntu 22.04, Debian 12 or newer)."
fi
ok "glibc ${glibc:-found}"
for tool in curl git sha256sum; do
  command -v "$tool" >/dev/null || fail "Install $tool first, for example: sudo apt install $tool"
done
ok "curl, git"
command -v bwrap >/dev/null || fail "Agents run in a bubblewrap jail. Install it, then run this command again:
    sudo apt install bubblewrap"
if ! bwrap --ro-bind / / --dev /dev --proc /proc --unshare-pid true 2>/dev/null; then
  fail "bubblewrap is installed but cannot create a jail. On Ubuntu 24.04, allow user namespaces for bwrap:
    sudo tee /etc/apparmor.d/bwrap >/dev/null <<'EOF'
abi <abi/4.0>,
include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
EOF
    sudo systemctl reload apparmor
Then run this command again."
fi
ok "bubblewrap works"
found=""
for harness in claude codex opencode; do
  command -v "$harness" >/dev/null && found="$found $harness"
done
if [ -n "$found" ]; then
  ok "agent CLIs:$found"
else
  note "No agent CLI found (claude, codex or opencode). Install the one your profile uses before you launch an agent."
fi
# Like huggingface_hub: HF_TOKEN, else the token file of hf auth login.
TOKEN="${HF_TOKEN:-}"
TOKEN_FILE="${HF_HOME:-$HOME/.cache/huggingface}/token"
if [ -z "$TOKEN" ] && [ -f "$TOKEN_FILE" ]; then TOKEN="$(cat "$TOKEN_FILE")"; fi
[ -n "$TOKEN" ] || fail "The runner needs your Hugging Face sign-in to reach this private Home. Run 'hf auth login' (or set HF_TOKEN), then run this command again."
ok "Hugging Face sign-in"

bold "2/4 · Installing Launchpad ${COMMIT:0:8}"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
code="$(curl -sSL -w '%{http_code}' -H "Authorization: Bearer $TOKEN" "$HOME_URL/_home/runner/launchpad" -o "$tmp")" || fail "Could not reach Home at $HOME_URL."
if [ "$code" = 401 ]; then
  fail "Home refused your Hugging Face sign-in (401). Run 'hf auth login' with an account that can open this Space, then run this command again."
elif [ "$code" != 200 ]; then
  fail "Could not download Launchpad from Home (HTTP $code): $(head -c 300 "$tmp")"
fi
echo "$SHA256  $tmp" | sha256sum -c --quiet - || fail "The download does not match Home's build. Run this command again."
mkdir -p "$DIR" "$HOME/.local/bin" "$CONFIG"
install -m 755 "$tmp" "$BIN"
cat > "$HOME/.local/bin/launchpad-runner" <<EOF
#!/bin/sh
# The runner's own CLI (status, url, connect). It never touches another Launchpad install.
LAUNCHPAD_ROLE=runner exec "$BIN" "\$@"
EOF
chmod 755 "$HOME/.local/bin/launchpad-runner"
ok "$BIN"
ok "launchpad-runner command in ~/.local/bin"
# The service has no shell environment: tell it where the token is, readable only by you.
if [ -n "${HF_TOKEN:-}" ]; then
  (umask 077; printf 'HF_TOKEN=%s\n' "$HF_TOKEN" > "$CONFIG/env")
elif [ "$TOKEN_FILE" != "$HOME/.cache/huggingface/token" ]; then
  (umask 077; printf 'HF_TOKEN_PATH=%s\n' "$TOKEN_FILE" > "$CONFIG/env")
else
  : > "$CONFIG/env"
fi

bold "3/4 · Starting the runner"
service=false
if command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; then
  mkdir -p "$(dirname "$UNIT")"
  cat > "$UNIT" <<EOF
[Unit]
Description=Launchpad runner for $HOME_URL
After=network-online.target

[Service]
Type=simple
ExecStart=$BIN daemon --role runner
EnvironmentFile=-$CONFIG/env
Restart=on-failure
RestartSec=2
KillMode=control-group
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable launchpad-runner.service >/dev/null 2>&1
  systemctl --user restart launchpad-runner.service
  service=true
  ok "launchpad-runner.service"
  # Without lingering, systemd stops user services when you log out. Best effort: never fail the install for it.
  user="${USER:-$(id -un)}"
  if loginctl show-user "$user" -p Linger 2>/dev/null | grep -q '=yes'; then
    ok "Keeps running after you log out (lingering is on)"
  elif loginctl enable-linger "$user" >/dev/null 2>&1; then
    ok "Keeps running after you log out (lingering enabled)"
  else
    note "Could not enable lingering: the runner stops when you log out. To keep it running, run: sudo loginctl enable-linger $user"
  fi
else
  note "No systemd user session. Start the runner yourself in another terminal: launchpad-runner daemon --role runner"
fi

bold "4/4 · Pairing with Home"
# A saved pairing counts only while Home still knows it: a Home recreated at the same address, or a runner removed
# in Settings, answers 401, and the runner then asks to pair again (the request waits for your approval).
LINK="$HOME/.local/share/launchpad-runner/home.json"
paired=""
if [ -f "$LINK" ] && grep -q "\"url\": \"$HOME_URL\"" "$LINK"; then
  link_id="$(sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' "$LINK" | head -1)"
  link_secret="$(sed -n 's/.*"secret": *"\([^"]*\)".*/\1/p' "$LINK" | head -1)"
  state="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $TOKEN" -H "X-Launchpad-Runner: $link_id.$link_secret" \
    "$HOME_URL/api/runners/request/$link_id" 2>/dev/null || true)"
  case "$state" in
    200) paired=yes ;;
    401) note "Home no longer knows this runner (Home was recreated or the runner was removed): pairing again." ;;
    *) paired=yes; note "Could not check the pairing with Home (HTTP ${state:-error}): keeping it. If the runner does not show up on Home, run: launchpad-runner connect $HOME_URL" ;;
  esac
fi
if [ -n "$paired" ]; then
  ok "Already paired with $HOME_URL"
else
  HF_TOKEN="$TOKEN" "$HOME/.local/bin/launchpad-runner" connect "$HOME_URL" --name "$NAME"
  $service && systemctl --user restart launchpad-runner.service
fi
echo
bold "Done: $NAME runs agents for $HOME_URL."
echo "  Runner status:  launchpad-runner status"
$service && echo "  Runner logs:    journalctl --user -u launchpad-runner -f"
echo "  Update later:   run the same install command again."
