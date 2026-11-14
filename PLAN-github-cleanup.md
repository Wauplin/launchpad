# Plan: stop mirroring GitHub in Launchpad

Temporary working plan (delete once implemented). Revised from the planning audit of session 95754735 (its full text, with evidence and file references, is in `/tmp/plan3ef/PLAN.md` on this runner when available; this file is the authority).

## Goal

github.com (plus the Chrome extension) owns GitHub notifications, issues and pull requests. Launchpad keeps only what it owns: agents, sessions, Local PRs, worktrees and branches, Suggestions (agent comments and reviews) with their feedback ledger, private notes, item links and topics, reports. Launchpad stops caching, polling and rendering GitHub items. Every link or button about a remote issue or PR opens github.com.

## Corrections to the audit plan

The audit plan is a good inventory but does not fit this repository as written:

- **No rollout machinery.** CLAUDE.md: single user, no backward compatibility, no versioning or migration framework, `cache.db` can be deleted. Drop capability negotiation, 410 skew windows, landing/redirect pages, staged releases, rollback checkpoints, durable-record checksums and "convert old records to visible disabled". One commit set that removes the feature end to end.
- **Its 8 open decisions are settled below**, using the owner's direction and the least surprising default. The implementer does not ask again.
- **Everything else in the audit stays valid as a checklist**: the dependency traps (the shared GitHub client persists every GET; routines, extension context, composer `#`, ledger confirmation, reply resolution, diff selections and repository discovery all read the caches), and the list of durable records to keep.

## Decisions

