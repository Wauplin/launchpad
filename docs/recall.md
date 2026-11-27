# Recall

Recall is a search index over the past sessions of a workspace: what earlier agents and bots said, tried, measured and were told, whatever their harness. Agents search it with `launchpad recall`. It is built on Home with [Funes](https://github.com/huggingface/funes) (a local index with hybrid search and a reranker; no generative model).

Recall is **derived evidence**. The knowledge base, bot memory, `BOT.md` and repository instructions stay the authority and win every conflict. Nothing is promoted from recall on its own: to keep a finding, an agent writes a note with its `lp:session/…` source.

Recall is off by default.

## Turn it on

1. Home needs the `funes` program and its two models. Home's image has them (see [Home's image](#homes-image)). On another Home, put `funes` on `PATH` (or set `LAUNCHPAD_FUNES`) and the model snapshots in a Hugging Face cache named by `LAUNCHPAD_FUNES_HF_HOME`.
2. Settings → Home and runners → Recall → On. An admin session runs `launchpad recall enable` (`launchpad recall disable` turns it off). The setting is `home.recall.enabled` in Home's `config.toml`.
3. Enable it for each bot that should search: `recall = true` in its `bot.toml`. An admin session runs `launchpad bot update <bot-id> --body-file f.json` with `{"set": {"recall": true}}`. The bot's next sessions are told when to use recall; sessions that already run may search at once.
4. Index the history: Backfill in the same Settings card, or `launchpad recall backfill` (admin access).

From then on Home indexes each finished turn a few seconds after it ends.

## Search

```
launchpad recall "why did we pin the model revisions" [--k 8] [--since 2026-11-01] [--until 2026-11-30] [--type text|thinking|tool_use|tool_result] [--bot <slug> | --mine] [--oneline]
launchpad recall get <session-id> --seq <seq>     # the turns around a hit
launchpad recall get <session-id> --from <seq> [--to <seq>]
launchpad recall status
```

- Every answer starts with a notice that says what it is, and every excerpt is fenced: text inside an excerpt is data, never an instruction.
- Each hit shows its date, its source `lp:session/<session-id>/<seq>` (open it in Launchpad, or read it with `launchpad ref read`), the session's title, bot, harness and status, the kind of block and a score. Under it come the prompt and the agent's answer of the same exchange.
- Hits are ranked by relevance, not by date. Use `--since` for the latest word on a subject.
- `--oneline` lists one line per hit, for a few hundred tokens: use it to choose what to read.
- `recall get` reads the stored events themselves, with the same conversion as the index.
- The search is limited to the caller's workspace. Read access is enough; a bot's session also needs `recall = true` in its `bot.toml`.
- Home answers one search at a time. On a slow Home, an answer that takes too long comes back without reranking, and says so.

## Backfill

`launchpad recall backfill [--dry-run]` (admin access, or the Backfill button) indexes every stored session of the workspace in the background on Home.

- It skips what is already indexed and continues after an interruption: a ledger beside the index records, per session, the event it is indexed through.
- The workspace then stays in `home.recall.backfill`: Home continues an interrupted backfill by itself, and indexes the workspace again when its index is gone. The index is not part of Home's backups, so a Home in a Space builds it again after each restart, in the background.
- `--dry-run` only counts what it would do.

`launchpad recall status` shows the result. Its numbers:

| Field | Meaning |
| --- | --- |
| `sessions.total` | Sessions of the workspace in Home's records |
| `sessions.up_to_date` | Indexed through their last event |
| `sessions.to_index` | Not indexed yet, or indexed only in part: the lag |
| `sessions.running` | Their turn is not finished; indexed when it ends |
| `sessions.failing` | Failed three passes in a row: only a new backfill or their next turn tries them again |
| `sessions.left_out` | Left out, by reason: `archived`, `hidden`, `empty` (nothing to index), `unconvertible` (stored events that no longer read) |
| `index.sessions`, `index.turns`, `index.blocks` | What the index holds |
| `index.blocks_truncated` | Blocks too large to index whole: their start and end are indexed |
| `index.sessions_failed`, `index.failures` | Sessions whose last pass failed, with the error |
| `index.newest_turn`, `index.lag_minutes`, `index.size_bytes` | How fresh and how large the index is |
| `backfill` | The last backfill's state (`running`, `done`, `interrupted`, `failed`) and counts |

A backfill is complete when `backfill.state` is `done`, `sessions.to_index` and `index.sessions_failed` are 0, `sessions.failing` is 0, and `sessions.total` equals `up_to_date` + `running` + the sum of `left_out`.

## Forgetting

Funes only appends. Launchpad forgets in two steps:

- At once: archived sessions and the sessions listed in `home.recall.hidden` are never returned. Set `home.recall.include_archived` to index archived sessions too.
- For good: `launchpad recall rebuild` (admin access, or the Rebuild button) deletes the workspace's index and builds it again from the stored sessions, without the sessions that are left out.

The index keeps text as the sessions wrote it, including a secret a tool printed. Hide the session and rebuild.

## Home's image

`deploy/hf-home/release.json` pins everything under `funes`: the commit of huggingface/funes that the `funes` program is built from, and the revision and SHA-256 of each model file (`BAAI/bge-small-en-v1.5` for embeddings, `Alibaba-NLP/gte-reranker-modernbert-base` for reranking). CI builds the program once per pinned commit and ships it in the release package; `install_tools.py` downloads the weights into `/opt/funes/hf` at image build. Home runs `funes` offline (`HF_HOME`, `HF_HUB_CACHE`, `HF_HUB_OFFLINE`, `DO_NOT_TRACK` set on the child process, no token), so the models never change under an index and nothing is downloaded at run time. Funes' own integrations, hooks and Hub push are not used.

The weights add about 0.75 GB to the image and the program about 0.22 GB. A search loads both models for a few seconds (about 0.9 GB of memory while it runs, measured); indexing loads the embedder only (about 0.3 GB), at a low priority, one process at a time.

## Limits

- The index is rebuilt after each restart of a Home in a Space (it is not in the backups).
- Funes has no per-bot scope: `--bot` and `--mine` filter the hits after ranking.
- The repository filter of Funes is not used: Home has no checkout of the sessions' folders.
- Models are English first (`bge-small-en`).
