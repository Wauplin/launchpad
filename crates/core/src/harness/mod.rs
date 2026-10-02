//! Harness adapters, and what the composer needs to know about each harness (models, defaults).

pub mod claude;
pub mod codex;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::App;
use crate::model::{HarnessDefaults, HarnessKind, ModelInfo};

/// Effort levels accepted by each CLI (`claude --help`, Codex `model/list`), used when a model is unknown.
pub fn efforts(h: HarnessKind) -> Vec<String> {
    let v: &[&str] = match h {
        HarnessKind::Claude => &["low", "medium", "high", "xhigh", "max"],
        HarnessKind::Codex => &["low", "medium", "high", "xhigh", "max"],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// Model lists, refreshed in the background (reading them takes a process start).
#[derive(Clone, Default)]
pub struct ModelCache {
    inner: Arc<RwLock<HashMap<HarnessKind, Vec<ModelInfo>>>>,
}

impl ModelCache {
    pub fn defaults(&self, h: HarnessKind) -> HarnessDefaults {
        let (model, effort) = match h {
            HarnessKind::Claude => claude::defaults(),
            HarnessKind::Codex => codex::defaults(),
        };
        HarnessDefaults { model, effort, models: self.inner.read().unwrap().get(&h).cloned().unwrap_or_default(), efforts: efforts(h) }
    }
}

pub fn spawn_model_refresh(app: &App) {
    let a = app.clone();
    tokio::spawn(async move {
        // Let the daemon finish starting first.
        tokio::time::sleep(Duration::from_secs(2)).await;
        loop {
            match claude::models(&a).await {
                Ok(m) => {
                    a.models.inner.write().unwrap().insert(HarnessKind::Claude, m);
                }
                Err(e) => tracing::info!("claude models: {e:#}"),
            }
            match codex::models(&a).await {
                Ok(m) => {
                    a.models.inner.write().unwrap().insert(HarnessKind::Codex, m);
                }
                Err(e) => tracing::info!("codex models: {e:#}"),
            }
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
}

/// Updates Codex in the background at startup and every 6 hours. Running sessions keep the binary
/// they started with; headless modes never show the update prompt.
pub fn spawn_codex_updates(app: &App) {
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            a.status.update(|s| s.codex.updating_to = Some("latest".into()));
            let res = crate::procs::command(&a.env, "codex", None)
                .args(["update"])
                .stdin(std::process::Stdio::null())
                .output()
                .await;
            match res {
                Ok(out) => {
                    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                    tracing::info!("codex update ({}): {}", out.status, text.trim().lines().last().unwrap_or(""));
                }
                Err(e) => tracing::warn!("codex update: {e}"),
            }
            a.status.update(|s| s.codex.updating_to = None);
            crate::status::refresh_versions(&a).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}
