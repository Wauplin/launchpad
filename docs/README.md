# Launchpad documentation

## Use Launchpad

- [Getting started](getting-started.md): install Home, add a runner, onboarding tour, setup and first agent.
- [Today](today.md): every decision that waits for you, one compact row each, with its buttons.
- [Agents](agents.md): profiles, launching, sessions, worktrees, scheduling, quotas, Ask Launchpad and files.
- [GitHub](github.md): GitHub items as context, suggestions, private notes, Local PRs and the extension's view of github.com.
- [Remote access and phones](remote-access.md): pairing, Tailscale, the phone layout, notifications and a self-hosted Home.
- [Hugging Face](hugging-face.md): open models, trace copies in the Home bucket and Home in a Space.
- [Home and runners](home-and-runners.md): one Home that keeps records, runners and Burst that run agents, how sessions move and how work is saved.
- [Exit routes](exit-routes.md): reach a GitHub organization behind an IP allow list, or a tailnet service, through one runner that can reach it.
- [Knowledge](knowledge.md): the knowledge base of each workspace, the memory agents share.
- [Bots](bots.md): persistent agents with parallel threads, their instructions in `BOT.md` and their notes in the knowledge base.
- [Folios](folios.md): reports, plans, audits and sketchbooks that agents write as Markdown with typed blocks, which Launchpad renders and keeps, and where you comment, decide and send feedback.
- [Recall](recall.md): the search index over a workspace's past sessions (Funes), `launchpad recall`, backfill and rebuild.
- [Oracles](oracles.md): small Python checks that Home runs to answer True, False or Failed, the gates of routines, their Today row and the `launchpad oracle` CLI.
- [Bot channels and DMs](channels.md): channels and DMs where the bots of a workspace talk to each other, with you in every one, and the loop guard that stops them.
- [Connections and the Outbox](connections.md): a Telegram chat per workspace, outgoing webhooks whose drafts wait in Today, and incoming hooks.
- [Desktop companion](desktop.md): a Linux app for Ubuntu 26.04 GNOME with a Spotlight (ask, go to, quick launch), a top-bar menu of what waits for you, desktop notifications, a dock badge and updates in one click.
- [Chrome extension](../extension/README.md): Launchpad inside github.com (agents, private notes in the item's timeline, suggestions to Insert or post, Local PRs) and a chat about any page.
- [Configuration](configuration.md): files, environment variables, `config.toml` and the CLI.
- [Glossary](glossary.md): the words used in the UI.

## Work on Launchpad

- [Architecture](architecture.md): Home and runners, the UI, sessions, storage, the security model and the Rust crates.
- [Development](development.md): build, run, checks and CI.
- [Harness adapters](harness-adapters.md): how to add a coding agent harness.
- [Foundations spikes](foundations-spikes.md): harness logins through a proxy and conversations moved between runners, checked for the Home and runners plan.
- [Home in a Hugging Face Space](../deploy/hf-home/README.md): deploy, setup page, updates and the runner installer.
- [AGENTS.md](../AGENTS.md): rules for coding agents working on this repository.
- [Design invariants](invariants/README.md): the rules each area must keep, one page per area.
