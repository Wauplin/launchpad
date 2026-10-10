---
title: Launchpad UX suggestions
emoji: 🧭
colorFrom: green
colorTo: gray
sdk: static
pinned: false
---

# Launchpad UX suggestions (October 2026)

A standalone UI/UX review of Launchpad at commit `05f96f5`, focused on suggestions: what to change so the most important features are easy to reach without making the UI heavier. It includes interactive sketches built with fake data. It does not contain live screenshots, credentials, environment values or personal transcripts.

This review was written from scratch: it does not reuse `docs/research/ui-ux-review/`.

## Preview locally

```sh
python3 -m http.server -d docs/research/ux-suggestions 8765
```

Then open http://127.0.0.1:8765.

## Publish

The report is hosted in the protected static Space `Wauplin/launchpad-ux-suggestions`. Protected means the repository files are private, but anyone with the URL can open the rendered page: https://wauplin-launchpad-ux-suggestions.static.hf.space.

```sh
hf repos create Wauplin/launchpad-ux-suggestions --type space --space-sdk static --protected --exist-ok
hf upload Wauplin/launchpad-ux-suggestions docs/research/ux-suggestions . --repo-type space
```
