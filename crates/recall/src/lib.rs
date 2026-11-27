//! Recall: a search index over the past sessions of each workspace, built on Home with the `funes` program
//! (huggingface/funes). It is derived evidence beside the knowledge base, bot memory, BOT.md and repository
//! instructions, which stay authoritative: everything here can be deleted and built again from `state.db`.
//!
//! - `convert`: normalized session events → Funes turns.
//! - `ledger`: what each workspace's memory holds (cursors, turn map, omissions).
//! - `funes`: the helper program, its environment and its output.
//! - `index`: the indexer after turns, the backfill and the rebuild.
//! - `query`: `launchpad recall`, `recall get`, `recall status`.
//!
//! Off by default (`home.recall.enabled`). Rules: docs/invariants/recall.md.
pub mod convert;
pub mod funes;
pub mod index;
pub mod ledger;
pub mod query;

use launchpad_core::App;
use launchpad_core::model::Session;

/// A turn ended, or its work reached Home (`ext::Extensions::turn_ended`, `turn_saved`).
pub fn turn_finished(app: &App, session: &Session) {
    index::mark(app, session);
}

/// Home: starts the indexer. It never blocks or fails Home's start.
pub fn spawn(app: &App) {
    index::spawn(app);
}
