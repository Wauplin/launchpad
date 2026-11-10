//! Shared pieces of the HTTP crates (`launchpad-server`, `launchpad-api`): the error type, the state of the API
//! routes and session token authorization.

pub mod agent_auth;
pub mod error;
pub mod logs;

use std::sync::Arc;

use launchpad_core::App;

pub use error::{ApiError, ApiResult};

/// The server's front door as the API routes see it: whether browsers come from a remote origin, and which one.
/// Both change when the owner activates HTTPS.
pub trait FrontDoor: Send + Sync {
    fn remote(&self) -> bool;
    fn origin(&self) -> String;
}

/// The state of the routes in `launchpad-api`.
#[derive(Clone)]
pub struct Ctx {
    pub app: App,
    pub auth: Arc<dyn FrontDoor>,
}

/// Header of the requests Home forwards to a runner.
pub const FROM_HOME: &str = "x-launchpad-from-home";

/// Paths that authenticate with their own secret instead of the browser sign-in.
pub fn hook_path(path: &str) -> bool {
    path.strip_prefix("/hooks/").is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// Home: the agent session that sent this request (a request extension), from Home's own records: a session CLI
/// token checked on Home, or a runner's tunnel request that names one of the sessions Home placed on that runner.
/// Messages it sends are stamped as its (`launchpad_core::senders`), never as the owner's.
#[derive(Clone, Debug)]
pub struct AgentSession(pub String);
