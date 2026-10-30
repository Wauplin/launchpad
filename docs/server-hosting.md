# Running Launchpad on a server

This guide covers running the Launchpad daemon on a headless Linux host (a VPS, a home server or a laptop that stays on) and using it from a browser or a phone. One daemon runs everything: agents, terminals, the GitHub cache and the UI. Every device is a client of that daemon.

The daemon keeps listening on `127.0.0.1` and, by default, discovers an existing private Tailscale Serve HTTPS root route to its own port. Otherwise it also listens on its own running Tailscale node’s IP. Pairing prefers the matching HTTPS URL, then direct tailnet access. Set `[server] auto_tailnet = false` to keep loopback only. An explicit `public_origin` takes precedence over discovery. A gateway handles TLS for HTTPS features. Browser access uses pairing: see [Trusted remote access and pairing](../README.md#trusted-remote-access-and-pairing) in the README.

## 1. Install the daemon without the desktop shell

```sh
packaging/install.sh --headless             # build from source: daemon and CLI only
packaging/install.sh --headless --prebuilt  # from an unpacked prebuilt Linux package
```

`--headless` (Linux only):

- builds and installs only `~/.local/bin/launchpad` (no `launchpad-desktop`, no WebKitGTK build dependencies);
- installs no `.desktop` file, autostart entry or icons;
- installs and enables the systemd user unit `launchpad.service`;
- runs `loginctl enable-linger "$USER"`, so the user manager (and the daemon) starts at boot and keeps running after the last SSH session ends. If this fails, ask an administrator to run `sudo loginctl enable-linger <user>`.

Logs: `journalctl --user -u launchpad -f`.

## 2. Put a private door in front of it

Recommended: [Tailscale Serve](https://tailscale.com/kb/1312/serve). It gives HTTPS on your tailnet only, with no open port:

```sh
tailscale serve --bg 7317
```

The app is then at `https://<host>.<tailnet>.ts.net`. Settings → Devices → Pair a device offers Run command when Tailscale is installed, or Update HTTPS URL after manual setup; both can save and activate the matching private HTTPS origin and refresh the QR/link without restarting Launchpad. Startup discovery also recognizes matching routes when no explicit `public_origin` is configured. Alternatively, set `public_origin` in the `[server]` section of `~/.config/launchpad/config.toml` to that exact origin and restart when idle. Then run `launchpad pair` on the server and scan the QR code with each device. Alternatively, use a reverse proxy with an identity-aware login in front (Cloudflare Access, oauth2-proxy, …). Do not expose port 7317 directly to the internet.

The app must live at the root of a host name: the UI uses absolute paths (`/api`, `/ws`, `/assets`, `/auth`). Use a dedicated host name such as `launchpad.example.com`, not a sub-path such as `example.com/launchpad`.

## 3. First-run checklist (once, over SSH)

Each harness and the GitHub CLI use their own sign-in on the server. Settings → Harnesses → Sign-in shows which ones are missing and the command to run.

| Tool | Command | Notes |
|---|---|---|
| Claude Code | `claude auth login` (or start `claude` once) | Launchpad reads `~/.claude/.credentials.json` for quota readings. It never refreshes the Claude OAuth token. |
| Codex | `codex login --device-auth` | Device code flow works without a browser on the server. |
| OpenCode | `opencode auth login` | Or provider environment variables in a harness variant. |
| GitHub CLI | `gh auth login` | Used for the inbox, issues, PRs and publication. |
| Git push | SSH keys or `gh auth setup-git` | Needed to publish pre-draft PRs and merge worktrees into remote branches. |

Then clone or add projects: Projects → **Clone from GitHub** clones a repository into the projects root (`~/projects` by default) and registers it; **Scan projects root** finds existing checkouts.

## 4. Operations

- **Restarts interrupt agents.** The daemon owns the harness processes and terminals; restarting it (an update, a reboot) interrupts active turns. Update only when no agent is running; `install.sh` asks first when sessions are running.
- **Time zone.** Crons and the "at a date and time" start condition use the server's local time zone. Set it on purpose (`sudo timedatectl set-timezone Europe/Paris`). The cron and schedule editors show the server time zone when it differs from the browser's.
- **Backups.** Copy `state.db` with the SQLite backup API while the daemon runs, for example `sqlite3 ~/.local/share/launchpad/state.db ".backup /backups/launchpad-state.db"`. `cache.db` can be rebuilt and does not need a backup.
- **Several devices.** A focused window only marks sessions seen, and only suppresses notifications, when someone used it in the last minute, so an unattended laptop does not hide work from the phone. Composer drafts, PR viewed markers and unsent review notes are stored by the daemon and shared by every device. A terminal follows the size of the device that typed last; other devices see it letterboxed with a **Use this size** button.

## 5. Phones and other browsers

- **Phone layout.** Screens narrower than 720 px get the phone shell: Now, Agents, Launch, Review and Inbox tabs, and a workspace menu for Projects, Terminals, Devices and Settings. The workspace menu has **Use desktop layout**; Settings and zoom are per device.
- **Install and notifications.** Open the paired origin, then use the browser's Install / Add to Home Screen. In Settings → Devices, turn on notifications for this browser and choose whether session titles may show on the lock screen (off by default; notifications never contain code or messages). iOS delivers Web Push only to an app added to the Home Screen. Web Push needs HTTPS, which Tailscale Serve provides. The VAPID keys are in `vapid.json` in the data folder; back them up with `state.db` to keep existing subscriptions.
- **Links.** In a browser tab, GitHub links open in that browser, and Open in new tab replaces the desktop shell's new windows.
- **Flaky networks.** The UI pings the daemon, replays missed updates after reconnecting, resumes terminals from the last byte received and keeps unsent replies. Proxies must allow long-lived WebSockets on `/ws` and `/api/terminals/*/ws`.
- **Attachments.** Files upload once to the daemon; photos larger than 1.5 MB, larger than 2,048 px or in other formats are converted to JPEG in the browser first.

## Hugging Face Spaces (experimental)

The [Space connector components](../deploy/hf-space/README.md) are isolated from normal server packaging. They are not yet a deployable runtime: startup fails closed while HF app-visibility verification and the live isolation/recovery gates remain unresolved. See the [implementation and acceptance status](../deploy/hf-space/docs/acceptance.md) before provisioning a Space. Normal headless installations continue to use this guide and device pairing.
