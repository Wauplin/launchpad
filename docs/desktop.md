# Desktop companion (Linux)

The desktop companion (`launchpad-desktop`) is a small Linux app that sits beside your browser. It shows what waits for you in the top bar, sends desktop notifications and opens Launchpad pages in your default browser. The full app stays in the browser: the companion has no window for agents, chats or settings.

It targets **Ubuntu 26.04 with GNOME on Wayland** only. Other desktops, X11 and older Ubuntu releases are not supported.

It is a client of Home, like the Chrome extension. It holds one device of its own, which shows in **Settings → Devices** as a Desktop companion. It links to one Home and follows one workspace.

## Install

Coming in this release: a `.deb` package for Ubuntu 26.04 (amd64), built in CI and attached to each release. Until then, build it from source (see [Development](development.md#other-components)).

## Link to your Home

The first start opens the **Connect Launchpad** window. Pick the path that matches your Home.

### Home on a Hugging Face Space

- **Sign in with Hugging Face** opens Hugging Face in your browser. Approve Launchpad there. The companion waits for the answer on a local address (`http://127.0.0.1:<port>/callback`) for up to five minutes.
- **Use a Hugging Face token instead**: paste an access token with read access (from huggingface.co/settings/tokens). This is the only path when the build has no Hugging Face sign-in app.

The companion then lists the Launchpad Spaces of your account and its organizations: Spaces tagged `launchpad-home`, or titled “Launchpad Home”. Choose yours. The Space's gateway checks your token and creates the device. Your Hugging Face token goes only to huggingface.co and to the Space's own `https://<name>.hf.space` address.

When Hugging Face no longer accepts the sign-in, the menu shows **Sign in again…**. It reruns only the Hugging Face step and keeps the device.

### Dev or self-hosted Home

1. Under **Dev or self-hosted Home**, type Home's address and click **Get a pairing code**. The address must use HTTPS, or plain HTTP on this computer (loopback).
2. The companion shows an eight-character code and opens **Settings → Devices** of that Home in your browser.
3. Enter the code there and click **Approve desktop companion**. The code expires after five minutes.

### The keyring

The companion keeps its device secret and your Hugging Face login only in the system keyring (Secret Service, GNOME Keyring on Ubuntu). A locked keyring asks you to unlock it. Without a keyring, the companion refuses to link and says why. The settings file holds no secret.

### Sign out

The menu's **Sign out** removes this device from Home, deletes the companion's keyring items for that Home and forgets the link. **Revoke** in **Settings → Devices** of any Launchpad window does the same from Home's side: the companion then deletes its secrets and shows **Connect to Home…** again.

## The top-bar menu

The companion puts an icon in the top bar. The icon shows an amber count of the items that wait for you in Today, the same count as the web app's (99+ above 99). The icon is grey while the companion is not live.

The menu holds, from top to bottom:

- **Header**: Home's name and the connection state (see below), plus a keyring problem when there is one.
- **Update to <version>**: shown when a newer companion exists (coming in this release, see [Updates](#updates)).
- **Connect to Home…**, **Sign in again…** or **Retry now**, depending on the state.
- **Needs you**: up to four Today items, each opening its card in the browser, then “N more…”, which opens Today. “Nothing waits for you” when the list is empty.
- **Running**, **Quota paused** and **Sleeping**: your agents in those states (not bot threads), each opening the agent in Agents.
- **Bots**: bots with threads that ask or work ("Name · 1 asking, 2 working"), each opening the bot's chat.
- **Ask…**, **Go to…** and **Quick launch…**: for now these open the web app's Ask panel, Today and the launch composer. The Spotlight replaces them (see [Spotlight](#spotlight)). Ask and Quick launch need a live connection.
- **Open Launchpad**: Today in the browser.
- **Workspace**: the workspace the companion follows, shown when you have more than one. The count, the lists and the notifications cover only that workspace.
- **Notifications** and **Start at login**: switches.
- **Sign out** and **Quit**.

Every link opens in your default browser and names the workspace (`ws=`).

On Ubuntu, the top-bar icon is drawn by the **Ubuntu AppIndicators** extension, which the Ubuntu session turns on by default. The plain “GNOME” session and a disabled extension show no icon. The companion keeps running without it: notifications, the dock badge and `launchpad-desktop --action …` still work.

## Connection states

The header of the menu shows one state:

| State | Meaning |
|---|---|
| Not connected | No Home is linked yet. |
| Connecting… | The companion opens the live event stream. |
| Live | The stream is open; the menu and the count follow Home as it changes. |
| Offline | Home did not answer three times. The header shows when the last data arrived. |
| Home is starting | The Space builds or wakes up. |
| Home is updating | Home installs an update; writes are paused. |
| Sign in again | Hugging Face no longer accepts the sign-in (Space only). |
| This device was removed from Home | The device was revoked. |
| Update Home | Home is older than this companion. |

The companion checks `GET /api/version` before each stream. When the stream drops, it tries again after 1, 5, 15 and 30 seconds, then every minute. It never asks more often than every 5 seconds while Home does not answer. **Retry now** cuts the wait.

## Notifications

With **Notifications** on (the default), the companion shows a desktop notification when a Today item of its workspace asks a question (“<title> asks”), fails (“<title> failed”) or when a bot asks you in a channel. Each notification has **Open**, which opens the item in the browser.

- At most one notification per agent per minute. A new one replaces the earlier one of the same agent.
- The first load after a start only reads the list and notifies nothing.
- After a gap (offline, a dropped stream), new items become one summary: "While you were away: 2 asking, 1 failed".
- Connection problems notify once each: offline for over two minutes (then “Launchpad is back online”), Home starting for over two minutes, sign in again, device removed, Home too old.

Notifications carry no token in their links. Home itself still sends no desktop notifications; Telegram is unchanged.

GNOME Shell shows an app's notifications only when the app's desktop file (`dev.launchpad.Desktop.desktop`) is installed. Without it, GNOME refuses them. The package installs it.

## Start at login and the dock badge

**Start at login** writes `~/.config/autostart/dev.launchpad.Desktop.desktop`, which starts the companion in the background at login (`--background`, no window). The first link turns it on. Once you turn it off, it stays off.

The companion also sends its count to the Ubuntu Dock (the Unity LauncherEntry signal). The dock shows it as a badge on the companion's icon while it is live, and hides it at zero.

## Command line

Only one companion runs at a time. A second `launchpad-desktop` hands its arguments to the running one and exits:

- `--action ask|goto|launch|open|today|connect|quit`: run a menu action.
- `--toggle`: open or close the Spotlight (bound to the global shortcut).
- `--background`: start without a window (used at login).
- `--quit`: stop the running companion.

## Spotlight

Coming in this release: a global shortcut (Ctrl+Alt+Space) that opens a small window to ask a bot, go to a Launchpad object or launch an agent.

## Updates

Coming in this release: the companion checks for a newer release and offers **Update to <version>** in the menu, which installs a verified package after your confirmation.

## Files

- Settings: `$XDG_CONFIG_HOME/launchpad-desktop/settings.toml` (usually `~/.config/launchpad-desktop/settings.toml`). It holds the linked Home, the workspace, the switches and an optional `hf_client_id`. It holds no secret.
- Logs: `$XDG_STATE_HOME/launchpad-desktop/companion.log` (usually `~/.local/state/launchpad-desktop/companion.log`), readable only by you and rotated at 1 MB. Tokens, secrets and codes are removed before a line is written. `LAUNCHPAD_DESKTOP_LOG=debug` shows more.

## Sources

- Hugging Face OAuth, loopback redirect URIs (any port for `http://127.0.0.1`), PKCE for public apps: https://huggingface.co/docs/hub/oauth
- Ubuntu session mode of Ubuntu 26.04, which enables `ubuntu-appindicators@ubuntu.com` and `ubuntu-dock@ubuntu.com`: https://git.launchpad.net/ubuntu/+source/gnome-shell/plain/debian/ubuntu-session-mods/ubuntu.json?h=ubuntu/resolute, installed by gnome-shell-common: https://packages.ubuntu.com/resolute/all/gnome-shell-common/filelist
- GNOME Shell 50.1 refuses notifications of an app without a desktop file (`InvalidAppError`): https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/js/ui/notificationDaemon.js
- LauncherEntry count on the dock (`com.canonical.Unity.LauncherEntry.Update`, read by Dash to Dock, the base of the Ubuntu Dock): https://raw.githubusercontent.com/micheleg/dash-to-dock/master/launcherAPI.js
