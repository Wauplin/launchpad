//! The JSON API routes that need only the App and the front door's state. `launchpad-server` serves them behind
//! its authentication, relay and forwarding layers.

mod activity;
mod api;
mod bots;
mod channels;
mod code;
mod connections;
mod diagnostics;
mod ext;
mod items;
mod kb_http;
mod mcp;
mod predrafts;
mod prompt_context;
mod proposals;
mod review_state;
mod setup;
mod suggestions;
mod today;
mod vault;

use axum::Router;
use launchpad_http::{ApiError, Ctx, agent_auth, error};

/// Every route of this crate.
pub fn router() -> Router<Ctx> {
    Router::new()
        .merge(api::router())
        .merge(code::router())
        .merge(items::router())
        .merge(predrafts::router())
        .merge(review_state::router())
        .merge(suggestions::router())
        .merge(proposals::router())
        .merge(bots::router())
        .merge(channels::router())
        .merge(connections::router())
        .merge(prompt_context::router())
        .merge(setup::router())
        .merge(mcp::router())
        .merge(vault::router())
        .merge(kb_http::router())
        .merge(today::router())
        .merge(activity::router())
        .merge(diagnostics::router())
        .merge(ext::router())
}
