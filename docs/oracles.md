# Oracles

An oracle is a small check that answers one question with **True**, **False** or **Failed**, for example “is CI red on main?” or “is there a newer release on PyPI?”. It is one deterministic Python file with inline dependencies. Home runs it with `uv`, without a model and without a session. Routines use oracles as gates: a gated routine starts its bot or agent only when the oracle's answer calls for it, and stays quiet the rest of the time. Agents and you can also run an oracle directly instead of polling an API.

Oracles belong to a workspace. They live on the **Schedules** page, in the **Oracles** tab.

## The file

An oracle is the file `oracles/<slug>.py` of its workspace. The slug uses lowercase letters, digits, `-` and `_`. Home stores the file like a plain folder, with **Versions**, diffs and **Restore**. A file is at most 256 KB.

The file starts with a [PEP 723](https://peps.python.org/pep-0723/) block: `requires-python`, `dependencies` and a `[tool.launchpad]` table. Then comes the code, which ends with `run(check)`:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.launchpad]
# title = "CI red on a branch"
# description = "True when the latest completed run of a workflow failed."
# params = { repo = "repo", branch = "string:main", workflow = "string:tests.yml" }
# min_interval = "2m"
# access = { tools = ["gh"] }
# ///
import os
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    p = ctx.params
    r = ctx.http.get(f"https://api.github.com/repos/{p.repo}/actions/workflows/{p.workflow}/runs",
                     params={"branch": p.branch, "status": "completed", "per_page": 1},
                     headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"]})
    r.raise_for_status()
    runs = r.json()["workflow_runs"]
    if not runs:
        return verdict(False, summary="no run yet")
    last = runs[0]
    return verdict(last["conclusion"] == "failure",
                   summary=f"{p.workflow} on {last['head_sha'][:7]}: {last['conclusion']}",
                   fingerprint=last["head_sha"], links=[last["html_url"]])


run(check)
```

The keys of `[tool.launchpad]`:

| Key | Meaning |
| --- | --- |
| `title` | Required. The name shown everywhere. |
| `description` | The question the oracle answers. |
| `params` | Typed parameters: `string`, `repo` (`owner/name`), `bool`, `int` or `number`. Write `type:default` for a default value (`"string:main"`); a parameter without a default is required. |
| `min_interval` | How long an answer stays fresh, at least `1m` (the default). Callers within that time get the same answer instead of a new run. |
| `timeout` | At most `60s`, `10s` by default. |
| `access` | What the oracle may reach: `tools`, `hosts` and `launchpad` (see [Access](#access)). |

Durations are written `500ms`, `10s`, `2m`, `1h`, `1d`, or as a number of seconds. Unknown keys are refused, so a typo shows at once. Saving refuses a file that does not parse, or that names a tool Home's registry does not have.

## The helper

Every oracle imports `launchpad_oracle`, a small helper that ships inside Launchpad. Home adds it to each run, so it never goes in `dependencies`. Its API only grows: Launchpad updates add to it and never remove or rename.

- `run(check)` reads the input, calls `check(ctx)` and writes the result. Call it once, at the end of the file.
- `Context` is what `check` gets: `ctx.params` (typed, as `ctx.params.repo` or `ctx.params["repo"]`), `ctx.event` (why it runs, for example `{"kind": "schedule"}`), `ctx.cursor` (the caller's cursor, or `None`), `ctx.now` (the start time, in UTC), `ctx.timeout`, `ctx.hosts`, `ctx.http` and `ctx.launchpad`.
- `verdict(value, *, summary, fingerprint, links, items, cursor)` returns True or False with its evidence. `fingerprint` names what is true (a commit, a version), so a gate can fire again when it changes. `cursor` proposes a new cursor (see [Cursors](#cursors)); leave it out to keep the current one.
- `failed(reason)` stops the check with Failed: use it when the oracle cannot decide (a partial page, an unexpected answer). Never return False for that.
- `ctx.http` is a plain [httpx2](https://pypi.org/project/httpx2/) client with the oracle's timeout. Every request, redirects included, goes only to the allowed hosts; another host raises `HostRefused`, and the result is Failed. All HTTP methods are allowed.
- `ctx.launchpad` is `{"url", "token", "workspace"}` when the oracle has `launchpad = "read"`, else `None`. Call Home's read API through `ctx.http` with `Authorization: Bearer <token>`.

`print` inside `check` goes to the oracle's log (stderr), not to the result.

## Outcomes

An evaluation ends with one of three outcomes:

- **True** or **False**: the oracle's answer, with its summary, fingerprint, links and items (at most 32 KB together).
- **Failed**: the oracle could not answer. Failed is never read as False. Its class says why: `access` (a host or a tool it may not use), `http` (an HTTP error status or a connection error), `timeout`, `env` (uv could not prepare the environment, for example an unknown dependency), `script` (an exception in the code), `contract` (no result, or a result that breaks the rules) or `declared` (the oracle called `failed()`).

Each evaluation is kept for 30 days (at most 2,000 per oracle), with who asked for it and what each caller did with the answer.

## How Home runs an oracle

Home runs every saved oracle with `uv run`, with the helper added, and no lock file. You keep dependencies up to date yourself. Home needs `uv` on its PATH; the Space image includes it.

- **Limits**: 16 evaluations run at once; more wait for a free slot. uv gets up to two minutes to prepare the environment, then the timeout starts. At the timeout, Home stops the whole process. A proposed cursor is at most 4 KB, the evidence at most 32 KB.
- **One run for many callers**: callers that ask the same oracle with the same parameters and cursor at the same time share one run. Within `min_interval`, they get the last answer again (also a Failed one, so a failing API is not called in a loop).
- **Environment**: Home builds it from scratch: `PATH`, a temporary `HOME`, uv's cache under Home's data folder, and the real token of each tool the oracle lists, under the tool's variables (for example `GH_TOKEN`). Nothing else from Home reaches the oracle.
- **Secrets stay out of results**: every token an evaluation held is replaced with `[redacted]` in its summary, links, items, errors and log before anything stores or shows them.

## Access

An oracle reaches nothing until its file says so. `access` takes three keys, each empty by default:

- `tools`: tools of Home's registry (Settings → Credentials and tools), for example `["gh"]`. Each one puts its real token in the oracle's environment and adds the tool's hosts to the allowed hosts. A tool that is not granted to the workspace, or has no token, fails the evaluation with class `access`.
- `hosts`: other hosts `ctx.http` may call, such as `["pypi.org"]`. `*.example.com` allows its subdomains; `"*"` allows every host.
- `launchpad`: `"none"` (the default) or `"read"`. With `"read"`, Home gives each evaluation its own token for Home's read API (`LAUNCHPAD_URL`, `LAUNCHPAD_TOKEN` and `LAUNCHPAD_WORKSPACE` in the environment, and `ctx.launchpad`). It reads what a read-level agent of this workspace reads, with GET requests only, and never runs oracles. Home keeps it in memory and revokes it when the evaluation ends.

Oracle code is trusted: it runs on Home with real tokens, outside the agents' jail and proxy. That is why only you widen an oracle's access (see [Who may do what](#who-may-do-what)). The host check of `ctx.http` keeps honest code on its hosts; it is not a sandbox.

## Cursors

A cursor lets an oracle answer "what is new since last time?". Each consumer has its own cursor: a gated routine has one, and each name you give `--cursor` on the CLI has one per oracle. The oracle reads it as `ctx.cursor` and proposes the next one with `verdict(..., cursor=...)`.

The cursor moves only when the consumer acts on the answer. A gated routine commits it in the same step that claims its run, so items are never lost when a run fails to start. A False that starts no run commits at once. A Failed never moves a cursor.

## Gates

A routine with a schedule trigger can have a gate: before each scheduled run, Home asks an oracle, and the answer decides whether the run starts. Gates work on bot routines and on other routines. Run now ignores the gate.

In the routine editor (**Edit** on Schedules, or the bot's About → **Triggers**), a schedule trigger has **Only when an oracle says**: pick an oracle of the workspace (or **Always**, for no gate), fill in its parameters, then choose:

- **Outcomes**: which answers start a run. **True** and **False** start a run with the routine's instructions and the evidence. **Failed** starts a repair run, once per failure streak, with its own instructions. An unchecked outcome stays silent.
- **Fire** (required, no default): **When it changes** starts a run when the outcome changes, or when True comes back with a new fingerprint. **Every time** starts a run for every routed answer.
- **Cooldown** (optional, for example `6h`): the same answer with the same fingerprint stays quiet that long after a run.
- **Remind** (optional, for example `24h`): with **When it changes**, the same answer starts a run again once that long has passed.

A bot routine file writes the gate as a `[gate]` table:

```toml
title = "Fix red CI"
schedule = "*/10 * * * *"
instructions = "CI is red: find the cause and fix it in a Local PR."

[trigger]
kind = "schedule"

[gate]
oracle = "ci-red"
params = { repo = "acme-labs/storefront" }
run_on = ["true", "failed"]
fire = "change"
cooldown = "6h"

[gate.failed]
instructions = "The ci-red oracle fails. Find out why and tell me."
```

A tick that starts no run is a **check**: no thread, no session and no quota, recorded as a grey entry in the routine's history. Consecutive checks with the same answer collapse into one line, such as "143 checks · False since 09:00 · not routed". A run that starts gets the evidence in its message, fenced and marked as data from outside Launchpad, never as your instructions. Quota, the loop guard, the thread cap and the routine limits apply to gated runs as to any run. Failed never counts as a change: True, then Failed, then True again starts nothing new.

The gate's oracle must exist when you save the routine: apply the oracle card first. A gate naming an oracle that is gone, or with wrong parameters, makes the run fail with the reason, shown on the routine and in Today → Fix. A paused oracle turns each tick into a quiet check.

## Today

A failing oracle has one row in Today → **Fix**: “Oracle “New PRs and issues” is failing”, with the number of failed evaluations, the main error class and the last error, and the routines that wait on it. The row updates in place; it never adds one row per failure. Open it to see the error with the count of each class, the first and last failure, the routines that wait and the last good answer.

- **Open** opens the oracle. **⋯** has **Test now** (runs it again with the parameters of its last evaluation; a True or False answer ends the failure), **Pause** and **Mark read**.
- The row goes to Done at the next True or False (“Recovered after 7 failed evaluations”), or when you click **Mark read**. After Mark read, it comes back at the next failure.
- A paused oracle's row is hidden while it is paused.

A gated routine whose oracle fails is not a “Routine failed” row: the oracle's row covers it.

## Schedules → Oracles

The **Oracles** tab of Schedules lists the workspace's oracles, with **New oracle** as its main button. The tab's count is amber when an oracle fails. Filters: **All**, **Failing** and **Unused**. Each row shows a dot, the title and slug, the last answer (a red line with the streak when it fails), the routines that use it, its access, its dependencies and its last five outcomes as squares. **Open** opens it; **⋯** has **Test now** and **Pause** or **Resume**.

An oracle's page shows its slug, who edited it last, its Python version and its dependencies, with **Save** (enabled once you change something) and **⋯** (**Discard changes**, **Pause** or **Resume**, and **Delete**, which names the routines gated on it). Its tabs:

- **Definition**: the question and the file (an editor with Python colors), and the **Test** panel: one field per parameter and a cursor, then **Test**, which runs the file you see on Home. The answer shows its outcome, duration, error class, summary, fingerprint, proposed cursor, links and log. A test never commits a cursor. A test of an unsaved draft records nothing; a test of the saved oracle shows in its History, and a True or False ends its failure streak.
- **Access**: what the oracle can reach. Switches for the workspace's tools (with their hosts and variables), a field for hosts, and **Launchpad state (read, this workspace)**. The switches edit the file's `access = { … }` line, and wider access is highlighted before you save. **Environment** lists what the process gets.
- **Dependencies**: the Python version, the inline dependencies, the bundled helper and how Home runs the file. Dependency updates are yours.
- **History**: the recent evaluations as squares and as a list, **All** or **Failed**. Each line shows the time, the outcome and summary, and each caller with what it did (fired, quiet, counted, skipped). Quiet answers with the same fingerprint collapse into one line.
- **Used by**: the routines gated on this oracle.
- **Versions**: each change with its author and diff, and **Restore**.

## Who may do what

| Who | What |
| --- | --- |
| You | Everything: create, edit, test drafts, pause, mark read, restore and delete. Delete asks again when routines are gated on the oracle. |
| Admin bots | Save code and dependency changes directly (`launchpad oracle save`), pause and resume. A version that adds a tool, a host or Launchpad read is refused: it must be a card for you. The bot's chat shows each change as a receipt. |
| Other agents | List, read, run and test drafts in their own jail, and propose an oracle with a card. |

An **oracle card** (`launchpad propose oracle`) holds the slug and the whole file. It shows the diff, the access it adds (amber) or removes, and **Apply**. An admin session may accept a card only when it adds no access. Oracle cards never start on their own, even for a bot in auto mode.

## CLI

Agents and you use `launchpad oracle`. The workspace is the session's, or `--workspace <id>`.

| Command | What it does |
| --- | --- |
| `launchpad oracle list` | The workspace's oracles, with their last answer, failure streak and users. |
| `launchpad oracle show <slug>` | One oracle with its file, parameters and access. |
| `launchpad oracle history <slug> [--failed] [--limit 20]` | Its recent evaluations, with their callers. |
| `launchpad oracle run <slug> [--param k=v]… [--cursor <name> \| --since <value>] [--max-age 2m] [--json]` | Runs it on Home. `--cursor` names your own cursor for this oracle, which moves only on True or False. `--since` runs once from a cursor value. `--max-age` accepts a fresh answer (never fresher than `min_interval`). |
| `launchpad oracle test --file <path> [--param k=v]… [--cursor <value>] [--json]` | Runs a draft in the agent's own jail, never on Home, and records nothing (see below). |
| `launchpad oracle save --file <path> [--slug <slug>]` | Admin: saves the file (the slug defaults to the file name). |
| `launchpad oracle pause <slug>`, `launchpad oracle resume <slug>` | Admin. |
| `launchpad propose oracle --title "…" --spec-file o.json` | Proposes an oracle card; the file holds `{"slug": "ci-red", "content": "<the whole file>"}`. |

The exit code of `run` and `test` is the outcome: `0` True, `1` False, `2` Failed. Every CLI error exits `3`, so `1` and `2` always mean an answer.

`oracle test` runs the draft with `uv run` on the agent's runner, with the session's own tools (through its proxy) and the same helper. It notes on stderr when a listed tool is not granted to the workspace (the oracle would fail on Home) and that Launchpad read access is not available in a local test (`ctx.launchpad` is `None`). The runner needs `uv`: its setup checklist recommends it, with the command that installs it.