1. **Suggestions stay as they are**: Post from Launchpad (`suggestion.post`, including review line comments and the `--merge`/`--run-ci` follow-ups), Discard, extension Insert/prefill and the ledger. Only their dependence on cached items changes (see Replacements).
2. **Ready actions `github.run-ci` and `branch.push-to-pr` stay** (bots use them; they read GitHub live at the click). Local PR publish/publish-all/merge, Draft PR/Open PR when done, Push to PR follow (`git_follow`) stay unchanged.
3. **Generic item actions go** with the detail page: labels, reviewers, assignees, state, draft/ready, comment, merge from the item page (`items::mutate` and its actions/routes), the owner's pending review panel on a remote PR (`pending-review:` review-state keys, "Use" of an agent review). Keep only the functions that Suggestions' Post and its follow-ups still call.
4. **Routine GitHub triggers go** (`Trigger::Github`, `GithubEvent`, `github_seen` watermark, `Extensions::inbox_synced`, `crons::inbox_synced_inner`, editor options, CLI/docs). No live routine uses one. An old routine file with `kind = "github"` simply fails to parse and shows the existing `file_error` (never runs); no special conversion code.
5. **Inbox goes entirely**: polling task (`inbox::spawn`), sync, previews, subscriptions, thread actions (done/read/unsubscribe/restore/reply), `inbox.observed`, `inbox.done-handled`, `inbox_agent_done`, `launchpad inbox list|done|unsubscribe`, `BotSettings.inbox_done` (an existing `bot.toml` with `inbox_done` must still load: check how bot.toml parses and ignore or drop the key), agent context Inbox routes, the Inbox page, its nav entry, phone tab, counts, WS invalidation, browser offline snapshot key, status warnings and diagnostics about Inbox sync. State tables `inbox_override`, `inbox_active`, `inbox_observed`: stop creating them and drop them with `DROP TABLE IF EXISTS` at start.
6. **No GitHub response cache of any kind**: remove `cache.db` entirely (it holds only GitHub tables: `gh_response`, `inbox_*`, `gh_*`, `image_cache`; verify, then remove its path, open, schema, handle and tests, and delete the file at start if present), the `GitHub::request` GET persistence and conditional/ETag reuse, `GitHub::suggest`'s in-memory `recent` list, `items.rs` cached detail/list/diff/meta, and the remote image proxy (`/img`, `items::{image,proxy_images,collect_images}`, `<data_dir>/images`: delete the folder at start) if only remote item rendering uses it. Request coalescing is fine; storing payloads (disk or memory) is not.
7. **Remote detail views and lists go**: `web/src/features/github/ItemView.tsx`, `RepoItems.tsx` and everything only they use, the `/github/:owner/:repo/:number` route (no redirect page; links stop pointing there), folder Pull requests/Issues tabs (replace with plain external links to the repository's pulls/issues on github.com), metadata pickers, remote item/list/meta/diff/file/image APIs in `crates/api/src/items.rs`. Move shared diff/review components still used by Local PRs, Files or Suggestions instead of deleting them.
8. **`[github] poll_interval_seconds` goes** (config, template, `settings.set` shared list, Settings UI, docs). An existing config.toml with the key must still load. Keep GitHub login/status and the rate-limit display (explicit calls still use them). `home_update` release checks are unrelated: keep.
9. **Today**: remove notification-derived items (`TodayKind::ReviewRequest` and the like, demo entries); keep Suggestions, reports, Local PRs, agents, channel asks.
10. **Repository discovery from the Inbox goes** (`project_clone` reading `gh_item`/Inbox; Settings → GitHub list of repositories without a folder). Clone from GitHub by explicit name stays.
11. **Bots and KB**: the code change does not edit live bots. The coordinator handles `$LAUNCHPAD_KB` notes and bot instruction proposals afterwards.

## Replacements (live, unpersisted reads)

- **Canonical URL helper** in web (and the extension, which may reuse `web/src/lib`): `https://github.com/{owner}/{repo}/pull/{n}` when the kind is known, else `/issues/{n}` (GitHub redirects issue numbers of PRs to the PR); keep `#issuecomment-…`/`#discussion_r…` anchors and `/files` for diffs. Validate owner/repo names and positive numbers. Open in a new tab (`target="_blank" rel="noopener"`). Replace every internal `/github/…` link: router, TopBar/CommandField, Today rows/cards, Suggestions and LineComments ("Edit in pull request" becomes "Open on GitHub"), agents SessionSummary/GitPanel, BotChat, ProjectPage, ProposalCard, Local PR links to published PRs, Markdown link transforms, topics (topic pages stay internal; add the external link), Telegram/notification links if any, demo routes/fixtures, onboarding tour chapters, extension panel/content links. `rg '/github/'` must end with no internal route to remote items.
- **Suggestions**: `resolve_reply` reads the comment live (issue comment or review comment by ID; a `PRRT_` thread through one GraphQL query; a bare ID tries issue comment then review comment). Post's head check reads the PR head live (keep "outdated" marking of line comments on a moved head).
- **Ledger confirmation** (`ledger::confirm`, scheduled at 20 s, 2 min, 10 min, 1 h): read the item's issue comments (and review comments for line parts) live, newest pages first, and the owner's login live (`GET /user`, may be held in memory for the process). Same matching rules (owner, after insert, ≥ 85 %). Nothing else triggers confirmation now that item detail refreshes are gone.
- **Diff selections** (`selection_lines`): fetch the PR diff from GitHub at delivery, never cached; keep the "head moved" first line.
- **Composer `#` suggestions** (`prompt_context::github`, `GitHub::suggest`): live GitHub search (issues/PRs of the folder's repository) per query with the UI's existing debounce, no stored list.
- **Launch/message context with `github_refs`**: the prompt names the item (repo, number, URL, comment anchor) and lets the agent read it with its tools; it never inlines cached bodies. Check `compose.rs`/`prompt_context.rs`.
- **Extension context** (`crates/github/src/ext.rs`, `/api/ext/context|rows|notifications`): answer from Launchpad records only, still with no GitHub call. Drop fields that came from the cache: `ci`, `head_repo`, `maintainer_can_modify`, `outdated` (Post checks the head), Inbox thread fields and `handled`. `push_ready`: an agent branch of the item that is not followed yet and has commits (no cached head check; `push_to_pr` already validates fast-forward and fork permissions live at the click). Remove the extension's notifications-page integration (`content/github/notifications.ts`, `inbox.observed`, Done all handled, thread-ID rows) and the CI note next to Approve and run; keep Pull requests/Issues list rows by `owner/repo#number`. `/api/ext/notifications` keeps only Today counts (or goes if only the badge's Today count remains elsewhere).

## Keep (do not touch beyond the replacements)

`draft_comment` (Suggestions), `suggestion_ledger`, `predraft` and Local PR review state, `task_worktree`, Home Git, `git_follow`, `item_note`, `github_ref_use`, sessions, topics, proposals/reports, activity, `browser_operation`, Outbox, attribution signing, GitHub token handling, path validation (`github::valid_name`), rate-limit accounting, `home_update`.

## Work order (one implementer, one pass)

1. Core client: remove persistence from `GitHub::request`/`get`/`items`, remove `cache.db`, adapt callers (compile errors are the consumer list).
2. Replacements above (suggestions, ledger, selection lines, composer, prompt refs, extension context).
3. Remove Inbox, Today review requests, GitHub triggers, item views/APIs/actions, poll setting, image proxy, repository discovery, CLI commands, `inbox_done`, agent auth exceptions; regenerate types (`cargo run -p launchpad -- gen-types`).
4. Web and extension: links, removed pages and nav (laptop and phone), counts, offline cache keys, demo, tour, extension.
5. Repository docs and guidelines: README, CLAUDE.md (rules that mention Inbox, item caching, GitHub triggers, `inbox_done`, `inbox.observed`, ext context fields, notifications page, image proxy, poll interval), `docs/*`, `extension/README.md`, `skills/launchpad/SKILL.md`, `launchpad_cli::cli_reference`, `bots::instruction`, Ask Launchpad's seeded BOT.md, `kb::instructions`.

## Acceptance

- `rg -i 'inbox|gh_response|cache\.db|poll_interval|ReviewRequest|inbox_done|Trigger::Github|/img'` over crates, web, extension, docs and CLAUDE.md: every remaining hit is unrelated or justified (e.g. "Inbox" in `initial.md` history).
- No background GitHub request at idle (no poller task left); no table or file stores GitHub payloads.
- No internal route or link to a remote issue/PR; Local PR pages unchanged.
- Suggestions: create, reply, review with line comments, Post, Discard, Insert from the extension, ledger confirmation all still compile and keep their logic.
- Checks: `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; `cd web && pnpm typecheck && pnpm lint:hooks && pnpm ui-check` and the affected `pnpm test:<name>` (at least review, sessions, composer, operations, proposals, chat, tour, drafts, markdown); extension `pnpm typecheck && pnpm build`; deploy/hf-home Python tests only if touched.
