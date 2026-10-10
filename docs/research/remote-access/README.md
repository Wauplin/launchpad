---
title: Launchpad beyond the laptop
emoji: 🚀
colorFrom: green
colorTo: blue
sdk: static
app_file: index.html
pinned: false
fullWidth: true
header: mini
short_description: "Launchpad on VPS, web, mobile and Android: a field guide"
---

# Launchpad remote access field guide

Research and proposal only, prepared on 2026-10-10 against Launchpad commit `3100c0f`. No remote access, mobile application, Android client or Phase 7 feature was implemented. The report includes repository evidence, primary documentation links, deployment comparisons, failure scenarios, interactive concept sketches and an incremental acceptance plan.

Open [index.html](index.html) directly, or serve this directory with `python3 -m http.server 8765 --bind 127.0.0.1`. The self-contained page uses no external assets or live API connections. The Print / save PDF button provides a reading copy.

Hosted at https://huggingface.co/spaces/Wauplin/launchpad-remote-field-guide with protected visibility. Protected keeps the Hub repository source private while the rendered page remains publicly accessible; it is not an access-control mechanism for a Launchpad daemon.

Publish changes using the authenticated HF CLI: `hf upload Wauplin/launchpad-remote-field-guide docs/research/remote-access --type space --include index.html --include README.md --commit-message "Update remote access research"`. Creating the original Space used `hf repos create Wauplin/launchpad-remote-field-guide --type space --sdk static --protected`.
