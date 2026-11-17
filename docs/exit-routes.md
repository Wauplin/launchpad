# Exit routes

An **exit route** sends some URLs out through one runner, the **exit runner**, instead of the machine that makes the request. Use it when only one of your machines can reach a service:

- **A GitHub organization behind an IP allow list.** GitHub refuses every address the organization did not allow. Home in a Space has a shared, changing cloud address, so GitHub refuses it with “has an IP allow list enabled”. Your laptop may reach the organization, for example through a company Tailscale app connector that routes GitHub's address ranges. A route through the laptop lets Home, Burst and your other runners use the organization too.
- **A service on your tailnet or LAN**, such as a Grafana at `https://grafana.example.ts.net/`. Home and Burst cannot resolve or reach it. A route through a runner on the tailnet reaches it for them.

An exit route is not a Tailscale exit node: it carries only the URLs of the route, and only through a Launchpad runner.

## Add a route

Open **Settings → Home and runners → Exit routes** and click **New route**. The form opens in place:

- **Reaches**: **GitHub organization** or **URL prefixes**.
- **Organization**: the organization's name, for example `acme-internal`. Or **URL prefixes**: one per line, at most 20. `https://host/` sends the whole host.
- **Through**: the exit runner, one of your machine runners (never Burst). A dot shows whether it is connected now.
- **Name**: named in errors and logs. Launchpad suggests the organization or the host. Use 1 to 40 lowercase letters, digits, `.`, `_` or `-`.

Each route row shows its name, an **online** or **offline** badge, what it reaches (“GitHub organization”, “All of grafana.example.ts.net”, or “3 URL prefixes on host”), and its exit runner. An offline route also says since when. The prefixes are in a tooltip. When the route does not work, the row shows why in amber: no runner has that name, or the exit's last failed connection (for example “runner laptop could not connect to grafana.example.ts.net:443 (route grafana): errno 111: Connection refused”). **Edit** changes a route. **Delete** is in **⋯**, with **Undo** in the toast. Typed values are kept as a draft when you leave the page.

Routes are yours: only your window changes them (action `egress.set`). Agents, even with admin access, cannot add or change a route. Routes are stored as `[[egress]]` entries in Home's `config.toml`:

```toml
[[egress]]
name = "acme-internal"
github_org = "acme-internal"
via = "laptop"                      # one runner: its name or ID

[[egress]]
name = "grafana"
urls = ["https://grafana.example.ts.net/"]
via = "laptop"
```

A route has either `github_org` or `urls`, never both. Home checks every write of the file and refuses an invalid route.

### From Clone from GitHub

When **Add folder → Clone from GitHub** fails because GitHub refuses Home's address, Home reads the repository from GitHub's API to find out why. When the answer names an IP allow list, the dialog shows an amber box: "<org> accepts only allow-listed addresses". Pick the runner under **Through** and click **Add route and clone**. Launchpad adds a GitHub organization route through that runner, then clones the repository again.

## The two kinds

### GitHub organization

A GitHub organization route covers these URL prefixes. Case does not matter in the organization's name.

| URL prefix | Mode | Used by |
| --- | --- | --- |
| `https://github.com/<org>/` | require | git clone, fetch and push, LFS batch API |
| `https://api.github.com/repos/<org>/` | require | REST: pull requests, issues, comments, reviews, contents, checks |
| `https://api.github.com/orgs/<org>/` | require | The organization's REST endpoints |
| `https://codeload.github.com/<org>/` | require | Tarballs and zip files |
| `https://raw.githubusercontent.com/<org>/` | require | Raw files |
| `https://uploads.github.com/repos/<org>/` | require | Release uploads |
| `https://api.github.com/graphql` | prefer | `gh pr …`, `gh issue …`, reply threads |
| `https://api.github.com/search/` | prefer | Searches |

- **require**: always through the exit. While the exit is offline, the request fails at once.
- **prefer**: through the exit while it is online, else direct. A GraphQL request does not name the organization in its URL (a mutation names only node IDs), so Launchpad cannot tell an organization's query from another one. While the exit is offline, public GraphQL and search keep working, and a query about the organization gets GitHub's own IP allow list refusal.

The exit runner opens only GitHub's hosts on port 443 for this kind: `github.com`, `api.github.com`, `codeload.github.com`, `raw.githubusercontent.com` and `uploads.github.com`.

### URL prefixes

Explicit `http://` or `https://` prefixes, all in mode require. A prefix has a host and an optional port and path, with no user name, query or fragment. Matching compares the host and the path in lowercase, on whole path segments (`https://host/api` matches `/api` and `/api/v1`, not `/apis`), and ignores the query. The exit opens only each prefix's host and port.

A prefix cannot name `localhost`, a loopback, link-local or cloud metadata address. Private and tailnet addresses are fine: reaching them is the point.

Prefer `https://`. A plain `http://` route works, but its traffic crosses Home and the exit in clear text, like any `http://`. Each `http://` connection carries one request.

When a URL matches several routes, the first require match in the order of the list wins, then the first prefer match.

## What goes through the exit

Every request leaves from the machine that sends it, as before, unless it matches a route:

