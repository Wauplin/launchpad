# Launchpad for Chrome — E1

Read pages you explicitly share with a Launchpad agent, using your existing profiles. The side panel bundles the same composer, launch options, Summary, Trace, quota/cost controls and follow-up actions as the app. The daemon owns sessions and browser grants; the extension never calls a model API.

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

- **Summarize page** prepares a draft and adds that page. **+ Add this page** explicitly grants a page to the open draft or browser session. Chrome may ask for access to that site. Switching tabs never redirects a grant.
- Select text and use **Ask Launchpad about selection** in Chrome's context menu. Choose a profile/model in Options, edit the task and click **Launch**. The menu prepares a draft; it never starts a paid turn automatically.
- Review the page chips beside the conversation. Remove access with ×. Browser sessions also appear in Launchpad with a **Browser · read only** marker.
- **Use trace as context** and **Follow up** prepare another conversation. They pin the source trace through a stored event and start with no page grants. Add the page again before launching. Follow-up reuses the browser scratch folder; the source history remains separate.

Each browser session has a scratch folder under the daemon data directory, no project/worktree, and no terminal handoff. Drafts are saved separately per extension device, without overwriting the web app's coding drafts. Scheduling browser launches is unavailable; a page grant requires an explicit launch.

## Read tools and limits

The common stdio MCP server (`launchpad browser-mcp`) exposes `browser_tabs`, `browser_read` (`article`, `selection`, `full`), `browser_outline`, and `browser_trace` for attached trace references. Calls contain opaque granted-tab handles; models never receive Chrome credentials. Text is labeled as untrusted data and is paginated (12,000 characters by default, at most 16,000 per read). Extraction is bounded to 200,000 characters, 10,000 visited nodes, 80 nesting levels and 600 outline entries. Iframes and closed shadow roots are omitted. This DOM reader is not Google Docs' internal document API; canvas-only editors may offer little readable text.

Grants bind the paired device, Chrome tab, isolated document nonce and exact URL, including same-document SPA navigation. Navigation, tab closure, browser restart or extension reload requires adding the page again. Grants persist in the daemon, expire after 30 minutes without a successful read, and are revoked on Stop/Archive or chip removal. Restored drafts can contain expired chips; remove them and add the current page. Device revocation disconnects both the panel and worker. The Chrome site permission may remain installed; it does not grant a session access on its own.

Password and one-time-code fields are excluded from text/outline extraction; input values are not included in article text. The outline includes ordinary visible field values. Arbitrary page text may still contain sensitive information: add only pages you intend the selected profile to read.

Claude disables built-in tools and other MCP servers. Codex uses a read-only sandbox and disables other configured MCP servers/plugins/apps, but native shell tools can still read local files. OpenCode uses its native permission rules to deny other tools and disables configured MCP servers; it has no OS sandbox. The selected harness's limitation is shown in the composer. Browser grants remain independent of YOLO/approval modes. No click, type, navigation, form submission, debugger, screenshot, computer-use or coding-session browser tools are implemented in E1.

The worker uses the existing event socket with replay, 25-second pings, reconnection and an alarm. Chrome omits SameSite cookies on extension WebSocket upgrades, so the extension first requests a 30-second, single-use upgrade ticket using its authenticated cookie. Tickets work only on `/ws` from the pinned extension origin and inherit device revocation. Ordinary browser cookies and origin checks stay unchanged.

See [E0 findings and verification](../docs/spikes/chrome-extension-e0.md). Daily use on real signed-in sites, native messaging on macOS/Windows/Chromium snap, and the under-three-second warm response target still need validation.
