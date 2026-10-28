//! Onboarding progress (`docs/ARCHITECTURE.md`, section 17): one row per daemon in `state.db`, so a second
//! browser does not show the welcome again. It records the welcome answer, the tour chapters seen and the
//! dismissed onboarding items (banner, setup checklist entries). It is the only thing the demo mode writes.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct Onboarding {
    /// The welcome answer: `tour`, `skip` or `later`; null while it was never shown or answered.
    pub welcome: Option<String>,
    /// Tour chapter IDs played to their end at least once.
    pub chapters_seen: Vec<String>,
    /// Dismissed onboarding items: `banner`, `setup` (the sidebar Finish setup entry) or `setup:<item>`.
    pub dismissed: Vec<String>,
}

/// The onboarding record plus whether this daemon looks like a fresh install.
#[derive(Clone, Debug, Serialize, TS)]
pub struct OnboardingState {
    #[serde(flatten)]
    #[ts(flatten)]
    pub record: Onboarding,
    /// No folder, no profile, no session and no onboarding record: the welcome dialog opens.
    pub fresh_install: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Patch {
    pub welcome: Option<String>,
    /// Added to `chapters_seen`.
    pub chapters_seen: Vec<String>,
    /// Added to `dismissed`.
    pub dismiss: Vec<String>,
    /// Removed from `dismissed`.
    pub restore: Vec<String>,
}

async fn read(app: &App) -> Result<Option<Onboarding>> {
    app.state_db
        .call(|c| {
            let json: Option<String> = c.query_row("SELECT json FROM onboarding WHERE id = 1", [], |r| r.get(0)).optional()?;
            Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
        })
        .await
}

pub async fn get(app: &App) -> Result<OnboardingState> {
    let record = read(app).await?;
    let fresh_install = record.is_none() && app.config.get().profiles.is_empty() && app
        .state_db
        .call(|c| Ok(c.query_row("SELECT NOT EXISTS(SELECT 1 FROM project) AND NOT EXISTS(SELECT 1 FROM session)", [], |r| r.get::<_, bool>(0))?))
        .await?;
    Ok(OnboardingState { record: record.unwrap_or_default(), fresh_install })
}

pub async fn patch(app: &App, p: Patch) -> Result<OnboardingState> {
    let mut o = read(app).await?.unwrap_or_default();
    if let Some(w) = p.welcome {
        anyhow::ensure!(["tour", "skip", "later"].contains(&w.as_str()), "unknown welcome answer");
        o.welcome = Some(w);
    }
    let add = |list: &mut Vec<String>, items: Vec<String>| {
        for i in items {
            if !i.is_empty() && i.len() <= 80 && !list.contains(&i) {
                list.push(i);
            }
        }
    };
    add(&mut o.chapters_seen, p.chapters_seen);
    add(&mut o.dismissed, p.dismiss);
    o.dismissed.retain(|d| !p.restore.contains(d));
    let json = serde_json::to_string(&o)?;
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO onboarding (id, json) VALUES (1, ?1) ON CONFLICT (id) DO UPDATE SET json = ?1", [json])?;
            Ok(())
        })
        .await?;
    app.bus.publish("onboarding", "changed", serde_json::Value::Null);
    Ok(OnboardingState { record: o, fresh_install: false })
}