| Sender | Covered requests |
| --- | --- |
| Home's git | Clone from GitHub, mirror fetches, Merge, Publish, Push to PR: git sends the route's URLs to the exit by itself |
| Home's GitHub client | Suggestions, Local PR publication and sync, ledger checks, reply resolution |
| Burst jobs | Requests that get a token, sent by Home's relay, and other connections to a whole-host route |
| Oracles | `tools.github` and `tools.http` calls that match a route |
| Agents on other runners | Requests to hosts where the runner proxy adds a token (it reads their URL), plain `http://` requests, and connections to a whole-host route |
| The exit runner itself | Nothing: its own requests go out directly, as before |

On a runner, the proxy reads the full URL only on hosts where it adds a token (it terminates TLS there) and for plain `http://` requests. For other HTTPS hosts it sees only the host and port, so only whole-host prefixes (`https://host/`) route those connections.

Agents that work on the organization's code need no route: worktrees come from Home Git, so they work on any runner, also while the exit sleeps. Only Home's own git talks to GitHub.

## When the exit is offline

A request of a require prefix fails at once, with one message:

> acme-internal is reached only through runner laptop, which is offline (route acme-internal).

Nothing waits and nothing is queued:

- **Clone, mirror fetch, Merge, Publish, Push to PR** fail with the message. Click again once the runner is back. A failed agent merge goes back to the agent, as usual, with the message.
- **Home's GitHub client** fails before it sends anything, so a Suggestion is never marked “delivery uncertain”.
- **An agent's `gh` or REST call** gets a 503 answer with the message as text, so `gh` prints it. A raw connection to a whole-host route fails as “network unreachable”.
- **Oracles** raise `OSError` with the message: the evaluation ends Failed, class `http`, never False.
- **prefer prefixes** go direct.
- Requests that match no route are not affected.

## Oracles

An oracle runs on Home, so its calls follow Home's routes:

- `tools.github.get("/repos/acme-internal/app/pulls")` goes through the exit. The call log says "… via laptop".
- `tools.http.get(url)` for a URL of a route skips Home's public-address rule: Home does not resolve the host, the exit resolves it (MagicDNS names work) and connects only to the route's targets. URLs outside routes keep the public-only rule.
- A failed or refused connection through the exit raises `OSError` with the exit's reason.

`tools.launchpad.egress.list()` returns the routes with their exit's state, like `launchpad egress list`. It reads the runner links Home already has and opens no connection. A routine that should run only while the organization is reachable can be gated on a small oracle:

```python
# /// oracle
# title = "Exit online"
# description = "Is the exit runner of an exit route connected?"
# tools = ["launchpad"]
# ///
from launchpad_oracle import failed, tools, verdict


def check(route: str = "acme-internal"):
    for r in tools.launchpad.egress.list():
        if r["name"] == route:
            state = "online" if r["online"] else "offline"
            return verdict(r["online"], summary=r["exit"] + " " + state, fingerprint=state)
    failed("no exit route named " + route)
```

Save it as `exit-online`, then gate the routine with `oracle = "exit-online"`, `run_on = ["true"]` and `fire = "every"` (see [Gates](oracles.md#gates)). While the laptop sleeps, each tick is a quiet check instead of a failed run. An oracle that reads the organization can also call `failed("exit offline")` first, so “no new pull requests” never hides “could not look”.

## CLI

`launchpad egress list` prints the routes as JSON, at every Launchpad CLI access level. Routes are shared by every workspace. Each entry has `name`, `kind` (`github_org` or `urls`), `github_org`, `urls`, `prefixes` (every URL prefix the route sends through its exit), `exit` (the runner's name), `exit_id`, `online`, `since` (when `online` last changed, in Unix milliseconds) and `last_error`. It holds no secret.

## Where it shows

- **Folder page**: a folder whose repository matches a route shows a fact in its header: "Through laptop · online", in amber when offline (on the phone, in the subtitle).
- **Access log**: in an agent's Summary → Run → Credentials, a request that went through an exit shows “via laptop”.
- **Logs**: the exit logs `exit route <name>: dial host:port`, and Home logs `exit route <name>: host:port through <exit>`.

## Security

- **Tokens never reach the exit.** TLS runs from the machine that adds the token (the runner proxy, or Home) to GitHub. The exit, and Home for other runners' traffic, only carry encrypted bytes and see the host and port.
- **No pivot.** The exit opens only the hosts and ports of its own routes, checked by Home and again by the exit. It never opens its own loopback (where Launchpad's daemons listen), link-local or cloud metadata addresses, even when a name resolves to them.
- **Routes are yours.** `egress.set` is an owner action: not an admin action, and not one of the shared settings an admin bot may change.
- **The exit gains nothing.** Its runner credential only answers Home's requests to connect.
- **Home's bridge is local.** Home's git and clients reach the exit through a bridge on `127.0.0.1`, protected by a secret that never appears in a process list.

## Limits

- **No databases or raw TCP services.** Routes carry URLs only. There is no route kind for a host and port, and no local port on runners.
- **SSH remotes** are not covered: Launchpad uses HTTPS remotes.
- **Tailnet names** resolve on the exit for routed HTTP requests and oracles. A tool that connects without a proxy resolves the name on its own machine, which fails outside the tailnet.
- **Signed asset URLs**: release assets and LFS objects download from `objects.githubusercontent.com`. If the allow list applies there too, add that host with a URL prefixes route.
- **Bandwidth and latency**: the first clone of a large repository goes through the exit's uplink. Requests from other runners and Burst cross Home and the exit, so expect a few hundred milliseconds more per round trip.
- **A compromised runner** that holds a GitHub token could send any request to GitHub's hosts through the exit, not only the organization's paths. That is the trust it already has with its tokens.
