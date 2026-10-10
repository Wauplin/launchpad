//! Named harness variants (`[[variants]]` in `config.toml`): environment, launch defaults and billing.
//!
//! Launch specs store only the variant ID; its settings are read again whenever a process starts,
//! so environment values (API keys) are never copied into `state.db`.
use crate::{
    App,
    config::{Billing, HarnessVariant},
    model::{HarnessKind, LaunchSpec},
};
use anyhow::{Result, bail};

/// The variant of a spec, checked against the harness. A missing variant is an error, never a fallback.
pub fn resolve(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> Result<Option<HarnessVariant>> {
    let Some(id) = &spec.variant else { return Ok(None) };
    let Some(v) = app.config.get().variant(id).cloned() else {
        bail!("the harness variant {id:?} no longer exists in Settings → Harnesses");
    };
    if v.harness != harness {
        bail!("the harness variant {:?} is based on {}, not {}", v.name, v.harness.display(), harness.display());
    }
    Ok(Some(v))
}

/// Environment overrides for processes of this spec, with `~/` expanded.
pub fn env(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> Result<Vec<(String, String)>> {
    Ok(resolve(app, harness, spec)?
        .map(|v| {
            v.env
                .into_iter()
                .map(|(k, v)| {
                    let v = if v.starts_with("~/") { crate::paths::expand_home(&v).to_string_lossy().into_owned() } else { v };
                    (k, v)
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Billed per use: subscription quota protection does not apply.
pub fn pay_as_you_go(app: &App, spec: &LaunchSpec) -> bool {
    spec.variant
        .as_deref()
        .and_then(|id| app.config.get().variant(id).map(|v| v.billing == Billing::PayAsYouGo))
        .unwrap_or(false)
}
