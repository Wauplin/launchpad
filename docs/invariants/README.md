# Design invariants

The rules that each area of Launchpad must keep. [AGENTS.md](../../AGENTS.md) holds only the short, cross-cutting rules; the detail of each area lives here. Read the page of an area before changing it, and update that page in the same commit when you change or add one of its rules. Read only the pages you need: together they are large.

| Area | Page |
| --- | --- |
| Crate layering, single user, click only, OS, presets | [general.md](general.md) |
| Harness contract, session states, Today, schedules and routines, sleeping, attachments, idempotency, notifications | [sessions.md](sessions.md) |
| Profiles, quota monitors, automatic work, paid usage, spend limits | [quota.md](quota.md) |
| Links and short IDs of Launchpad objects (`refs`) | [references.md](references.md) |
| Topics | [topics.md](topics.md) |
| Folders, Home Git, temporary folders, worktrees, Local PRs, merges, plain folders, Files | [folders.md](folders.md) |
| Bind address, origins, pairing, devices, extension and desktop authentication, SSE bus | [remote-access.md](remote-access.md) |
| GitHub reads and writes, extension context, diff selections, Push to PR, attribution, ledger | [github.md](github.md) |
| Bots, threads, routing, conversations, senders and hops, Chats, bot page, publishing | [bots.md](bots.md) |
| Bot channels and DMs, delivery, loop guard, digests, files, scheduled posts | [channels.md](channels.md) |
| Connections, Outbox, incoming hooks, phone notifications | [connections.md](connections.md) |
| Ask Launchpad, Launchpad CLI access levels, workspace boundary for agents, proposals and cards | [proposals.md](proposals.md) |
| Oracles, their pre-check, evaluation and routine gates | [oracles.md](oracles.md) |
| Recall: the derived search index over past sessions | [recall.md](recall.md) |
| Folios, their format, reader, sketches, data API, marks and Share | [folios.md](folios.md) |
| Desktop companion (`desktop/`) | [desktop.md](desktop.md) |
| Terminals | [terminals.md](terminals.md) |
| Home and runners: replication, tunnel, placement, moves, updates, backups, Burst, exit routes, trace copies | [home-and-runners.md](home-and-runners.md) |
| Knowledge base, bot memory, the Librarian | [knowledge.md](knowledge.md) |
| Credentials, the jail, daemon Git, `safe_fs`, network capture, vault, proxy, Slack, leases | [jail.md](jail.md) |
| UI rules in full: components, sizes, action groups, phone shell, workspaces, demo, setup | [ui.md](ui.md) |
| Chrome extension (`extension/`) | [extension.md](extension.md) |
