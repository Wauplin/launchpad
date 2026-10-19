//! Named harness variants (`[[variants]]` in `config.toml`): environment, launch defaults and billing.
//!
//! Launch specs store only the variant ID; its settings are read again whenever a process starts,
//! so environment values (API keys) are never copied into `state.db`.
use crate::{
    App,
    config::{Billing, HarnessVariant},
    model::{HarnessKind, LaunchSpec, ModelInfo},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;

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

/// Environment overrides for processes of this spec, with `~/` and `${NAME}` references expanded.
/// `model` is the session's current model (the variant default applies when it is None) and `host`
/// the SSH host of the session folder.
pub fn env(app: &App, harness: HarnessKind, spec: &LaunchSpec, model: Option<&str>, host: Option<&str>) -> Result<Vec<(String, String)>> {
    let Some(v) = resolve(app, harness, spec)? else { return Ok(vec![]) };
    variant_env(app, &v, model.or(v.model.as_deref()), host)
}

/// The environment of a variant, with `~/` and `${NAME}` references expanded.
pub fn variant_env(app: &App, v: &HarnessVariant, model: Option<&str>, host: Option<&str>) -> Result<Vec<(String, String)>> {
    v.env
        .iter()
        .map(|(key, value)| {
            let value = if value.starts_with("~/") { crate::paths::expand_home(value).to_string_lossy().into_owned() } else { value.clone() };
            let value = expand(&value, |name| lookup(app, host, model, name))
                .with_context(|| format!("{key} of the harness variant {:?}", v.name))?;
            Ok((key.clone(), value))
        })
        .collect()
}

/// Names referenced as `${NAME}` in a variant value; malformed references are errors.
pub fn references(value: &str) -> Result<Vec<&str>> {
    let mut names = vec![];
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let end = after.find('}').context("a ${ reference is not closed with }")?;
        let name = &after[..end];
        anyhow::ensure!(
            !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "invalid reference ${{{name}}}"
        );
        names.push(name);
        rest = &after[end + 1..];
    }
    Ok(names)
}

fn expand(value: &str, mut lookup: impl FnMut(&str) -> Result<String>) -> Result<String> {
    let mut out = String::new();
    let mut rest = value;
    for name in references(value)? {
        let token = format!("${{{name}}}");
        let start = rest.find(&token).expect("reference found by references()");
        out.push_str(&rest[..start]);
        out.push_str(&lookup(name)?);
        rest = &rest[start + token.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `${LAUNCHPAD_MODEL}` is the session model; other names come from the login shell environment of
/// this computer, and `HF_TOKEN` falls back to the token saved by `hf auth login`. Never resolved for
/// SSH hosts, so local credentials are not copied there implicitly.
fn lookup(app: &App, host: Option<&str>, model: Option<&str>, name: &str) -> Result<String> {
    if name == "LAUNCHPAD_MODEL" {
        return model.map(str::to_string).context("${LAUNCHPAD_MODEL} needs a model: choose one in the composer or a default model for the variant");
    }
    if let Some(host) = host {
        bail!("${{{name}}} is resolved only for agents on this computer; enter the value itself to use this variant on {host}");
    }
    if let Some(v) = app.env.get(name).filter(|v| !v.is_empty()) {
        return Ok(v.to_string());
    }
    if name == "HF_TOKEN" {
        return hf_token(app).context("no Hugging Face token: run hf auth login, or set HF_TOKEN");
    }
    bail!("${{{name}}} is not set in your login shell environment")
}

/// The token file of `huggingface_hub`: `HF_TOKEN_PATH`, else `$HF_HOME/token`, else `~/.cache/huggingface/token`.
fn hf_token(app: &App) -> Option<String> {
    let path = match (app.env.get("HF_TOKEN_PATH"), app.env.get("HF_HOME"), app.env.get("XDG_CACHE_HOME")) {
        (Some(p), _, _) => crate::paths::expand_home(p),
        (None, Some(home), _) => crate::paths::expand_home(home).join("token"),
        (None, None, Some(cache)) => std::path::Path::new(cache).join("huggingface/token"),
        _ => dirs::home_dir()?.join(".cache/huggingface/token"),
    };
    let token = std::fs::read_to_string(path).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Models of an OpenAI-compatible `/v1/models` list. With per-provider details (Hugging Face router),
/// only models that a live provider serves with tool use are kept, since coding agents need tools.
pub async fn models(url: &str) -> Result<Vec<ModelInfo>> {
    anyhow::ensure!(reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")), "the model list must be an HTTP(S) URL");
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
    let body: Value = client.get(url).send().await?.error_for_status()?.json().await?;
    let list = body["data"].as_array().context("the response has no data list")?;
    let models: Vec<ModelInfo> = list
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            let Some(providers) = m["providers"].as_array() else {
                return Some(ModelInfo { id, display_name: String::new(), description: String::new(), efforts: vec![] });
            };
            let usable: Vec<&Value> = providers.iter().filter(|p| p["status"].as_str().is_none_or(|s| s == "live") && p["supports_tools"].as_bool() != Some(false)).collect();
            if usable.is_empty() {
                return None;
            }
            let names: Vec<&str> = usable.iter().filter_map(|p| p["provider"].as_str()).collect();
            // `auto` routing may pick any provider, so the smallest window is the safe one.
            let context = usable.iter().filter_map(|p| p["context_length"].as_u64()).min();
            let providers = format!("{} provider{}: {}", names.len(), if names.len() == 1 { "" } else { "s" }, names.join(", "));
            let description = match context {
                Some(c) => format!("{}K context · {providers}", c / 1000),
                None => providers,
            };
            Some(ModelInfo { id, display_name: String::new(), description, efforts: vec![] })
        })
        .collect();
    anyhow::ensure!(!models.is_empty(), "the list has no usable models");
    Ok(models)
}

/// Billed per use: subscription quota protection does not apply.
pub fn pay_as_you_go(app: &App, spec: &LaunchSpec) -> bool {
    spec.variant
        .as_deref()
        .and_then(|id| app.config.get().variant(id).map(|v| v.billing == Billing::PayAsYouGo))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_expand_in_order_and_reject_malformed_names() {
        let names = |v| references(v).map_err(|e| e.to_string());
        assert_eq!(names("${A}-${B_2}/${A}").unwrap(), ["A", "B_2", "A"]);
        assert_eq!(names("$HOME and $ alone").unwrap(), Vec::<&str>::new());
        assert!(names("${A").is_err() && names("${}").is_err() && names("${A-B}").is_err() && names("${1A}").is_err());
        let out = expand("x${A}y${B}z${A}", |n| Ok(n.to_lowercase())).unwrap();
        assert_eq!(out, "xaybza");
    }
}
