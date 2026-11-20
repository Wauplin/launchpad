# Oracles

An oracle is a small Python check that answers one question with **True**, **False** or **Failed**, for example “is CI red on main?”, “was a new issue opened?” or “has an agent waited for me for more than two hours?”. It is one file of a workspace, with no model and no session. Home runs every oracle in [Monty](https://github.com/pydantic/monty), a Python interpreter for untrusted code, and the oracle reaches the outside only through four read-only tools: `http`, `github`, `huggingface` and `launchpad`.

Routines use oracles as gates: a gated routine starts its bot or agent only when the oracle's answer calls for it, and stays quiet the rest of the time. Agents and you can also run an oracle directly instead of polling an API.

Oracles belong to a workspace. They live on the **Schedules** page, in the **Oracles** tab.

## A first oracle

This oracle answers “is the latest completed run of a workflow on this branch a failure?”:

```python
# /// oracle
# title = "CI red on a branch"
# description = "Is the latest completed run of a workflow on this branch a failure?"
# tools = ["github"]
# min_interval = "2m"
# ///
from launchpad_oracle import tools, verdict


def check(repo: str, branch: str = "main", workflow: str = "tests.yml"):
    runs = (
        tools.github.get(
            f"/repos/{repo}/actions/workflows/{workflow}/runs",
            params={"branch": branch, "status": "completed", "per_page": 1},
        )
        .raise_for_status()
        .json["workflow_runs"]
    )
    if not runs:
        return verdict(False, summary=f"no completed run of {workflow} on {branch} yet")
    last = runs[0]
    return verdict(
        last["conclusion"] == "failure",
        summary=f"{workflow} on {branch} @ {last['head_sha'][:7]}: {last['conclusion']}",
        fingerprint=last["head_sha"],
        links=[last["html_url"]],
    )
```

- The header, between `# /// oracle` and `# ///`, holds the title, the question, the tools the code uses and how long an answer stays fresh.
- One import: everything an oracle needs comes from `launchpad_oracle`.
- `def check(...)`: Home calls it once per evaluation. Its arguments are the oracle's parameters: `repo` is required, and `branch` defaults to `"main"`.
- `tools.github.get(endpoint, params=…)` is a GET on GitHub's REST API with your GitHub token. `.raise_for_status()` turns a 4xx or 5xx into Failed, and `.json` is the parsed body.
- `verdict(...)` answers True or False with evidence: a summary, a fingerprint that names what is true, and links.

## The file

An oracle is the file `oracles/<slug>.py` of its workspace. The slug uses lowercase letters, digits, `-` and `_` (at most 63 characters). Home stores the file like a plain folder, with **Versions**, diffs and **Restore**. A file is at most 256 KB.

### The header

The file starts with a block in [PEP 723](https://peps.python.org/pep-0723/) syntax, of type `oracle`. Each line starts with `#`, and the content is TOML:

```python
# /// oracle
# title = "New issues"
# description = "Was an issue opened in the repository since the last check?"
# tools = ["github"]
# min_interval = "5m"
# timeout = "10s"
# ///
```

| Key | Required | Meaning |
| --- | --- | --- |
| `title` | yes | The name shown everywhere. |
| `description` | no | The question the oracle answers, as one sentence. |
| `tools` | no | The tools the code calls: any of `"http"`, `"github"`, `"huggingface"`, `"launchpad"`. Empty by default: an oracle with no tools can only compute from its arguments. |
| `min_interval` | no | How long an answer stays fresh: at least `1m`, and `1m` by default. Callers within that time get the same answer instead of a new evaluation. |
| `timeout` | no | The wall-clock budget of an evaluation, tool calls included: `10s` by default, at most `60s`. |

Durations are written `500ms`, `10s`, `2m`, `1h`, `1d`, or as a number of seconds. Unknown keys are refused, so a typo shows at once. There is no `params`, `dependencies`, `requires-python` or `hosts` key: parameters are the arguments of `check`, and the language, the standard library and the tools are the same for every oracle.

### The pre-check

Before an oracle is saved or run, Home checks it and formats it. This catches mistakes such as a misspelled name or a missing import before they become a Failed evaluation hours later. The pre-check runs on every save (your **Save**, `launchpad oracle save`, an applied card), on every run of a draft (**Test**, `launchpad oracle test`) and when an oracle card is filed:

1. **Header**: the oracle block parses, with known keys and tools only.
2. **Lint and fix**: `ruff check --fix` with Launchpad's fixed rule set (below), safe fixes only.
3. **Format**: `ruff format` with a line length of 112.
4. **Signature**: `check` exists and its arguments follow the rules of [Arguments](#arguments).
5. **Parse**: Monty parses the result. This catches what Monty does not support, such as a class with a base class or a `yield`.
6. **Type check**: the file is type-checked against the interface of `launchpad_oracle`. Type errors are warnings, shown with their line; they never block a save or a run.

When steps 1 to 5 pass, Home keeps the formatted file: that is what it saves, versions, diffs and runs. You see the formatted text the next time you open the oracle.

When a step finds an error it cannot fix, nothing is saved or run. The oracle page, the CLI (exit code 3) or the card shows each error with its line, column, rule and message, for example `oracle.py:14:9: F821 Undefined name `tool``.

| Rules | Catches | Fixed automatically |
| --- | --- | --- |
| `E9` | Syntax errors and source that cannot be read | No: refused |
| `F` (Pyflakes) | Undefined names, redefinitions, unused imports and variables, f-strings without placeholders, duplicate dict keys… | Unused imports, unused variables, f-strings without placeholders and the other safe fixes; the rest is refused |
| `I` (isort) | Unsorted imports | Yes |

The rule set is fixed: Ruff reads no configuration file, and `# noqa` comments work as usual. It holds only rules whose fixes Monty can run. Ruff runs on Home, as the binary of Home's image.

## Writing check

Home runs the file, then calls `check` once, passing every argument by name. There is no `run(check)` line, no `main` and no context object: defining `check` is enough.

### Arguments

The arguments of `check` are the oracle's parameters. Their annotations are their types and their defaults are their defaults. A parameter without a default is required.

```python
def check(repo: str, branch: str = "main", limit: int = 20, drafts: bool = False, hours: float = 2, label: str | None = None):
```

| Annotation | Callers pass |
| --- | --- |
| `str` | Any text |
| `int` | A whole number |
| `float` | Any number (an `int` default is fine) |
| `bool` | True or false |
| `T \| None`, with `T` one of the above | A value, or nothing |

Every argument has an annotation. Defaults are literals: a string, a number, `True`, `False` or `None`. `*args`, `**kwargs`, positional-only parameters (`/`) and `async def check` are refused. Home reads the signature at the pre-check, shows it as the parameter fields of **Test** and of the gate editor, and checks every value before an evaluation starts: a missing, unknown or mistyped value fails the call with a clear error and never reaches the code. `launchpad oracle run --param limit=50` and a gate's `params = { limit = 50 }` name arguments the same way.

Two names are reserved. No caller may pass them; Home gives them to `check` only when its signature asks for them, in any position and without a default:

| Argument | Annotation | Value |
| --- | --- | --- |
| `cursor` | `str \| None` | This caller's cursor (see [Fingerprints and cursors](#fingerprints-and-cursors)). `None` on a caller's first evaluation. |
| `now` | `datetime` | The start of the evaluation, in UTC. Monty's clock is frozen at this moment, so `datetime.now(timezone.utc)` returns the same value. Import `datetime` to use the annotation. |

What started the evaluation (a gate, the CLI, a test) and the workspace ID are not given to the code: the answer depends only on the arguments and the tools, so a test answers like a live run.

### Answering

- `return verdict(value, *, summary="", fingerprint=None, links=None, items=None, cursor=…)` answers True or False with its evidence. `summary` is one line for people. `fingerprint` names what is true (a commit, a version, a set of IDs), so a gate can fire again when it changes. `links` are URLs, and `items` any JSON-like values a routine run receives. `cursor` proposes the caller's next cursor; leave it out to keep the current one.
- `return True` or `return False` is short for a verdict without evidence.
- `failed(reason)` ends the evaluation with Failed (class `declared`). Use it whenever the oracle cannot decide: a partial page, an unexpected answer, a missing repository. Never return False for “I don't know”.

Any other return value fails with class `contract`. Summary, fingerprint, links and items share a 32 KB budget, and the cursor is at most 4 KB.

### Logs

`print()` writes to the evaluation's log (at most 64 KB), shown in **Test** and **History** next to the call log. It never changes the answer.

### The language

Oracles are written in the Python that Monty runs: a subset of Python 3.14 with part of the standard library.

- **Works**: functions and lambdas, comprehensions, `match`, `:=`, `try`/`except`, f-strings, unpacking, simple classes and dataclasses, and `sorted` or `max` with `key=`. The modules `json`, `re`, `datetime`, `hashlib`, `base64`, `math` and `random` work; `random` is seeded per evaluation.
- **Does not work**: class inheritance (so no custom exception classes), method decorators such as `@property`, `__getattr__`, `yield`, `asyncio`, any third-party package, `urllib`, `zoneinfo`, `socket`, `subprocess`, `os.environ`, `open()`, threads and `time.sleep` (an oracle checks once and never waits).

To build a query string, pass `params=` to `.get`: Home encodes it.

## Tools

An oracle imports `tools` from `launchpad_oracle`. A tool that is not in the header's `tools` list raises `PermissionError` on its first call, with a message that names the line to add. Every call goes to Home, which checks it, adds the credential, runs it and logs it. Oracle code never holds a token.

| In the oracle | Reaches | Credential |
| --- | --- | --- |
| `tools.http.get(url)` | Any public http(s) address, and the URLs of [exit routes](exit-routes.md) | None |
| `tools.github.get("/repos/…")` | `https://api.github.com` + the endpoint | The registry tool `gh`, added by Home |
| `tools.huggingface.get("/api/…")` | `https://huggingface.co` + the endpoint | The registry tool `hf`, added by Home |
| `tools.launchpad.sessions.list()`, `tools.launchpad.today()`… | Home's read API for the oracle's own workspace, answered inside Home | None needed |
| `tools.launchpad.get("/api/…")` | Any GET route of the read level | None needed |

The whole interface is a short Python stub that Home also gives to the type checker. `launchpad oracle show` names where to read it (`/api/oracles/<workspace>/_interface`).

### Response

Every `.get` returns a `Response`:

| Field | Rule |
| --- | --- |
| `status_code` | The final HTTP status, after redirects. |
| `text` | The body decoded as strict UTF-8, whatever the Content-Type says, without a leading byte-order mark. `None` when the bytes are not valid UTF-8 (an image, a binary file). An empty body is `""`. |
| `json` | The parsed value of `text`, whatever the Content-Type says. `None` when `text` is `None` or empty, or does not parse as JSON. A body that is literally `null` also gives `None`. |
| `raise_for_status()` | For a status of 400 or more, ends the evaluation with Failed, class `http`, message `HTTP 404`; the call log shows which call it was. Otherwise returns the response itself, so calls chain: `tools.github.get(…).raise_for_status().json`. |

Decoding never raises: check `r.json is None` or `r.text is None` instead of catching errors. A 4xx or 5xx is a normal response until you call `raise_for_status()`, so an oracle can treat a 404 as an answer. Response headers are not exposed. To paginate, use the API's own page or cursor parameters and stop at a short or empty page.

### tools.http

`tools.http.get(url, params=None, headers=None) -> Response`

A GET to any public address, never with a credential: public APIs and pages such as PyPI, npm, release feeds and status pages.

- `url` is absolute, `http://` or `https://`, any port, without a user name or password. `params` are added to its query string.
- `headers` may set any request header except `Authorization`, `Proxy-Authorization`, `Cookie`, `Host` and the connection headers (`ValueError`). Credentials come only from tools: a secret written in an oracle file would be readable by every agent of the workspace.
- **Public addresses only.** Home resolves the host and refuses it with `PermissionError` when an address is not public: loopback, private, link-local, carrier-grade NAT, multicast, unspecified, documentation ranges, cloud metadata and their IPv4-mapped forms. It connects only to the address it checked. Home's own address is refused too.
- **Exit routes.** A URL of an [exit route](exit-routes.md) goes through the route's exit runner, which resolves the host and connects only to the route's targets, so tailnet and private addresses of a route work. Home does not apply its public-address rule to it. The call log shows “via <exit>”. While the exit is offline, the call raises `OSError` with the route's message.
- **Registry hosts are trusted by name.** The hosts of the tools of Home's registry (`api.github.com`, `huggingface.co`…) are reached whatever they resolve to, still without a credential.
- Redirects are followed up to 5 times, and each hop is checked again. No cookies are kept. Home sends `User-Agent: Launchpad-Oracle/<version>` unless the oracle sets its own.

### tools.github

`tools.github.get(endpoint, params=None, headers=None) -> Response`

A GET on GitHub's REST API, `https://api.github.com` + `endpoint`, with the token of the registry tool `gh`. Write the endpoint as in GitHub's REST documentation.

- `endpoint` starts with `/` and may hold a query string, which `params` extends. A scheme, a host, `//`, a `.` or `..` segment, a fragment, a backslash or a control character is refused (`ValueError`), so the token only goes to `api.github.com`.
- Home sends `Authorization: Bearer <token>`, `Accept: application/vnd.github+json` and `X-GitHub-Api-Version: 2022-11-28`. Set `Accept` yourself for another format, such as `application/vnd.github.diff` for a pull request's diff as text.
- A redirect to another host (a log or archive download) is followed without the token, under the rules of `tools.http`.
- A repository of an organization with an [exit route](exit-routes.md) is read through the route's exit runner, with the token added on Home. The call log shows “via <exit>”; while the exit is offline, the call raises `OSError`.
- Only GET is possible, so an oracle cannot write to GitHub, whatever the token's scopes. GraphQL, which needs POST, is not available.

### tools.huggingface

`tools.huggingface.get(endpoint, params=None, headers=None) -> Response`

A GET on the Hugging Face Hub, `https://huggingface.co` + `endpoint`, with the token of the registry tool `hf`: Hub API endpoints such as `/api/models/{repo}` or `/api/whoami-v2`, and file URLs such as `/{repo}/resolve/main/README.md`. The endpoint rules, redirects and the GET-only rule are those of `tools.github`. The Hub answers 401 or 404 for a repository the token cannot see: use `status_code` to tell “not found” from an error.

### tools.launchpad

Launchpad commands mirror the CLI's read commands, for the oracle's own workspace: `launchpad sessions list` becomes `tools.launchpad.sessions.list()`. Home answers them inside its own process, through the same routes and checks as the CLI at the read level: no token is made and nothing leaves Home.

| Python | CLI |
| --- | --- |
| `folders.list()`, `bots.list()`, `sessions.list()`, `schedules.list()`, `proposals.list()`, `activity.list()`, `profiles.list()`, `presets.list()` | `launchpad <group> list` |
| `sessions.show(session_id)` | `launchpad sessions show <id>` |
| `today()` | `launchpad today` |
| `topics.list()`, `topics.show(topic)` | `launchpad topics list\|show` |
| `oracle.list()`, `oracle.show(slug)`, `oracle.history(slug, failed=False, limit=20)` | `launchpad oracle list\|show\|history` |
| `notes(repo, number)` | `launchpad notes --repo --number` |
| `egress.list()` | `launchpad egress list`: the [exit routes](exit-routes.md) with whether their exit runner is online (shared by every workspace) |

Each returns the same JSON as the CLI, as Python values. Session times such as `status_since` are Unix milliseconds. An error, such as an unknown session, raises `RuntimeError` with Launchpad's message.

`tools.launchpad.get(path, params=None) -> Response` reads any other GET route of the read level, such as a session's trace; `path` starts with `/api/`. Routes outside the read level, routes tied to a session and running oracles are refused with `PermissionError`. Lists are narrowed to the oracle's workspace.

### Errors

Monty has no class inheritance, so the tools raise built-in exceptions. Home remembers each error it raises into the code, so an uncaught one fails the evaluation with the right class.

| Raised | When | Uncaught: Failed with class |
| --- | --- | --- |
| `PermissionError` | The tool is not in the header, not granted to the workspace or has no token; a private address or Home itself; a Launchpad route outside the read level | `access` |
| `ValueError` | A bad argument: an endpoint that does not start with `/`, a refused header, a limit out of range | `script` |
| `OSError` | A DNS, TCP or TLS failure, a reset connection, more than 5 redirects, a redirect to an address tools do not follow, an exit route whose exit runner is offline or could not connect | `http` |
| `RuntimeError` | A Launchpad command answered an error | `http` |

An oracle may catch these to decide what an error means, for example an `OSError` on an optional source. `raise_for_status()` and `failed()` cannot be caught. Running out of time during a tool call and running out of a budget (see [Limits](#limits)) are not exceptions either: they end the evaluation at once, with class `timeout` or `limit`.

## Outcomes

An evaluation ends with one of three outcomes:

- **True** or **False**: the oracle's answer, with its summary, fingerprint, links, items and proposed cursor.
- **Failed**: the oracle could not answer. Failed is never read as False. Its class says why:

| Class | Cause |
| --- | --- |
| `access` | A tool missing from the header, not granted to the workspace, without a token, or refused (a private address, a route outside the read level). |
| `http` | `raise_for_status()` on a 4xx or 5xx, a connection error, or a Launchpad command error. |
| `timeout` | The evaluation ran past its timeout, tool calls included. |
| `limit` | A budget ran out: memory, tool calls, response size or bytes received; or the code called `time.sleep`. |
| `script` | An exception in the code, with its line. Syntax errors, undefined names and unsupported constructs are refused by the pre-check before anything runs. |
| `contract` | `check` returned something other than a verdict or a bool, or the evidence or cursor is too large. |
| `declared` | The oracle called `failed(reason)`. |

Each evaluation is kept for 30 days (at most 2,000 per oracle), with its callers, what each caller did with the answer, its log and its call log.

## Fingerprints and cursors

A **fingerprint** names what is true: a commit SHA, a version, a sorted list of IDs. A gate that fires “when it changes” fires again when True comes back with a new fingerprint, and stays quiet while it is the same. Build fingerprints from stable identifiers, never from times or counts that move on every evaluation.

A **cursor** lets an oracle answer “what is new since last time?”. Each consumer has its own: a gated routine has one, and each name given to `launchpad oracle run --cursor` has one per oracle. The oracle reads it through a `cursor: str | None` argument and proposes the next one with `verdict(…, cursor=…)`.

- The cursor moves only when the consumer acts on the answer. A gated routine commits it in the same step that claims its run, so items are never lost when a run fails to start. A False that starts no run commits at once.
- Failed never moves a cursor, and a test never commits one.
- On a caller's first evaluation `cursor` is `None`. A good first answer is False with a cursor at “now”, so a new gate does not fire on old items.

This oracle uses issue numbers as its cursor, so each routine run sees only what is new, and it does not fire on its first evaluation:

```python
# /// oracle
# title = "New issues"
# description = "Was an issue opened in the repository since the last check?"
# tools = ["github"]
# min_interval = "5m"
# ///
from launchpad_oracle import tools, verdict


def check(cursor: str | None, repo: str, limit: int = 20):
    params = {"state": "open", "sort": "created", "direction": "desc", "per_page": limit}
    found = tools.github.get(f"/repos/{repo}/issues", params=params).raise_for_status().json
    issues = [i for i in found if "pull_request" not in i]
    since = int(cursor or "0")
    newest = max([i["number"] for i in issues] + [since])
    new = [i for i in issues if i["number"] > since]
    if cursor is None or not new:
        return verdict(False, summary="nothing new", cursor=str(newest))
    return verdict(
        True,
        summary=f"{len(new)} new issue(s)",
        items=[{"number": i["number"], "title": i["title"], "link": i["html_url"]} for i in new],
        fingerprint=str(newest),
        cursor=str(newest),
    )
```

This one treats a 404 as a Failed with a clear reason, and sets its cursor without firing on its first evaluation:

```python
# /// oracle
# title = "Model repository updated"
# description = "Has the model repository received a new commit since the last check?"
# tools = ["huggingface"]
# min_interval = "5m"
# ///
from launchpad_oracle import failed, tools, verdict


def check(cursor: str | None, model: str, revision: str = "main"):
    r = tools.huggingface.get(f"/api/models/{model}/revision/{revision}")
    if r.status_code == 404:
        failed(f"{model}@{revision} not found (private, gated or renamed?)")
    sha = r.raise_for_status().json["sha"]
    if cursor is None:
        return verdict(False, summary=f"first check: {model} is at {sha[:7]}", cursor=sha)
    return verdict(
        sha != cursor,
        summary=f"{model}@{revision}: {cursor[:7]} -> {sha[:7]}",
        fingerprint=sha,
        links=[f"https://huggingface.co/{model}/commits/{revision}"],
        cursor=sha,
    )
```

## Broken oracles

A stored file that no longer passes the pre-check stays stored and is **broken**: a file in the older format (`# /// script` with `[tool.launchpad]`, `ctx`, `run(check)`), or a version brought back with **Restore**. A broken oracle shows the reason, with its line, on its row and its page. It is never evaluated: **Test now**, `launchpad oracle run` and gates refuse it. Each tick of a routine gated on it is a quiet check (“oracle broken: …”), and Today shows one row for it until you save a version that passes. To fix it, rewrite the file in the current format and save it.

## How Home runs an oracle

Gate ticks, `launchpad oracle run`, your **Test**, **Test now** in Today and agents' `launchpad oracle test` all follow one path on Home:

1. A draft is pre-checked first. A saved oracle was checked when it was saved and runs as stored.
2. Home checks the parameters and the tools. For a saved oracle, it reuses an answer younger than `min_interval`, or joins an evaluation already running with the same file, parameters and cursor.
3. A fresh Monty worker process runs the file, then calls `check`. Each tool call pauses the worker while Home checks, runs and logs the call.
4. Home maps the result to True, False or Failed with its class, and checks the sizes of the evidence and the cursor.
5. A saved oracle's evaluation is recorded with its log and call log, then the caller may commit its cursor. A draft returns the same result and records nothing.

So a test and a live evaluation share the pre-check, the interpreter, the tools, the credentials, the limits and the clock. An agent's draft runs on Home, never in the agent's jail or through its runner.

Within `min_interval`, callers get the last answer again, including a Failed one, so a failing API is not called in a loop. `--max-age` on the CLI accepts an older answer. Drafts never share or reuse answers.

### Limits

| Limit | Value | When reached |
| --- | --- | --- |
| Wall clock, tool calls included | `timeout`: 10 s by default, at most 60 s | Failed, `timeout` |
| Tool calls per evaluation | 200 | Failed, `limit` |
| Response body per call | 10 MiB | Failed, `limit` |
| Bytes received per evaluation | 50 MiB | Failed, `limit` |
| Memory | 128 MiB | Failed, `limit` |
| Redirects per call | 5 | `OSError` |
| Evidence; cursor; log | 32 KB; 4 KB; 64 KB | Failed, `contract`; the log is cut |
| Evaluations at once | 16 | More wait for a free slot |

### Call log

Home records every tool call of an evaluation: the tool, the call (`GET /repos/acme/app/actions/workflows/tests.yml/runs?branch=main&per_page=1`, `sessions.list()`), the status or error, the bytes received and the duration. Request headers are never recorded. The call log is kept with the evaluation, shown in **Test** and **History**, and printed by `launchpad oracle test --json` and `launchpad oracle history --calls`. A `raise_for_status()` failure points at its call.

## Security

Oracle code is treated as untrusted, even when you saved it: an agent may have written it, and it handles text from GitHub and the web. Monty is the sandbox: oracle code has no filesystem, environment, network, subprocesses or packages, and each evaluation runs in its own worker process with an empty environment. Every tool is implemented and checked in Home's own code, never in the oracle:

- Every HTTP call is a GET, and the Launchpad commands are a fixed list of read commands: the oracle chooses only argument values, never a route outside the read level.
- Tokens never enter the worker: not in its environment, a response or an error message. Every secret an evaluation used is also removed from its result before Home records it.
- A token goes only to its own tool's host, also through redirects. `tools.http` refuses private addresses and Home itself, so an oracle cannot reach Home's gateway or the machine's network.
- Memory, time, call and byte budgets stop runaway code.

What the tools do not protect against: an oracle reads what its tokens read, so granting a tool is the decision. An oracle with a credentialed tool and `http` could put what it read into a GET to any public address; the Tools tab and oracle cards flag that combination. Evidence reaches routine runs fenced and marked as data from outside Launchpad, but it is still text an agent reads.

## Who may do what

The credentialed tools use tools of Home's registry (Settings → Credentials and tools). Each must be granted to the oracle's workspace and have a token; otherwise the evaluation fails with class `access` and a message that names the tool. Saving warns about a missing grant or token.

| Oracle tool | Registry tool |
| --- | --- |
| `github` | `gh` |
| `huggingface` | `hf` |
| `launchpad` | None: the oracle's own workspace, at the read level |
| `http` | None |

| Who | What |
| --- | --- |
| You | Everything: create, edit, test any draft with any tools, pause, mark read, restore and delete. Delete asks again when routines are gated on the oracle. |
| Admin bots | Save directly (`launchpad oracle save`) when the bot's own session holds every credentialed tool of the file (`github`, `huggingface`); `http` and `launchpad` need nothing. Pause and resume. A file with any other tool must be a card for you. The bot's chat shows each change as a receipt. |
| Other agents | List, show, read the history and run saved oracles. Test drafts on Home (`launchpad oracle test`) whose credentialed tools their own session holds. Propose an oracle with a card. |

A saved oracle runs with the tools you approved, whoever calls it: an agent that runs it gets the answer, not the tools. A draft never inherits tools from the saved oracle of the same slug.

An **oracle card** (`launchpad propose oracle`) holds the slug and the whole file. The file is pre-checked when the card is filed (a refused file cannot be filed) and stored formatted. The card shows the diff, the tools it adds (amber) or removes, type warnings, a warning when it combines a credentialed tool with `http`, and **Apply**, which saves it. An admin session may accept a card on the same terms as its own save. Oracle cards never start on their own, even for a bot in auto mode.

## Gates

A routine with a schedule trigger can have a gate: before each scheduled run, Home evaluates an oracle, and the answer decides whether the run starts. Gates work on bot routines and on other routines. **Run now** ignores the gate.

In the routine editor (**Edit** on Schedules, or the bot's settings → **Triggers**), a schedule trigger has **Only when an oracle says**: pick an oracle of the workspace (or **Always**, for no gate), fill in its parameters, then choose:

- **Outcomes**: which answers start a run. **True** and **False** start a run with the routine's instructions and the evidence. **Failed** starts a repair run, once per failure streak, with its own instructions. An unchecked outcome stays silent.
- **Fire** (required, no default): **When it changes** starts a run when the outcome changes, or when True comes back with a new fingerprint. **Every time** starts a run for every routed answer.
- **Cooldown** (optional, for example `6h`): the same answer with the same fingerprint stays quiet that long after a run.
- **Remind** (optional, for example `24h`): with **When it changes**, the same answer starts a run again once that long has passed.

A bot routine file writes the gate as a `[gate]` table:

```toml
title = "Triage new issues"
schedule = "*/5 8-19 * * 1-5"
instructions = "New issues were opened. Read the issues in the evidence, label them and draft answers for me to review."

[trigger]
kind = "schedule"

[gate]
oracle = "new-issues"
params = { repo = "acme/app" }
run_on = ["true", "failed"]
fire = "every"

[gate.failed]
instructions = "The new-issues oracle fails. Find out why and tell me."
```

A tick that starts no run is a **check**: no thread, no session and no quota, recorded as a grey entry in the routine's history. Consecutive checks with the same answer collapse into one line, such as “143 checks · False since 09:00 · not routed”. A run that starts gets the evidence in its message, fenced and marked as data from outside Launchpad, never as your instructions. Quota, the loop guard, the thread cap and the routine limits apply to gated runs as to any run. Failed never counts as a change: True, then Failed, then True again starts nothing new.

To run a routine only while an exit runner is online, gate it on an oracle that reads `tools.launchpad.egress.list()` (example in [Exit routes](exit-routes.md#oracles)).

The gate's oracle must exist when you save the routine: apply the oracle card first. A gate naming an oracle that is gone, or with wrong parameters, makes the run fail with the reason, shown on the routine and in Today → Fix. A paused or broken oracle turns each tick into a quiet check.

## Today

A failing oracle has one row in Today → **Fix**: “Oracle “New issues” is failing”, with the number of failed evaluations, the main error class, the last error, the failing call and the routines that wait on it. The row updates in place; it never adds one row per failure. Open it to see the error with the count of each class, the first and last failure, the routines that wait and the last good answer.

- **Open** opens the oracle. **⋯** has **Test now** (runs it again with the parameters of its last evaluation; a True or False answer ends the failure), **Pause** and **Mark read**.
- The row goes to Done at the next True or False (“Recovered after 7 failed evaluations”), or when you click **Mark read**. After Mark read, it comes back at the next failure.
- A broken oracle has the same kind of row, “Oracle “…” is broken”, with the reason, until you save a version that passes.
- A paused oracle's row is hidden while it is paused.

A gated routine whose oracle fails is not a “Routine failed” row: the oracle's row covers it.

## Schedules → Oracles

The **Oracles** tab of Schedules lists the workspace's oracles, with **New oracle** as its main button. The tab's count is amber when an oracle fails or is broken. Filters: **All**, **Failing** (broken ones too) and **Unused**. Each row shows a dot, the title and slug, the last answer (a red line with the streak when it fails, or the reason when it is broken), the routines that use it, its tools as chips and its last five outcomes as squares. **Open** opens it; **⋯** has **Test now** and **Pause** or **Resume**.

An oracle's page shows its title, slug and who edited it last, with **Save** (enabled once you change something) and **⋯** (**Discard changes**, **Pause** or **Resume**, and **Delete**, which names the routines gated on it). A broken oracle shows why at the top. Its tabs:

- **Definition**: the question and the file in an editor with Python colors, pre-check errors and type warnings at their lines, and the **Test** panel: one field per parameter, a cursor field when `check` takes one, then **Test**, which runs the file you see on Home. The answer shows the outcome, duration, error class, summary, fingerprint, proposed cursor, links, items, the call log and the log. A test never commits a cursor. A test of the saved file shows in History and ends a failure streak on True or False; a test of unsaved changes records nothing. A pre-check error blocks **Save** and **Test**.
- **Tools**: one switch per tool (`http`, `github`, `huggingface`, `launchpad`), each with what it reaches, its registry tool and whether that tool is granted to the workspace and has a token. The switches edit the header's `tools = […]` line. An added tool is highlighted before you save, and a credentialed tool combined with `http` says it can send what it reads to any public address.
- **History**: the recent evaluations as squares and as a list, **All** or **Failed**. Each line shows the time, the outcome and summary, and each caller with what it did (fired, quiet, counted, skipped); opening one shows its call log and log. Quiet answers with the same fingerprint collapse into one line.
- **Used by**: the routines gated on this oracle.
- **Versions**: each change with its author and diff, and **Restore**.

**New oracle** starts from a template with the header, one `check` and a commented call of each tool.

## CLI

Agents and you use `launchpad oracle`. The workspace is the session's, or `--workspace <id>`.

| Command | Access | What it does |
| --- | --- | --- |
| `launchpad oracle list` | read | The workspace's oracles, with their tools, last answer, failure streak, whether they are broken, and the routines gated on them. |
| `launchpad oracle show <slug>` | read | One oracle with its file, parameters, tools, type warnings and where its interface is. |
| `launchpad oracle history <slug> [--failed] [--limit 20] [--calls]` | read | Its recent evaluations with their callers; `--calls` adds each call log. |
| `launchpad oracle run <slug> [--param k=v]… [--cursor <name> \| --since <value>] [--max-age 2m] [--json]` | read | Runs the saved oracle on Home. `--cursor` names your own cursor for this oracle, which moves only on True or False. `--since` runs once from a cursor value. `--max-age` accepts an answer up to that age. |
| `launchpad oracle test --file <path> [--slug <slug>] [--param k=v]… [--cursor <value>] [--json]` | propose | Pre-checks the draft, then runs it on Home exactly like the saved oracle would, and records nothing. `--json` adds the log and the call log. |
| `launchpad oracle save --file <path> [--slug <slug>]` | admin | Saves the file, pre-checked and formatted (the slug defaults to the file name), on the terms of [Who may do what](#who-may-do-what). |
| `launchpad oracle pause <slug>`, `launchpad oracle resume <slug>` | admin | Pauses or resumes the oracle. |
| `launchpad propose oracle --title "…" --spec-file o.json` | propose | Proposes an oracle card; the file holds `{"slug": "ci-red", "content": "<the whole file>"}`. |

A pre-check error of `save`, `test` or `propose oracle` prints each error with its line and rule and exits `3`; type warnings are printed but do not stop it. The exit code of `run` and `test` is the outcome: `0` True, `1` False, `2` Failed. Every CLI error exits `3` (an unknown, paused or broken oracle, a bad parameter), so `1` and `2` always mean an answer.
