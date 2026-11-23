# Desktop companion (Linux)

The desktop companion (`launchpad-desktop`) is a small Linux app that sits beside your browser. It shows what waits for you in the top bar, sends desktop notifications and opens Launchpad pages in your default browser. The full app stays in the browser: the companion has no window for agents, chats or settings.

It targets **Ubuntu 26.04 with GNOME on Wayland** only. Other desktops, X11 and older Ubuntu releases are not supported.

It is a client of Home, like the Chrome extension. It holds one device of its own, which shows in **Settings → Devices** as a Desktop companion. It links to one Home and follows one workspace.

## Install

The companion comes as a `.deb` package for **Ubuntu 26.04 on amd64** only. The quickest way to install it is one command:

```sh
curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/desktop/packaging/install.sh | bash
```

This installs the newest build of `main` (the `main-build` prerelease). For the latest release instead, run `curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/desktop/packaging/install.sh | bash -s -- --release`. While the latest release has no `.deb`, it stops with an error that says so.

The script:

- Checks that the machine runs Ubuntu 26.04 on amd64, and stops with a clear error otherwise.
- Finds the newest `launchpad-desktop_<version>_amd64.deb` of `main-build` (or of the latest release) from GitHub's release data. It takes only a package that has its `.sha256` file.
- Downloads it into a private temporary folder, which it removes when it ends.
- Checks the file before it installs anything. Its SHA-256 must equal the digest GitHub gives for the asset and the published `.sha256` file. The package's own fields must say `launchpad-desktop`, `amd64` and the expected version.
- Only then runs `sudo apt-get install`. Apt installs the libraries it needs (GTK 4) and recommends `gnome-keyring`, `xdg-desktop-portal-gnome` and `packagekit`.
- Stops without changes when the same version is already installed. When a newer version is installed, it refuses and tells you to remove it first (`sudo apt remove launchpad-desktop`).

Add `--dry-run` (`… | bash -s -- --dry-run`) to check without installing: the script finds, downloads and checks the package, then prints the apt command instead of running it. For tests, `LAUNCHPAD_DESKTOP_INSTALL_API` points the script at a local fake release server; it works only with `--dry-run`.

To install by hand instead:

1. Download `launchpad-desktop_<version>_amd64.deb` and its `.sha256` file from the [latest release](https://github.com/Wauplin/launchpad/releases/latest). Builds of the newest `main` commits are on the `main-build` prerelease.
2. Check the download: `sha256sum -c launchpad-desktop_<version>_amd64.deb.sha256`.
3. Install it: `sudo apt install ./launchpad-desktop_<version>_amd64.deb`.

A build of `main` has a version like `2026.11.23.0~main.165830.g614bccb`. GitHub stores the `~` of a file name as `.`, so its files are named `launchpad-desktop_2026.11.23.0.main.165830.g614bccb_amd64.deb` and `….deb.sha256`. The `.sha256` file still names the package with `~`, so `sha256sum -c` does not find the download. Rename the `.deb` to the name with `~` first, or run `sha256sum` on it and compare the result with the first word of the `.sha256` file. Release files have no `~` and need neither.

The package installs:

- `/usr/bin/launchpad-desktop`.
- The desktop file `dev.launchpad.Desktop.desktop`, so the companion shows in the app grid as **Launchpad**. Its dock menu has **Ask Launchpad**, **Go to…**, **Quick launch** and **Open Today**. GNOME needs this file for notifications, the global shortcut and the dock badge.
- Its icons.

The package adds no start at login for every user. The first link turns **Start at login** on for you (see [Start at login and the dock badge](#start-at-login-and-the-dock-badge)).

To remove it: `sudo apt remove launchpad-desktop`. Sign out first if you also want to remove its device from Home.

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
- **Update to <version>**: shown when a newer companion exists; the item then follows the update's steps (see [Updates](#updates)).
- **Connect to Home…**, **Sign in again…** or **Retry now**, depending on the state.
- **Needs you**: up to four Today items, each opening its card in the browser, then “N more…”, which opens Today. “Nothing waits for you” when the list is empty.
- **Running**, **Quota paused** and **Sleeping**: your agents in those states (not bot threads), each opening the agent in Agents.
- **Bots**: bots with threads that ask or work ("Name · 1 asking, 2 working"), each opening the bot's chat.
- **Ask…** opens the [Spotlight](#spotlight), **Go to…** opens it with `>` (Go to only), and **Quick launch…** opens the quick launch form. Ask and Quick launch need a live connection.
- **Open Launchpad**: Today in the browser.
- **Shortcut**: the active global shortcut, then **Change shortcut…** (portal) or **Use a GNOME custom shortcut** (when the portal gave none).
- **Settings…**: the companion's settings window (see [Commands and settings](#commands-and-settings)).
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
| Update the companion | Home needs a newer companion (see [Updates](#updates)). |

The companion checks `GET /api/version` before each stream. When the stream drops, it tries again after 1, 5, 15 and 30 seconds, then every minute. It never asks more often than every 5 seconds while Home does not answer. **Retry now** cuts the wait.

## Notifications

With **Notifications** on (the default), the companion shows a desktop notification when a Today item of its workspace asks a question (“<title> asks”), fails (“<title> failed”) or when a bot asks you in a channel. Each notification has **Open**, which opens the item in the browser.

- At most one notification per agent per minute. A new one replaces the earlier one of the same agent.
- The first load after a start only reads the list and notifies nothing.
- After a gap (offline, a dropped stream), new items become one summary: "While you were away: 2 asking, 1 failed".
- Connection problems notify once each: offline for over two minutes (then “Launchpad is back online”), Home starting for over two minutes, sign in again, device removed, Home too old.

`/dnd` in the Spotlight pauses them for one hour. An answer or a question of an ask you sent from the Spotlight also notifies you while the Spotlight is hidden. Notifications carry no token in their links. Home itself still sends no desktop notifications; Telegram is unchanged.

GNOME Shell shows an app's notifications only when the app's desktop file (`dev.launchpad.Desktop.desktop`) is installed. Without it, GNOME refuses them. The package installs it (see [Install](#install)).

## Start at login and the dock badge

**Start at login** writes `~/.config/autostart/dev.launchpad.Desktop.desktop`, which starts the companion in the background at login (`--background`, no window). The first link turns it on. Once you turn it off, it stays off.

The companion also sends its count to the Ubuntu Dock (the Unity LauncherEntry signal). The dock shows it as a badge on the companion's icon while it is live, and hides it at zero.

## Command line

Only one companion runs at a time. A second `launchpad-desktop` hands its arguments to the running one and exits:

- `--action ask|goto|launch|open|today|connect|quit`: run a menu action.
- `--toggle`: open or close the Spotlight (what the global shortcut runs). A plain second `launchpad-desktop` does the same once the companion is linked.
- `--background`: start without a window (used at login).
- `--quit`: stop the running companion.

## Spotlight

The Spotlight is a small window over your desktop. Use it to ask a bot, go to a Launchpad object or launch an agent without switching to the browser.

### Opening it

- **Ctrl+Alt+Space** opens and closes it. The companion registers this shortcut through the GlobalShortcuts portal of xdg-desktop-portal. The first time, GNOME asks you to allow the shortcut; later starts bind it again without asking.
- **Change shortcut…** in the menu opens GNOME's own dialog to pick another key.
- When the portal gives no shortcut (you refused it, or it failed), the menu offers **Use a GNOME custom shortcut**. This adds one custom shortcut, "launchpad-desktop", that runs `launchpad-desktop --toggle` with Ctrl+Alt+Space. It shows in **Settings → Keyboard → Custom Shortcuts**, and your other custom shortcuts stay as they are. **Remove GNOME custom shortcut** in the companion's settings takes it out and goes back to the portal.
- `launchpad-desktop --toggle` always works, from any launcher or script.
- The menu's **Ask…** and **Go to…** open it too.

The portal and the custom shortcut need the installed desktop file `dev.launchpad.Desktop.desktop` and `launchpad-desktop` on your PATH. The package installs both (see [Install](#install)).

### What you type

| Input | What the list shows |
|---|---|
| Nothing | Needs you, then the 5 rows you opened most recently. |
| Free text | **Ask <bot>** (or **Launch a new agent** when the target is New agent), up to 6 Go to matches, **Launch a new agent with this prompt instead** and **Continue in the browser**. |
| `@x` | The bots whose name matches; Enter completes `@slug `. |
| `@bot text` | **Ask <bot>** with that text; "No bot named x" for an unknown name. |
| `>text` | Go to matches only (up to 12). |
| `/` | The commands (see [Commands and settings](#commands-and-settings)). |
| `lp:…` or a Launchpad address, alone | **Open**: Home resolves it and the browser opens it at that place. |

Text of more than 4,000 characters offers no Ask: continue in the browser with Ctrl+D. While Home is not connected, the list shows no Ask or Launch rows; Go to and the browser still work. The footer always says what Enter does.

### Keys

| Key | Action |
|---|---|
| ↑ ↓ | Move the selection (headings are skipped). |
| Enter, or a click | Run the selected row. |
| Ctrl+Enter | Ask, then open the thread in the browser. |
| Tab, Shift+Tab | Next or previous Ask target. |
| Ctrl+D | Continue in the browser (see [Hand-off](#hand-off)). |
| Ctrl+O | Open the current answer's thread in the browser. |
| Ctrl+N | Start a new ask. |
| Ctrl+W | Next workspace. |
| Esc | Hide; the text is kept for 15 minutes. Clicking elsewhere does the same. |
| Shift+Esc | Hide and discard the text. |

Every control has a keyboard mnemonic, and the footer is a status line that screen readers announce.

### Ask

- **Target**: the bot set as **Default Ask target** in the companion's settings, while it exists. Otherwise the web app's rule: the workspace's oldest admin bot, else its oldest bot. Tab cycles through the other bots (alphabetical), then New agent. `@bot` at the start of the text picks a bot for that ask.
- **Sending**: each ask starts a new thread of the bot, sent once even after a retry.
- **Answer**: the bot's answer shows in the Spotlight as it arrives, up to 1,200 characters. Ctrl+O, or Ctrl+Enter when you send, opens the whole thread in the browser.
- **Replies**: text you type under an answer goes to the same thread. When the bot asks a question, your text answers it. An approval request is never answered from the Spotlight: Allow or Deny it in the browser.
- **Closed Spotlight**: when an answer or a question arrives while the Spotlight is hidden, one notification tells you, and a click opens the thread.

### Go to

The search covers the companion's workspace: fixed pages, Today cards, agents (not bot threads or archived agents), open chats, bots, channels and DMs, folios, open Local PRs and folders. A word that starts a name ranks first, then a word inside it, then letters in order. Ties go to what needs you, then to the most recent. Every result opens in the browser.

A link or a short ID (`lp:chat/…`, see [Links and references](links.md)) typed alone opens that object, like the web app's Go to.

### Quick launch

Enter on a Launch row, `/launch [text]` or **Quick launch…** in the menu opens the quick launch form:

- **Prompt**.
- **Folder**: Temporary folder, or a Git or plain folder of the workspace. The default is the folder you used last, else the folder of your newest agent.
- **When done**: Keep, Local PR, Merge or Read only (Git folders only). The default is the folder's own setting; Draft PR and Open PR become Local PR, since the quick launch never publishes.
- **Profile**: the one you used last, else the first.

Only the **Launch** button, or Ctrl+Enter while the form has focus, starts an agent. The Enter key alone never does. A second click or a retry launches once. The browser then opens at the new agent. Paid usage is always off. For anything else, use **Continue in the browser** (Ctrl+D) to finish in the composer.

### Hand-off

Ctrl+D (or **Continue in the browser**) moves your text to the browser:

- **For a launch**: the companion adds the text to Home's launch composer draft, after any text you already had there, and opens the composer. Nothing is lost when the browser changed the draft meanwhile. When the draft cannot be saved, a text of up to 2,000 characters goes in the link instead.
- **For an ask**: the companion copies the text to the clipboard and opens a new chat with the bot. Paste it there.

### Commands and settings

| Command | Action |
|---|---|
| `/launch [text]` | Open the quick launch form with the text. |
| `/workspace` | Follow the next workspace. |
| `/today` | Open Today in the browser. |
| `/dnd` | Pause the companion's notifications for 1 hour. |
| `/settings` | Open the settings window. |

The settings window (**Settings…** in the menu) shows the shortcut's state with its buttons, the **Default Ask target**, **Notifications** and **Start at login**.

Message search, pinned launches, option buttons for questions and a chat draft on Home wait for a later release (see [Not in this release](#not-in-this-release)).

## Updates

The companion checks GitHub for a newer version 20 seconds after it starts, then every 6 hours. A failed check tries again later, after 15 minutes at first, and up to 6 hours apart. The check reads the public releases of Wauplin/launchpad, without any token.

- **Channel**: a release build follows releases; a build of `main` follows the `main-build` prerelease. Set `update_channel = "release"` or `"main"` in the [settings file](#files) to change it. A build you made yourself checks nothing.
- **Offer**: when a newer package exists, the menu shows **Update to <version>**, and one notification per version says so.
- **Never automatic**: nothing is downloaded or installed until you click **Update to <version>**.

The click:

1. Downloads the package from the repository's release into a private folder in `~/.cache/launchpad-desktop/`.
2. Checks it three ways. Its SHA-256 must equal the digest GitHub gives for the asset and the published `.sha256` file. A `main-build` file name has `.main.` where the version has `~main.` (see [Install](#install)); the companion reads the real version from the name, and accepts a `.sha256` file that names either form. Then the package's own fields must say `launchpad-desktop`, `amd64` and the offered version. Any mismatch refuses the package and deletes it.
3. Installs it through PackageKit. Ubuntu asks for your password in its own dialog; that is your confirmation. Without PackageKit, it uses `pkexec apt-get install` instead. Without both, it shows the `sudo apt install` command to run.
4. Restarts the companion in the new version, in the background.

The menu item follows these steps: **Downloading <version>…**, **Verifying <version>…**, **Installing <version>… (confirm in the password dialog)** and **Restarting into <version>…**. A failure shows **Update to <version> failed: <reason> · Retry**, and a click tries again. Canceling the password dialog is a failure too; nothing changed.

When Home needs a newer companion, the menu header says **Update the companion** next to the update item.

## Not in this release

The first release puts the Spotlight, the menu and linking first. These parts of the design wait:

- **A status window**: the menu already lists what needs you, running agents and busy bots.
- **A search provider for GNOME Activities**: the Spotlight covers search with its own shortcut.
- **A GNOME Shell extension**: the standard tray icon needs none, and an extension must follow each GNOME release.
- **KDE, other desktops and X11**: the first target is Ubuntu 26.04 GNOME on Wayland only.
- **Ubuntu 24.04**: its GNOME 46 comes before GNOME 48, the first release with a GlobalShortcuts portal backend, and the package is built against the libraries of 26.04.
- **Chat drafts on Home**: an ask handed to the browser goes through the clipboard until Home keeps chat drafts.
- **An `lp:` link handler**: opening `lp:` IDs from other apps needs a URL scheme handler; paste them in the Spotlight for now.
- **Several Homes**: one companion links to one Home.
- **Scoped tokens**: the companion's device has full access, like a browser's.
- **A search API on Home**: Go to searches lists the companion loads itself.
- **Search in message text** and topics, routines, oracles and folio pages in Go to: they need that search API or more lists.
- **One notification across clients**: the browser, the extension and the companion each notify on their own.
- **An apt repository with signed packages**: it needs a signing key that you create. Updates are verified by checksums meanwhile.
- **Starting by a notification click** (`DBusActivatable`): a notification clicked after the companion quit does nothing.
- **Pinned launches** on the device: the quick launch with Home's defaults came first.
- **A text count beside the tray icon**: the tray library has no label, so the count is drawn in the icon.
- **More notification modes** (all events, quota notices, a do-not-disturb switch in the menu): there is needs-you mode and `/dnd` for now.
- **Answering with option buttons** in the Spotlight, a notification when an agent starts, and a preview check before Launch.

## Not yet verified on a real desktop

The companion was built and tested on a machine without a desktop. These parts still need a check on a real Ubuntu 26.04 GNOME session, a real Space and real GitHub builds:

- **Package**: apt resolves its libraries, the app shows in the grid with its icon and dock actions.
- **Top bar**: the icon shows through Ubuntu AppIndicators and looks sharp at 100% and 200%; menu clicks and switches work and the menu updates live.
- **Notifications and dock**: banners show under the app, **Open** works, a new one replaces the old one of the same agent; the dock badge shows and hides.
- **Keyring**: storing and deleting secrets in GNOME Keyring, the unlock prompt, and the message without a keyring.
- **Shortcut**: GNOME's consent dialog shows once; Ctrl+Alt+Space opens the Spotlight with keyboard focus; **Change shortcut…**; the custom shortcut replaces the portal's and **Remove** brings it back.
- **Spotlight**: placement, light and dark look, hiding on focus loss, keys, 200% text, screen reader names; answers stream, typed answers reach a bot's question, approvals are refused.
- **Hand-offs**: the default browser (the Firefox snap) opens the right page; the composer shows the draft; the clipboard keeps the text.
- **Linking**: Sign in with Hugging Face with a real OAuth app; a real Space's gateway creates the device, lists and revokes it; a sleeping Space shows Home is starting.
- **Life cycle**: one instance at a time, start at login, suspend and resume while offline.
- **Updates**: the password dialog appears, PackageKit installs from `~/.cache`, the companion restarts in the new version; the `pkexec` path.
- **CI**: the `ubuntu:26.04` jobs, the package's dependencies, the uploads to releases and `main-build`, and the update check against them.

## Files

- Settings: `$XDG_CONFIG_HOME/launchpad-desktop/settings.toml` (usually `~/.config/launchpad-desktop/settings.toml`). It holds the linked Home, the workspace, the switches, the shortcut choice, the default Ask target, `update_channel` and an optional `hf_client_id`. It holds no secret.
- Update downloads: a fresh private folder in `$XDG_CACHE_HOME/launchpad-desktop/` per update, removed when the update fails.
- Logs: `$XDG_STATE_HOME/launchpad-desktop/companion.log` (usually `~/.local/state/launchpad-desktop/companion.log`), readable only by you and rotated at 1 MB. Tokens, secrets and codes are removed before a line is written. `LAUNCHPAD_DESKTOP_LOG=debug` shows more.

## Sources

- Hugging Face OAuth, loopback redirect URIs (any port for `http://127.0.0.1`), PKCE for public apps: https://huggingface.co/docs/hub/oauth
- Ubuntu session mode of Ubuntu 26.04, which enables `ubuntu-appindicators@ubuntu.com` and `ubuntu-dock@ubuntu.com`: https://git.launchpad.net/ubuntu/+source/gnome-shell/plain/debian/ubuntu-session-mods/ubuntu.json?h=ubuntu/resolute, installed by gnome-shell-common: https://packages.ubuntu.com/resolute/all/gnome-shell-common/filelist
- GNOME Shell 50.1 refuses notifications of an app without a desktop file (`InvalidAppError`): https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/js/ui/notificationDaemon.js
- GNOME's GlobalShortcuts portal backend, added in xdg-desktop-portal-gnome 48.rc: https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/50.0/NEWS
- Host app registry of xdg-desktop-portal 1.21.1 (register the app ID once, before any portal call; it must match a desktop file): https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/1.21.1/data/org.freedesktop.host.portal.Registry.xml
- GlobalShortcuts portal, version 2 in 1.21.1 (`BindShortcuts`, `ConfigureShortcuts`, `activation_token` in `Activated`): https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/1.21.1/data/org.freedesktop.portal.GlobalShortcuts.xml
- GNOME's backend binds shortcuts per session (`BindShortcuts` grabs the keys; a new session lists none): https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/50.0/src/globalshortcuts.c
- GNOME Settings shows the consent dialog only for new shortcuts of an app ID, and refuses an invalid app ID: https://gitlab.gnome.org/GNOME/gnome-control-center/-/raw/50.0/global-shortcuts-provider/cc-global-shortcut-dialog.c and https://gitlab.gnome.org/GNOME/gnome-control-center/-/raw/50.0/global-shortcuts-provider/cc-global-shortcuts-provider.c
- PackageKit 1.3.4 (the version in Ubuntu 26.04: https://packages.ubuntu.com/resolute/packagekit), `InstallFiles` and `SetHints(interactive=true)`: https://raw.githubusercontent.com/PackageKit/PackageKit/v1.3.4/src/org.freedesktop.PackageKit.Transaction.xml
- GitHub release assets carry a `digest`: https://docs.github.com/en/rest/releases/assets?apiVersion=2022-11-28
- LauncherEntry count on the dock (`com.canonical.Unity.LauncherEntry.Update`, read by Dash to Dock, the base of the Ubuntu Dock): https://raw.githubusercontent.com/micheleg/dash-to-dock/master/launcherAPI.js
