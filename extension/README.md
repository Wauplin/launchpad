# Launchpad for Chrome

Start an agent on the tabs you choose, watch it work and approve what matters. The side panel is a small chat built for this, with its own components; it shares only Launchpad's logo and colors. Everything technical (trace, costs, model, follow-ups, schedules, presets) stays in Launchpad, one click away with **Open in Launchpad**. The daemon owns tasks and browser grants; the extension never calls a model API. The design review and sketches are in the [protected report](https://huggingface.co/spaces/Wauplin/launchpad-chrome-extension-redesign).

## Install

Use Google Chrome 116 or newer. From a source checkout, use Node 24 (`web/.nvmrc`) and pnpm 10.33.0:

```sh
cd web && pnpm install --frozen-lockfile
cd ../extension && pnpm install --frozen-lockfile && pnpm build
```

Open `chrome://extensions`, enable **Developer mode**, choose **Load unpacked**, and select `extension/dist`. Pin Launchpad to the toolbar. Keep Developer mode enabled. The manifest's public key pins the extension ID to `bbefehapjjlbeodihifmpgidgjhmcklj`; keep this key and the daemon's `browser::EXTENSION_ID` together.

For one-click pairing on this computer, build/install the new `launchpad` binary, then register its native host (Python 3, standard library only):

```sh
python3 extension/register-native-host.py --binary "$HOME/.local/bin/launchpad"
```

The script writes a launcher and a Google Chrome native-messaging manifest in your user configuration directory; on Windows it also registers the manifest under HKCU. It does not restart Launchpad. For a dev daemon, use `--environment dev --binary /absolute/path/to/target/debug/launchpad`; `--port` supports isolated checks. A custom Chrome `--user-data-dir` needs its own `NativeMessagingHosts` folder. Packaged releases include `extension/dist` and the registration script, so building the extension is unnecessary for those installs. Unregister by deleting `dev.launchpad.browser.json` and its launcher (and the Windows HKCU key).

Open the side panel and click **Connect to Launchpad on this computer**. Chrome becomes a normal, revocable device named “Chrome on this computer” in Settings → Devices. The native host exposes only local pairing, never the daemon bearer token. Chrome keeps the HttpOnly device cookie; no credential is written to extension storage.

For another computer, choose **Connect to another computer**, enter the daemon's HTTPS origin, and approve the eight-character code in Settings → Devices on a paired device. Keep the requesting panel open until approval. HTTP is accepted only on localhost. Reopening the panel reconnects automatically.

## Use

