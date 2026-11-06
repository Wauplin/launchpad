---
name: implement-review-docs
description: Run a multi-step plan on Launchpad with fresh subagents per step (implementer, then reviewer, then docs), with the docs of step N running at the same time as the implementer of step N+1. Use when the owner hands over a plan with several steps or waves and asks to implement it with subagents.
---

# Implement, review, document

You are the orchestrator. You do no implementation yourself and keep your context small: you write prompts, start agents, read their short answers and keep a step table. Every step gets three fresh subagents: an **implementer**, a **reviewer** and a **docs** agent.

## Timeline

Only one agent at a time changes code. The docs agent only changes prose, so it runs next to the next implementer:

```
impl 1 → review 1 → impl 2 → review 2 → impl 3 → review 3
                  ↘ docs 1           ↘ docs 2           ↘ docs 3
```

- Start implementer N+1 and docs N in the same message, right after reviewer N ends.
- Start docs N+1 only after docs N ended (they edit the same files).
- Never run two implementers, two reviewers, or an implementer and a reviewer at the same time.
- Agents run in the background. Wait for their notifications; never poll, never predict a result.

## Before step 1

1. Make a run folder: `/tmp/lp-steps/<short-name>/`. Put the plan there as `plan.md` (or link to it) and write `common.md` from the template below. Fill in the worktree, the branch and the owner's answers.
2. Read AGENTS.md, then only the parts of the plan you need to split it into steps. A step should fit in one implementer context: about 15 items or 3 crates at most. Split a bigger step into sub-steps (6.1, 6.2, …), each with its own implementer.
3. For each step, note the files and functions the implementer will need. File pointers save many exploration calls.
4. Ask the owner now about open decisions, all in one question. Agents never ask the owner: they write the question at the top of their report and skip that item.

Keep this table in your context and update it after each agent:

| Step | Base commit | Impl | Review end commit | Docs |
|---|---|---|---|---|

## For each step N

1. **Implementer** (effort `medium`). Record `base_N=$(git rev-parse HEAD)` first. Prompt: the Implementer template below.
2. **Reviewer** (effort `high`), after the implementer ends. Prompt: the Reviewer template, with the range `base_N..HEAD`. When it ends, record `end_N=$(git rev-parse HEAD)`.
3. **Docs** (effort `medium`), with implementer N+1 in the same message. Prompt: the Docs template, with the range `base_N..end_N`.
4. Read each final answer (5–10 lines). Open a report file only when an agent is blocked or failed.

If an agent stops because the quota ran out, continue the same agent with SendMessage when the quota is back. A fresh agent would lose its context.

After the last docs agent, check `git status` (clean), run the CI set once if the last reviewer did not, and write the final summary: steps done, commits, checks and results, open questions, items left.

## Rules that save the most time

They come from the analysis of an earlier 11-hour run. Put them in `common.md`, so every agent gets them.

