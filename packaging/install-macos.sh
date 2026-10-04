#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${LAUNCHPAD_BIN_DIR:-$HOME/.local/bin}"
if [ "${1:-}" != --prebuilt ]; then
  (cd web && corepack enable && pnpm install --frozen-lockfile && pnpm build)
  cargo build --release --locked -p launchpad -p launchpad-desktop
fi
if [ -x "$BIN/launchpad" ] && curl -sf http://127.0.0.1:7317/api/health >/dev/null; then
  "$BIN/launchpad" update-install-lock
fi
mkdir -p "$BIN" "$HOME/Library/LaunchAgents" "$HOME/.local/share/launchpad"
# Keep the old daemon available until every replacement file is staged.
for binary in launchpad launchpad-desktop; do
  cp "target/release/$binary" "$BIN/.$binary.new"
  chmod 755 "$BIN/.$binary.new"
done
cp -R skills "$BIN/"
launchctl bootout "gui/$(id -u)/local.launchpad.desktop" 2>/dev/null || true
"$BIN/launchpad-desktop" --quit >/dev/null 2>&1 || true
launchctl bootout "gui/$(id -u)/local.launchpad.daemon" 2>/dev/null || true
# A daemon started by the desktop before installation is not managed by launchd.
pids=$(pgrep -f "^$BIN/launchpad daemon$" || true)
[ -z "$pids" ] || kill $pids
for binary in launchpad launchpad-desktop; do mv -f "$BIN/.$binary.new" "$BIN/$binary"; done
# XML escaping is required when a home path contains ampersands or quotes.
escaped=$(printf '%s' "$BIN" | sed 's/\&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/"/\&quot;/g')
for kind in daemon desktop; do
  if [ "$kind" = daemon ]; then binary=launchpad; arg=daemon; else binary=launchpad-desktop; arg=--hidden; fi
  plist="$HOME/Library/LaunchAgents/local.launchpad.$kind.plist"
  cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Label</key><string>local.launchpad.$kind</string><key>ProgramArguments</key><array><string>$escaped/$binary</string><string>$arg</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/></dict></plist>
EOF
  launchctl bootstrap "gui/$(id -u)" "$plist"
  if [ "$kind" = daemon ]; then
    # Wait before starting the warm desktop, whose fallback would otherwise race launchd.
    for ((wait_attempt=0;wait_attempt<100;wait_attempt++)); do
      curl -sf http://127.0.0.1:7317/api/health >/dev/null && break
      sleep 0.1
    done
    curl -sf http://127.0.0.1:7317/api/health >/dev/null || { echo 'The launchd daemon did not start.' >&2; exit 1; }
  fi
done
# Finder launcher, with the two executables still together in BIN.
mkdir -p "$HOME/Applications/Launchpad.app/Contents/MacOS" "$HOME/Applications/Launchpad.app/Contents/Resources"
cp desktop/src-tauri/icons/icon.icns "$HOME/Applications/Launchpad.app/Contents/Resources/icon.icns"
cat > "$HOME/Applications/Launchpad.app/Contents/Info.plist" <<'EOF'
<plist version="1.0"><dict><key>CFBundleExecutable</key><string>Launchpad</string><key>CFBundleIdentifier</key><string>local.launchpad</string><key>CFBundleIconFile</key><string>icon.icns</string><key>CFBundleName</key><string>Launchpad</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>
EOF
printf '#!/bin/bash\nexec %q "$@"\n' "$BIN/launchpad-desktop" > "$HOME/Applications/Launchpad.app/Contents/MacOS/Launchpad"
chmod 755 "$HOME/Applications/Launchpad.app/Contents/MacOS/Launchpad"
"$BIN/launchpad-desktop"
echo 'Launchpad installed in ~/Applications/Launchpad.app.'
