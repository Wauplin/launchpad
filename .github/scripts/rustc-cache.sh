#!/usr/bin/env bash
set -euo pipefail

# rust-embed discovers the UI's file list in a procedural macro. Bypass the
# compiler cache for this crate so newly added/deleted assets are always seen.
previous=''
for argument in "$@"; do
  if [[ "$previous" == --crate-name && "$argument" == launchpad_server ]]; then
    exec "$@"
  fi
  previous="$argument"
done

exec sccache "$@"
