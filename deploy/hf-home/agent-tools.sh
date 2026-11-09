#!/bin/sh
# Image-build only, as root. Home's Space image is also the Burst runners' image: these are the tools agents
# reached for in past sessions (and had to install or work around), so every Burst session finds them ready.
# Laptop runners and Local VMs get the same list from install-runner.sh and packaging/local-vm.sh.
set -eu

RUST_VERSION=1.93.1  # CI's toolchain (.github/workflows/ci.yml)
PNPM_VERSION=10.33.0 # web/package.json "packageManager"

apt-get update
apt-get install -y --no-install-recommends \
  build-essential cmake pkg-config \
  jq file xxd less patch unzip zip xz-utils bzip2 sqlite3 shellcheck wget ripgrep \
  procps lsof strace iproute2 dnsutils \
  chromium fonts-liberation fonts-noto-color-emoji
rm -rf /var/lib/apt/lists/*

# Burst jobs allow no user namespaces, so Chromium's own sandbox cannot start there (the job is the boundary).
# Debian's chromium wrapper reads this file.
cat > /etc/chromium.d/launchpad <<'EOF'
unshare -Ur true 2>/dev/null || CHROMIUM_FLAGS="$CHROMIUM_FLAGS --no-sandbox"
export CHROMIUM_FLAGS="$CHROMIUM_FLAGS --disable-dev-shm-usage"
EOF

# Rust without rustup: the standalone release in /usr/local, so it needs no RUSTUP_HOME in the agent's environment.
rust="rust-$RUST_VERSION-x86_64-unknown-linux-gnu"
cd /tmp
curl -fsSLO "https://static.rust-lang.org/dist/$rust.tar.xz"
curl -fsSL "https://static.rust-lang.org/dist/$rust.tar.xz.sha256" | sha256sum -c -
tar xJf "$rust.tar.xz"
"$rust/install.sh" --prefix=/usr/local --components=rustc,cargo,rust-std-x86_64-unknown-linux-gnu,clippy-preview,rustfmt-preview --disable-ldconfig
rm -rf "$rust" "$rust.tar.xz"

# A real pnpm executable: Corepack would download it again in every session's home.
npm install -g "pnpm@$PNPM_VERSION"
