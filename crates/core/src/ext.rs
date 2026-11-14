//! Extension points: the few calls the core makes to the crates above it (connections, GitHub features,
//! automation), and the state those crates keep per daemon. The binary installs one `Extensions` at startup;
//! until then, and in unit tests, every call does nothing.
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;

use crate::App;
use crate::model::Session;

#[async_trait]
pub trait Extensions: Send + Sync {
    /// A session notification for the Telegram chat of the session's workspace (`notify::session`). `event`: the event's name.
    fn notify_connections(&self, _app: &App, _session: &Session, _event: &'static str) {}
    /// A turn ended: routine run records and `session_done` triggers (`crons::turn_ended`).
    fn turn_ended(&self, _app: &App, _session: &Session) {}
    /// Home: the work of a session's last turn reached Home (its `unsaved` mark cleared after the turn ended).
    fn turn_saved(&self, _app: &App, _session: &Session) {}
    /// Home: a sleeping session started a turn before its wake-up (a message woke it): its wake-up `schedule_id` goes
    /// (`schedules::woken`).
    fn woken(&self, _app: &App, _session_id: &str, _schedule_id: &str) {}
    /// Instructions added to a session's initial prompt about agent-written GitHub text and connections.
    fn suggestion_instruction(&self, _app: &App, _session_id: &str) -> Option<String> {
        None
    }
    /// A bot was paused or resumed: its routines follow.
    async fn bot_paused(&self, _app: &App, _bot_id: &str, _paused: bool) -> Result<()> {
        Ok(())
    }
    /// Home: a bot's files changed (an owner save, Restore, an accepted card): its routines follow
    /// `routines/*.toml`.
    async fn bot_files_changed(&self, _app: &App, _bot_id: &str) -> Result<()> {
        Ok(())
    }
    /// A bot was deleted: its routines go with it.
    async fn bot_deleted(&self, _app: &App, _bot_id: &str) -> Result<()> {
        Ok(())
    }
    /// A bot channel card (`ProposalBody::Channel` or `ChannelJoin` as JSON, with its `kind`) of `workspace`: checked
    /// when it is filed (`apply` false: the card with IDs and names filled in) or applied when it is accepted
    /// (creates the channel or adds the member: `{"id": <channel ID>}`). `bot`: the author's bot, which a join card
    /// always names. `by`: who created it (`channel.created_by`).
    async fn channel_card(&self, _app: &App, _workspace: &str, _card: serde_json::Value, _bot: Option<&str>, _apply: bool, _by: &str) -> Result<serde_json::Value> {
        anyhow::bail!("bot channels are not available here")
    }
    /// Home: the lines of a pull request's diff a selection names (`DiffSelection::lines`), read from GitHub at
    /// delivery, with a note when the pull request moved since; None when they cannot be read.
    async fn selection_lines(&self, _app: &App, _selection: &crate::model::DiffSelection) -> Option<String> {
        None
    }
    /// The Today items of the crates above the core (proposals, Outbox drafts, suggestions, failed
    /// routines), waiting and handled (`today::Item::handled`).
    async fn today(&self, _app: &App) -> Vec<crate::today::Item> {
        vec![]
    }
}

struct None_;
impl Extensions for None_ {}

static EXTENSIONS: OnceLock<Box<dyn Extensions>> = OnceLock::new();

/// Called once by the binary at startup, before `App::start`.
pub fn install(extensions: Box<dyn Extensions>) {
    let _ = EXTENSIONS.set(extensions);
}

pub fn extensions() -> &'static dyn Extensions {
    EXTENSIONS.get().map_or(&None_, |e| e.as_ref())
}

/// Per-daemon state of the crates above the core, one value per type, created on first use.
#[derive(Clone, Default)]
pub struct Extras(Arc<Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>);

impl Extras {
    pub fn get<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let mut map = self.0.lock().unwrap();
        let value = map.entry(TypeId::of::<T>()).or_insert_with(|| Arc::new(T::default())).clone();
        value.downcast().expect("extra state keyed by its type")
    }
}
