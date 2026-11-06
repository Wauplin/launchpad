#!/usr/bin/env bash
# Creates a Local VM runner with Multipass: an Ubuntu virtual machine with Docker and a Launchpad runner whose
# boundary is the VM itself (agents run there without a jail, under Launchpad's network capture, and may use Docker,
# which needs root; agents cannot use sudo). Run it on the laptop; it installs nothing on the laptop itself except
# the VM.
#
# Usage: packaging/local-vm.sh <Home address> [--name launchpad-vm] [--cpus 4] [--memory 8G] [--disk 40G]
#
# Then approve the code it prints on Home (Settings → Home and runners). The VM's runner starts with the VM.
set -euo pipefail
HOME_URL="${1:-}"
if [ -z "$HOME_URL" ] || [[ "$HOME_URL" == -* ]]; then
  echo "Usage: packaging/local-vm.sh <Home address> [--name launchpad-vm] [--cpus 4] [--memory 8G] [--disk 40G]" >&2
  exit 2
fi
shift
NAME=launchpad-vm CPUS=4 MEMORY=8G DISK=40G
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --cpus) CPUS="$2"; shift 2 ;;
    --memory) MEMORY="$2"; shift 2 ;;
    --disk) DISK="$2"; shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
command -v multipass >/dev/null || { echo "Install Multipass first: sudo snap install multipass" >&2; exit 1; }
BIN="$(command -v launchpad || echo "$HOME/.local/bin/launchpad")"
[ -x "$BIN" ] || { echo "No launchpad binary on this machine: install Launchpad first." >&2; exit 1; }

if ! multipass info "$NAME" >/dev/null 2>&1; then
  multipass launch 24.04 --name "$NAME" --cpus "$CPUS" --memory "$MEMORY" --disk "$DISK"
fi
run() { multipass exec "$NAME" -- bash -lc "$1"; }

echo "Installing Docker, bubblewrap and Git in $NAME…"
run 'command -v docker >/dev/null || (curl -fsSL https://get.docker.com | sudo sh); sudo usermod -aG docker "$USER"; sudo apt-get install -y -qq git bubblewrap unzip >/dev/null'
echo "Installing Claude Code and Codex in $NAME (their vendors' installers)…"
run 'command -v claude >/dev/null || curl -fsSL https://claude.ai/install.sh | bash'
run 'command -v codex >/dev/null || (command -v npm >/dev/null || sudo apt-get install -y -qq nodejs npm >/dev/null; sudo npm install -g @openai/codex >/dev/null)'

echo "Copying Launchpad into $NAME…"
multipass transfer "$BIN" "$NAME:/tmp/launchpad"
run 'install -Dm755 /tmp/launchpad ~/.local/bin/launchpad && rm /tmp/launchpad'
# The VM is the boundary: no jail inside, Docker allowed.
run 'mkdir -p ~/.config/launchpad-runner && f=~/.config/launchpad-runner/config.toml; grep -q "^\[runner\]" "$f" 2>/dev/null || printf "\n[runner]\nboundary = \"vm\"\n" >> "$f"'
# Agents run under Launchpad's network capture: the runner proxy's CA goes into the VM's system store.
run 'sudo install -m 644 "$(LAUNCHPAD_ROLE=runner ~/.local/bin/launchpad proxy-ca)" /usr/local/share/ca-certificates/launchpad-runner.crt && sudo update-ca-certificates >/dev/null'
run 'mkdir -p ~/.config/systemd/user && cat > ~/.config/systemd/user/launchpad-runner.service <<UNIT
[Unit]
Description=Launchpad runner (Local VM)
After=network-online.target

[Service]
Type=simple
ExecStart=%h/.local/bin/launchpad daemon --role runner
Restart=on-failure
RestartSec=2
KillMode=control-group
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
UNIT
sudo loginctl enable-linger "$USER"; systemctl --user daemon-reload; systemctl --user enable --now launchpad-runner.service'

if [[ "$HOME_URL" == *.hf.space* ]]; then
  echo "A Home in a private Space needs your Hugging Face sign-in in the VM. If pairing fails, run: multipass exec $NAME -- hf auth login (or set HF_TOKEN there), then run this script again."
fi
echo "Pairing $NAME with $HOME_URL: approve the code below on Home (Settings → Home and runners)."
run "LAUNCHPAD_ROLE=runner ~/.local/bin/launchpad connect '$HOME_URL' --name '$NAME'"
