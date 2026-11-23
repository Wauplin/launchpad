# launchpad-desktop

The Linux desktop companion of Launchpad for Ubuntu GNOME on Wayland: a top-bar menu with live status, desktop notifications and browser hand-offs (the spotlight comes next). It is a thin client of Home, like the Chrome extension; the full UI stays in the browser. User documentation lives in `docs/`.

## Build

It is a Cargo workspace of its own (the root workspace excludes it). It needs the GTK 4 development files (GTK 4.8 or newer):

```sh
sudo apt install libgtk-4-dev
cargo check --manifest-path desktop/Cargo.toml --all-targets
cargo clippy --manifest-path desktop/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path desktop/Cargo.toml
```

Without root (an agent runner), put the Debian or Ubuntu packages in a local sysroot and point pkg-config at it:

```sh
S=/tmp/gtk4-apt; R=/tmp/gtk4-sysroot; mkdir -p $S/state/lists/partial $S/cache/archives/partial $S/debs $R
printf 'Dir::State "%s/state";\nDir::State::status "/var/lib/dpkg/status";\nDir::Cache "%s/cache";\nDebug::NoLocking "true";\n' $S $S > $S/apt.conf
APT_CONFIG=$S/apt.conf apt-get update
APT_CONFIG=$S/apt.conf apt-get install -s libgtk-4-dev | awk '/^Inst/ {print $2}' > $S/pkgs.txt
(cd $S/debs && APT_CONFIG=$S/apt.conf apt-get download $(cat $S/pkgs.txt))
for d in $S/debs/*.deb; do dpkg -x $d $R; done
# Point the development symlinks whose target is not in the sysroot at the system's runtime libraries.
L=$R/usr/lib/x86_64-linux-gnu
for f in $(find $L -maxdepth 1 -xtype l); do t=$(basename $(readlink $f)); for d in /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu; do [ -e $d/$t ] && ln -sf $d/$t $f; done; done
export PKG_CONFIG_SYSROOT_DIR=$R PKG_CONFIG_PATH=$L/pkgconfig:$R/usr/share/pkgconfig PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
export RUSTFLAGS="-C link-arg=-Wl,-rpath-link,$L -C link-arg=-L$L" LD_LIBRARY_PATH=$L:$R/lib/x86_64-linux-gnu
```

## Run

`launchpad-desktop` starts the companion (the first launch opens the connection window). A second launch hands its arguments to the running one: `--toggle` (the spotlight, bound to a global shortcut), `--action ask|goto|launch|open|today|connect|quit`, `--background` (autostart: no window), `--quit`. Logs go to stderr and `$XDG_STATE_HOME/launchpad-desktop/companion.log`, with secrets redacted; `LAUNCHPAD_DESKTOP_LOG=debug` shows more.

Sign in with Hugging Face uses a public OAuth app whose client ID comes from `LAUNCHPAD_HF_CLIENT_ID` at build time, or `hf_client_id` in `$XDG_CONFIG_HOME/launchpad-desktop/settings.toml`. Register the redirect URI `http://127.0.0.1/callback` (no port: Hugging Face accepts any port for loopback redirects). Without a client ID, the connection window takes a pasted Hugging Face token instead.

`cargo run --manifest-path desktop/Cargo.toml --example probe` checks pairing, `/api/version`, the event stream and the menu's groups against a scratch dev Home without a desktop or keyring (`LAUNCHPAD_PROBE_ORIGIN`, `LAUNCHPAD_PROBE_OWNER_TOKEN`); never point it at a real Home.
