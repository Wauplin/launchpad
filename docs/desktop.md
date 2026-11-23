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

The companion checks `GET /api/version` before each stream. When the stream drops, it tries again after 1, 5, 15 and 30 seconds, then every minute. It never asks more often than every 5 seconds while Home does not answer. **Retry now** cuts the wait.

## Notifications

With **Notifications** on (the default), the companion shows a desktop notification when a Today item of its workspace asks a question (“<title> asks”), fails (“<title> failed”) or when a bot asks you in a channel. Each notification has **Open**, which opens the item in the browser.

- At most one notification per agent per minute. A new one replaces the earlier one of the same agent.
- The first load after a start only reads the list and notifies nothing.
- After a gap (offline, a dropped stream), new items become one summary: "While you were away: 2 asking, 1 failed".
- Connection problems notify once each: offline for over two minutes (then “Launchpad is back online”), Home starting for over two minutes, sign in again, device removed, Home too old.

`/dnd` in the Spotlight pauses them for one hour. An answer or a question of an ask you sent from the Spotlight also notifies you while the Spotlight is hidden. Notifications carry no token in their links. Home itself still sends no desktop notifications; Telegram is unchanged.

GNOME Shell shows an app's notifications only when the app's desktop file (`dev.launchpad.Desktop.desktop`) is installed. Without it, GNOME refuses them. The package installs it.

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

The portal and the custom shortcut need the installed desktop file `dev.launchpad.Desktop.desktop` and `launchpad-desktop` on your PATH. The package installs both.

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

### Not in this release

These parts of the design wait for a later release, so that the shortcut, Ask, Go to and the menu came first:

- Search in message text, and topics, routines, oracles and folio pages or sections in Go to.
- Pinned launches on the device.
- Answering a bot's question with option buttons in the Spotlight (type the answer, or open the browser).
- A “do not disturb” switch in the menu (use `/dnd`), and a notification when an agent starts.
- A preview check before Launch (Home still checks the launch).
- A chat draft on Home for asks (the hand-off uses the clipboard until then).

## Updates

Coming in this release: the companion checks for a newer release and offers **Update to <version>** in the menu, which installs a verified package after your confirmation.

## Files

- Settings: `$XDG_CONFIG_HOME/launchpad-desktop/settings.toml` (usually `~/.config/launchpad-desktop/settings.toml`). It holds the linked Home, the workspace, the switches and an optional `hf_client_id`. It holds no secret.
- Logs: `$XDG_STATE_HOME/launchpad-desktop/companion.log` (usually `~/.local/state/launchpad-desktop/companion.log`), readable only by you and rotated at 1 MB. Tokens, secrets and codes are removed before a line is written. `LAUNCHPAD_DESKTOP_LOG=debug` shows more.

## Sources

- Hugging Face OAuth, loopback redirect URIs (any port for `http://127.0.0.1`), PKCE for public apps: https://huggingface.co/docs/hub/oauth
- Ubuntu session mode of Ubuntu 26.04, which enables `ubuntu-appindicators@ubuntu.com` and `ubuntu-dock@ubuntu.com`: https://git.launchpad.net/ubuntu/+source/gnome-shell/plain/debian/ubuntu-session-mods/ubuntu.json?h=ubuntu/resolute, installed by gnome-shell-common: https://packages.ubuntu.com/resolute/all/gnome-shell-common/filelist
- GNOME Shell 50.1 refuses notifications of an app without a desktop file (`InvalidAppError`): https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/js/ui/notificationDaemon.js
- Host app registry of xdg-desktop-portal 1.21.1 (register the app ID once, before any portal call; it must match a desktop file): https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/1.21.1/data/org.freedesktop.host.portal.Registry.xml
- GlobalShortcuts portal, version 2 in 1.21.1 (`BindShortcuts`, `ConfigureShortcuts`, `activation_token` in `Activated`): https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/1.21.1/data/org.freedesktop.portal.GlobalShortcuts.xml
- GNOME's backend binds shortcuts per session (`BindShortcuts` grabs the keys; a new session lists none): https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/50.0/src/globalshortcuts.c
- GNOME Settings shows the consent dialog only for new shortcuts of an app ID, and refuses an invalid app ID: https://gitlab.gnome.org/GNOME/gnome-control-center/-/raw/50.0/global-shortcuts-provider/cc-global-shortcut-dialog.c and https://gitlab.gnome.org/GNOME/gnome-control-center/-/raw/50.0/global-shortcuts-provider/cc-global-shortcuts-provider.c
- LauncherEntry count on the dock (`com.canonical.Unity.LauncherEntry.Update`, read by Dash to Dock, the base of the Ubuntu Dock): https://raw.githubusercontent.com/micheleg/dash-to-dock/master/launcherAPI.js
