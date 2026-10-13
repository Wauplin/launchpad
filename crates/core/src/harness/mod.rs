//! Registration and the common contract. Native protocols and optional services belong to adapters.
pub mod acp;
pub mod claude;
pub mod codex;
pub mod opencode;

use crate::{
    App,
    model::{HarnessDefaults, HarnessKind, ModelInfo, Session},
    sessions::{LiveSession, Sink, StartSpec},
    shellenv::UserEnv,
};
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::process::Command;
use ts_rs::TS;

#[derive(Clone, Copy, Serialize, TS)]
pub struct HarnessCapabilities {
    pub images: bool,
    /// Native steering or queuing while a turn runs.
    pub messages_while_running: bool,
    pub terminal: bool,
    /// Included subscription quotas can be monitored by this integration.
    pub quota: bool,
    /// Reports the cumulative USD cost of a session, so cost limits can apply.
    pub cost: bool,
}

#[derive(Clone, Copy, Serialize, TS)]
pub struct HarnessDescriptor {
    pub id: HarnessKind,
    pub name: &'static str,
    pub icon_svg: &'static str,
    pub color: &'static str,
    pub working_message: &'static str,
    pub capabilities: HarnessCapabilities,
}

pub struct Registration {
    pub descriptor: HarnessDescriptor,
    pub create: fn(&UserEnv) -> Arc<dyn Harness>,
    pub environment_marker: fn(&str) -> bool,
}

/// Add an adapter here and declare its module above; no host dispatch needs to change.
pub const REGISTRATIONS: &[Registration] = &[
    claude::REGISTRATION,
    codex::REGISTRATION,
    opencode::REGISTRATION,
];

pub fn registration(id: HarnessKind) -> &'static Registration {
    REGISTRATIONS
        .iter()
        .find(|r| r.descriptor.id == id)
        .expect("registered harness ID")
}

#[async_trait]
pub trait Harness: Send + Sync {
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>>;
    async fn models(&self, _app: &App) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    fn defaults(&self) -> (Option<String>, Option<String>) {
        (None, None)
    }
    fn efforts(&self) -> Vec<String> {
        vec![]
    }
    async fn version(&self, app: &App) -> Result<String>;
    /// Program whose presence on `PATH` means the CLI is installed.
    fn sign_in_program(&self) -> &'static str;
    /// Command the owner runs once on the daemon host to sign in.
    fn sign_in_command(&self) -> &'static str;
    /// Read-only sign-in check with this environment: never signs in, refreshes a token or returns secrets.
    async fn sign_in(&self, _env: &UserEnv) -> crate::signin::SignIn {
        crate::signin::SignIn { signed_in: None, detail: None }
    }
    fn spawn_services(&self, _app: &App) {}
    fn terminal_command(&self, _session: &Session, _is_temp: bool) -> Result<Vec<String>> {
        bail!("terminal handoff is not supported")
    }
    fn terminal_env(&self, _is_temp: bool) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    fn project_moved(&self, _old: &str, _new: &str) {}
    fn terminal_resume_markers(&self) -> &'static [&'static str] {
        &[]
    }
    fn title_command(&self, _app: &App, _session: &Session, _cwd: &Path) -> Option<Command> {
        None
    }
    fn title_response(&self, stdout: String) -> Result<String> {
        Ok(stdout)
    }
}

#[derive(Clone)]
pub struct Registry(Arc<HashMap<HarnessKind, Arc<dyn Harness>>>);
impl Registry {
    pub fn new(env: &UserEnv) -> Self {
        let mut entries = HashMap::new();
        for registration in REGISTRATIONS {
            assert!(
                entries
                    .insert(registration.descriptor.id, (registration.create)(env))
                    .is_none(),
                "duplicate harness registration"
            );
        }
        Self(Arc::new(entries))
    }
    pub fn get(&self, id: HarnessKind) -> &Arc<dyn Harness> {
        self.0.get(&id).expect("registered harness ID")
    }
    pub fn descriptors(&self) -> Vec<HarnessDescriptor> {
        REGISTRATIONS.iter().map(|r| r.descriptor).collect()
    }
    pub fn spawn_services(&self, app: &App) {
        for registration in REGISTRATIONS {
            self.get(registration.descriptor.id).spawn_services(app);
        }
    }
}

#[derive(Clone, Default)]
pub struct ModelCache {
    inner: Arc<RwLock<HashMap<HarnessKind, Vec<ModelInfo>>>>,
    refresh_locks: Arc<std::sync::Mutex<HashMap<HarnessKind, Arc<tokio::sync::Mutex<()>>>>>,
}
impl ModelCache {
    /// Query this adapter again, preserving the last successful list on failure.
    pub async fn refresh(&self, app: &App, id: HarnessKind) -> Result<HarnessDefaults> {
        let lock = self.refresh_locks.lock().unwrap().entry(id).or_default().clone();
        let _guard = lock.lock().await;
        let models = tokio::time::timeout(
            Duration::from_secs(60), app.harnesses.get(id).models(app),
        ).await??;
        anyhow::ensure!(!models.is_empty(), "no models returned by {}", id.as_str());
        self.inner.write().unwrap().insert(id, models);
        Ok(self.defaults(app, id))
    }

    pub fn defaults(&self, app: &App, id: HarnessKind) -> HarnessDefaults {
        let adapter = app.harnesses.get(id);
        let (model, effort) = adapter.defaults();
        HarnessDefaults {
            model,
            effort,
            models: self
                .inner
                .read()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_default(),
            efforts: adapter.efforts(),
        }
    }
}

pub fn spawn_model_refresh(app: &App) {
    // An unavailable adapter must not block discovery for the others.
    for registration in REGISTRATIONS {
        let id = registration.descriptor.id;
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            loop {
                if let Err(error) = app.models.refresh(&app, id).await {
                    tracing::info!("{} model discovery: {error:#}", id.as_str());
                }
                tokio::time::sleep(Duration::from_secs(30 * 60)).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_ids_round_trip_without_falling_back_to_another_adapter() {
        let mut ids = std::collections::HashSet::new();
        for registration in REGISTRATIONS {
            let id = registration.descriptor.id;
            assert!(ids.insert(id));
            assert_eq!(serde_json::from_value::<HarnessKind>(serde_json::json!(id)).unwrap(), id);
            assert_eq!(serde_json::to_value(id).unwrap(), serde_json::json!(id.as_str()));
        }
        assert!(serde_json::from_str::<HarnessKind>("\"unregistered\"").is_err());
    }
}