- **Builds**: pass `timeout: 600000` to Bash for cargo, Clippy, `cargo test` and `pnpm build`. Do not move a build to the background and then poll it with `sleep` or `until` loops. While working, use `cargo check --all-targets`. Run Clippy and `cargo test` once, before the last commit.
- **Screenshots**: use `node web/scripts/shots.mjs` only. Never write a new Playwright or DevTools script. See [Screenshots](../../../docs/development.md#screenshots).
- **GitHub**: never wait for real GitHub Actions runs. Check release and CI changes with local fakes or throwaway bare remotes. If a real run is needed, start it once in the background, name it in the report and move on.
- **Reading**: read whole sections at once (Read with a range, Grep with context) instead of many small `sed -n` calls. Start from the file pointers in the prompt.
- **Reports**: at most about 120 lines. Final answer to the orchestrator: 5–10 lines.

## Shared Git worktree

All agents commit in the same worktree, and two of them can run at the same time. So every agent follows these rules:

- Commit only your own files: `git commit -m "<message>" -- <path>...`. Never `git add -A`, `git add .` or `git commit -a`.
- Never use `git stash`, `git reset`, `git checkout -- <path>`, `git restore`, `git rebase` or `git clean`.
- If Git says `index.lock` exists, wait two seconds and try again. Never delete the lock file.
- The docs agent edits only `README.md` and `docs/`. Implementers and reviewers own everything else, including AGENTS.md (rules), tour chapters and demo fixtures.
- The docs agent reads the step's code with `git show` and `git diff base_N..end_N`. The working tree may already hold the next step's changes.
- The docs agent does not build or take screenshots: `web/dist` belongs to the implementer that runs at the same time. It uses the reviewer's screenshots.

## Template: common.md

```markdown
# Rules for every subagent of this run

Repository (your working directory): <worktree> on branch `<branch>`. Commit there with short messages that say what changed, each ending with the attribution line of the session. Never push, switch branches, create branches or worktrees, or merge.

## Sources

- AGENTS.md (= CLAUDE.md) is the law; it wins over the plan. Read it fully first. Read README.md and docs/architecture.md where your step touches them.
- The plan: /tmp/lp-steps/<name>/plan.md. The owner's answers (final, never ask again): <answers or file>.
- Earlier reports: /tmp/lp-steps/<name>/step<N>-impl.md, -review.md, -docs.md.

## Rules

- Follow AGENTS.md: safety rules, crate layering, the action list, the decision list (Today), no harness branches, UI rules, demo mode and tour chapters.
- When the step changes a rule, rewrite that rule in AGENTS.md in the same step (implementer or reviewer).
- Never stop or restart the production daemon (port 7317) or use its data. Use the dev daemon (port 7318) or scratch daemons. Clean up every terminal, test routine, schedule, worktree, scratch daemon, browser and server you start.
- Builds: Bash `timeout: 600000` for cargo, Clippy, `cargo test` and `pnpm build`; no background builds polled with sleep loops. `cargo check --all-targets` while working, from the repository root, never `-p`.
- Screenshots: `node web/scripts/shots.mjs --views 100,120,150,phone --out /tmp/lp-steps/<name>/shots/step<N> <pages>` (add `--demo` for demo mode). Never write your own screenshot script.
- GitHub: never wait for real GitHub Actions runs; use local fakes or throwaway bare remotes.
- Shared worktree: commit with `git commit -m "…" -- <paths>`; never `git add -A`, `git commit -a`, stash, reset, restore, rebase or clean. If `index.lock` exists, wait two seconds and retry.
- Open decisions: do not guess. Write the question at the top of your report and skip that item.

## Report

Write your report to the file named in your task: what you did (with commit hashes), what you left and why, the checks you ran with their results, and your suggestions (one line each). At most about 120 lines. Final answer to the orchestrator: 5–10 lines (state, commits, blockers).
```

## Template: Implementer

```
You are the implementer of step <N> (<title>) of <plan>. First read /tmp/lp-steps/<name>/common.md and follow it.

Your task: <the step's items, copied or pointed to precisely>. Owner answers that apply: <…>.
Start from these files: <file pointers: paths and function names>.

Done when: <the plan's check for this step>.

Commit as you go. Before your last commit, run Clippy and `cargo test` (if Rust changed), `(cd web && pnpm typecheck)`, `pnpm lint:hooks` (if hooks changed) and the matching `pnpm test:<name>`. For UI changes, take screenshots with web/scripts/shots.mjs into /tmp/lp-steps/<name>/shots/step<N>/impl. Leave README.md and docs/ to the docs agent; update AGENTS.md, tour chapters and demo fixtures yourself. Write your report to /tmp/lp-steps/<name>/step<N>-impl.md.
```

## Template: Reviewer

```
You are the reviewer of step <N> (<title>) of <plan>. First read /tmp/lp-steps/<name>/common.md and follow it. Then read /tmp/lp-steps/<name>/step<N>-impl.md.

Review the step's code: `git log <base_N>..HEAD -- . ':!README.md' ':!docs'` (the docs agent of the previous step may commit prose at the same time; ignore it). Check it against the plan, the owner's answers and AGENTS.md: safety rules, crate layering, the action list, the decision list, no harness branches, UI rules, demo and tour. Check the "Done when" yourself. Fix what you find and commit.

Run the CI set once at the end: `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings`, `cargo test`, `(cd web && pnpm typecheck)`, `(cd web && pnpm lint:hooks)` and the matching `pnpm test:<name>`. For UI changes, take the screenshots the docs agent will use: `node web/scripts/shots.mjs --views 100,120,150,phone --out /tmp/lp-steps/<name>/shots/step<N> <pages>`, in demo mode and with real data. Write your report to /tmp/lp-steps/<name>/step<N>-review.md.
```

## Template: Docs

```
You are the docs agent of step <N> (<title>) of <plan>. First read /tmp/lp-steps/<name>/common.md and follow it. Then read /tmp/lp-steps/<name>/step<N>-impl.md and step<N>-review.md. The step's changes are `git diff <base_N>..<end_N>`; read code with `git show`, because the next implementer is already changing the working tree.

Bring README.md (features) and the matching docs/ pages up to date with: <what the step changed for the owner>. Edit only README.md and docs/. Do not build and do not take screenshots: use the reviewer's screenshots in /tmp/lp-steps/<name>/shots/step<N>/. If AGENTS.md, a tour chapter or the demo looks wrong, write it in your report; do not fix it.

Write plain technical American English with short sentences, one line per paragraph. Before committing, run `.claude/skills/implement-review-docs/prose-check.py HEAD README.md docs/` (your uncommitted lines) and fix what it reports (aim for a reading ease above 50). Commit with `git commit -m "…" -- README.md docs/`. Write your report to /tmp/lp-steps/<name>/step<N>-docs.md.
```
