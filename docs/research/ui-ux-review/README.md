---
title: Launchpad UI and UX review
emoji: 🧭
colorFrom: green
colorTo: gray
sdk: static
app_file: index.html
pinned: false
fullWidth: true
header: mini
short_description: "28 UX findings and five interactive concept sketches"
---

# Launchpad UI and UX review

An evidence-based heuristic review prepared on 2026-10-10 against commit `96ad47642b20c8f08fd3ebf4d142369df578db2c`. Covers the shared shell and all major user-facing workflows, with 28 prioritized findings, code references, tradeoffs, acceptance checks, five interactive concept sketches and an incremental improvement sequence. Research only: no product behavior or approved Phase 7 scope changes.

Open [index.html](index.html) directly, or serve it with `python3 -m http.server 8766 --bind 127.0.0.1 --directory docs/research/ui-ux-review` from the repository root. The self-contained page uses no external assets, analytics or live app connections. Search and area/priority filters, sketch selection, disclosure panels and fictional recovery states work locally. Print / PDF includes every finding and sketch regardless of the current filters.

Hosted at https://huggingface.co/spaces/Wauplin/launchpad-ui-ux-review with protected visibility. Protected keeps the Hub repository source private while the rendered website remains publicly accessible; it does not restrict report viewers. See [Hugging Face Space visibility](https://huggingface.co/docs/hub/spaces-overview#space-visibility). Only fictional examples and code references are published; live screenshots, credentials, environment values and personal transcripts remain excluded.

Create the Space with `hf repos create Wauplin/launchpad-ui-ux-review --type space --sdk static --protected`. Publish changes with `hf upload Wauplin/launchpad-ui-ux-review docs/research/ui-ux-review --type space --include index.html --include README.md --commit-message "Update Launchpad UI and UX review"`.

## Verification

The current app UI passed `pnpm build` (including TypeScript checking). The dev daemon was compiled and run without restarting production. A real-time Chrome walkthrough inspected Agents, an idle session, the launch drawer at 1440, 1280 and 800 px with the dev environment's 120% zoom, and Settings. The measured sidebar/drawer widths were 240/528 physical pixels; Agents' list adds 336, leaving 336 px for session content at 1440 and 176 px at 1280. At 800 px the main container itself shrinks to 32 px while the fixed agent list overflows it. This confirms the space-pressure finding; no product resize fix was implemented.

The report passed browser checks at 1440, 800 and 390 px: 28 findings, combined search/area filtering, filter reset, all five sketch selections, recovery-state switching, demo feedback, no horizontal document overflow at 800/390 px and no JavaScript page errors. Print media exposes all five sketches. Desktop and sketch screenshots were visually inspected; verification captures stay outside the published folder.

No new agent turns, terminals, schedules, suggestions or GitHub publications were created for checks. GitHub mutations, real posting/merge, harness handoff and billing were reviewed from code, not exercised. Native WebKitGTK, macOS, Windows, screen-reader behavior and daily-use acceptance remain unverified by this report.
