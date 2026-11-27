# Recall invariants

Recall is a derived search index over past sessions (`crates/recall`, [Recall](../recall.md)).

- Recall is evidence, never an authority. The knowledge base, bot memory, `BOT.md` and repository instructions win every conflict. Nothing is promoted from recall automatically.
- Every answer of `launchpad recall` and `recall get` starts with the same notice and fences each excerpt (`query::notice`, `senders::fence`); Home never passes Funes' own output on. Each hit carries its date and `lp:session/<id>/<seq>`.
- The index is derived only: every workspace's memory and ledger (`<data>/recall/ws/<workspace>`) can be deleted and built again from `state.db`. No backup, restore or record depends on it.
- One memory per workspace (its own `FUNES_HOME`). A search reads only the caller's workspace, which Home takes from the session (`agent_auth`, `Names::Workspace`), and drops hits whose session is no longer in it.
- Reads need read access; a bot's session also needs `recall = true` in its bot.toml (`launchpad_api::recall::caller_bot`). Backfill, rebuild and the settings are admin actions (`recall.*`). The settings are Home-wide: only admin access reads them, and an agent changes only its own workspace's entries of `home.recall.hidden`.
- Off by default (`home.recall.enabled`). Off, or without the `funes` program or its weights, Home indexes nothing and searches answer a clear error; nothing else changes.
- Recall never affects a turn, a backup or Home's start: indexing runs after the turn in a background task, and every failure ends in the log, the ledger (`recall status`) or the backfill report.
- Only finished turns are indexed, each once: Funes' chunk IDs ignore the text, so a turn given twice keeps its first text. The converter (`convert.rs`) is deterministic, and a change of its output needs a new `CONVERTER_VERSION`, which makes Home rebuild each memory.
- A session's ledger row moves only after Funes accepted its turns; the Funes `seq` of a session is dense and assigned by the ledger. Backfill and indexing are therefore idempotent and resumable.
- One Funes writer at a time on Home (`index::Service::writer`), at a low priority, and one search at a time (`index::Service::reader`).
- Archived and hidden sessions are filtered when a search answers, from Home's records, and left out of the next rebuild. A rebuild is the only deletion.
- `funes` runs as a one-shot child with a cleared environment: no token, its own `HOME`, `HF_HOME` and `HF_HUB_CACHE` on the weights of the image, `HF_HUB_OFFLINE` and `DO_NOT_TRACK` set. Home never downloads weights and never uses Funes' integrations, hooks or Hub push.
- The `funes` commit and each model file's revision and SHA-256 are pinned in `deploy/hf-home/release.json`.