- **New chat** proposes the current tab as a chip. **+** opens a checklist of the window's tabs. Type the task and press Enter: sending is the only sharing step (Chrome may ask for the site's access during that click). Remove a chip with × to share no tab: the box then shows “The agent opens its own tabs” and the task gets a “New tab” handle (a grant without a page) from which the agent opens the sites it needs in new tabs of the task's group. Each new site shows a site review unless you always allow it. Tabs the agent opens are Work tabs, whatever the Read/Act choice, which applies to the tabs you share. Replying to a task with no tab left gives it a new handle. Suggestions (Summarize, Explain my selection, Compare my open tabs) only fill the box.
- **Agent** lists your Launchpad profiles, the last one used from the extension first, with "Subscription · N% of the limit used" or "Pay per use" (and one line when its costs can't be tracked). Model, effort and permission mode come from the profile and your latest launches; change them in Launchpad.
- **Read · Act**: Read only reads the shared tabs. Act also clicks, types and navigates in them; submitting, paying, deleting or going to a new site still shows a review card. The last mode used is remembered. **Always allow Act on <site>** (in the same menu, or **Always allow <site>** on a site review) remembers the site in the daemon: tasks sharing only such sites start in Act, and the agent can go to them without a site review. Consequential actions are always reviewed. Settings lists and removes these sites.
- **Project** (shown when Launchpad has projects; suggested when a tab is on the project's GitHub repository): the agent works in a new task worktree of the project with its normal tools plus the browser tools, and the task appears on that project page. Without a project, the task runs in a scratch folder with browser tools only. Page text can try to mislead an agent; share only pages you trust with a project task.
- Paste or drop screenshots into the box. **Ask Launchpad about selection** in Chrome's context menu adds the tab and the selected text as a quote; it never launches by itself.
- The task view shows the conversation with a compact, collapsible step list per turn. The send button becomes Stop while it works; you can type meanwhile. Replying continues the same task, also after Done, Stop or Archive; **+** shares more tabs with it, and the Read/Act pill switches the running task. Shared tabs show above the box with × to stop sharing, plus **Take control** / **Resume** while it acts.
- While the agent acts in a tab, the tab joins the cyan "Launchpad" group, gets a thin outline and a corner pill with **Take control** and **Stop** (trusted clicks only). Chrome's debugger bar also appears.
- Reviews, questions and permission requests appear as cards in the conversation; another task's review shows a banner at the top. **Tasks** groups browser tasks into Needs you, Running and Earlier. The ⋯ menu offers Open in Launchpad, Rename, Stop sharing all tabs, Archive and Settings (connection, text size, default agent, always-allowed sites, Disconnect). Text size (100–150%, default 120%) is stored per browser profile in the extension, independently of Launchpad's own zoom.

Browser tasks also appear in Launchpad with a "Browser" marker (or "<project> · Browser"). Follow up, presets, scheduling, Preview prompt and cost limits are available only in Launchpad. Browser tasks cannot continue in a terminal TUI (it would lose the browser tools); project tasks still offer a shell Terminal in their folder.

## Tools and limits

The common stdio MCP server (`launchpad browser-mcp`) exposes `browser_tabs`, `browser_read` (`article`, `selection`, `full`), `browser_outline`, and `browser_trace` for attached trace references. Work adds `browser_click`, `browser_type`, `browser_select`, `browser_press`, `browser_scroll`, `browser_navigate`, `browser_open_tab`, `browser_propose_form`, `browser_wait` and `browser_handover`. Actions take document-local refs from the latest outline and return an updated outline. Navigation/open-tab to a different origin requires an owner site review and Chrome site permission; unexpected cross-site redirects require adding the destination explicitly. Work grants follow same-origin document/SPA navigation; Read grants keep exact document/URL binding. Switching tabs never redirects an agent.

Calls contain opaque granted-tab handles; models never receive Chrome credentials. Text is labeled as untrusted data and is paginated (12,000 characters by default, at most 16,000 per read). Extraction is bounded to 200,000 characters, 10,000 visited nodes, 80 nesting levels and 600 outline entries. Iframes and closed shadow roots are omitted. This DOM reader is not Google Docs' internal document API; canvas-only editors may offer little readable text.

Grants bind the paired device, Chrome tab, isolated document nonce and exact URL, including same-document SPA navigation. For Read grants, navigation requires adding the page again. Tab closure, browser restart or extension reload requires fresh page access. Grants persist in the daemon, expire after 30 minutes without a successful read, and are revoked on Stop/Archive or chip removal. Restored drafts can contain expired chips; remove them and add the current page. Device revocation disconnects both the panel and worker. The Chrome site permission may remain installed; it does not grant a session access on its own.

Password and one-time-code fields are excluded from text/outline extraction; input values are not included in article text. The outline includes ordinary visible field values. Arbitrary page text may still contain sensitive information: add only pages you intend the selected profile to read.

Browser-only tasks: Claude disables built-in tools and other MCP servers. Codex uses a read-only sandbox and disables other configured MCP servers/plugins/apps, but native shell tools can still read local files. OpenCode uses its native permission rules to deny other tools and disables configured MCP servers; it has no OS sandbox. Project tasks keep the harness's normal tools, MCP servers and the saved permission mode, and add `launchpad_browser` next to them. Browser grants remain independent of YOLO/approval modes. Work uses a fixed CDP input command set through `chrome.debugger`, never arbitrary evaluation, console or network tools. Screenshots (including model image-capability discovery), developer access, iframe/shadow-root interaction and E3 coding-session browser access remain outside this action milestone.

The worker uses the existing event socket with replay, 25-second pings, reconnection and an alarm. Chrome omits SameSite cookies on extension WebSocket upgrades, so the extension first requests a 30-second, single-use upgrade ticket using its authenticated cookie. Tickets work only on `/ws` from the pinned extension origin and inherit device revocation. Ordinary browser cookies and origin checks stay unchanged.

See [E0 findings and verification](../docs/spikes/chrome-extension-e0.md). Daily use on real signed-in sites, native messaging on macOS/Windows/Chromium snap, and the under-three-second warm response target still need validation.

## Uncertain actions

The daemon writes an action claim before dispatch. The worker consumes the request ID before input, and replayed events must still belong to a live, device-bound broker call. Errors, crashes, interrupted calls or a lost response retain the claim and block further actions, including after daemon restart. The panel asks “Did the last action go through?”. Check the page yourself (**Show the tab**), then click **I checked — continue** and tell the agent what you saw. Continue never retries the previous input. Same-origin inspection can follow navigation; a new site needs an explicit page grant. Never repeat Submit merely because its reply was lost. Pause/Stop, device disconnection, quota pauses and turn end detach the debugger; already dispatched input may have reached the site.

See [E2 implementation and verification](../docs/spikes/chrome-extension-e2.md).
