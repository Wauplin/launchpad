//! Named harness profiles (`[[profiles]]` in `config.toml`): environment, launch defaults and billing.
//!
//! Launch specs store only the profile ID; its settings are read again whenever a process starts,
//! so environment values (API keys) are never copied into `state.db`.
use crate::{
    App,
    config::{Billing, Profile},
    model::{HarnessKind, LaunchSpec, ModelInfo, ModelProvider},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;

/// The profile of a spec, checked against the harness. A missing profile is an error, never a fallback.
pub fn resolve(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> Result<Option<Profile>> {
    let Some(id) = &spec.profile else { return Ok(None) };
    let Some(v) = app.config.get().profile(id).cloned() else {
        bail!("the harness profile {id:?} no longer exists in Settings → Profiles");
    };
    if v.harness != harness {
        bail!("the harness profile {:?} is based on {}, not {}", v.name, v.harness.display(), harness.display());
    }
    Ok(Some(v))
}

/// Cost support for the selected model, including a profile's default model.
pub fn supports_cost(app: &App, harness: HarnessKind, spec: &LaunchSpec, model: Option<&str>) -> bool {
    let config = app.config.get();
    let profile = spec.profile.as_deref().and_then(|id| config.profile(id));
    crate::harness::registration(harness).descriptor.supports_cost(model.or(spec.model.as_deref()).or_else(|| profile.and_then(|p| p.model.as_deref())))
}

/// Environment overrides for processes of this spec, with `~/` and `${NAME}` references expanded.
/// `model` is the session's current model (the profile default applies when it is None) and `host`
/// the SSH host of the session folder.
pub fn env(app: &App, harness: HarnessKind, spec: &LaunchSpec, model: Option<&str>, host: Option<&str>) -> Result<Vec<(String, String)>> {
    let Some(v) = resolve(app, harness, spec)? else { return Ok(vec![]) };
    profile_env(app, &v, model.or(v.model.as_deref()), host)
}

/// The environment of a profile, with `~/` and `${NAME}` references expanded for the machine that runs it.
///
/// On an SSH host, `~/` is the host's home folder. `${LAUNCHPAD_MODEL}` is always resolved here; other
/// references are resolved here and sent only when the profile allows sending credentials, otherwise the
/// host helper resolves them from the host's own login shell (`host_values`).
pub fn profile_env(app: &App, v: &Profile, model: Option<&str>, host: Option<&str>) -> Result<Vec<(String, String)>> {
    let home = match host {
        Some(h) => Some(app.hosts.home(h).with_context(|| format!("{h} is not connected"))?),
        None => None,
    };
    v.env
        .iter()
        .map(|(key, value)| {
            let value = match (value.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
                (Some(_), None) => crate::paths::expand_home(value).to_string_lossy().into_owned(),
                (None, _) => value.clone(),
            };
            let value = expand(&value, |name| lookup(app, host.filter(|_| !v.send_credentials), model, name))
                .with_context(|| format!("{key} of the profile {:?}", v.name))?;
            Ok((key.clone(), value))
        })
        .collect()
}

/// Resolves the `${NAME}` references left for a host (see `profile_env`) from its login-shell environment,
/// with the same `HF_TOKEN` fallback as on this computer. Runs in the host helper.
pub fn host_values(login: &crate::shellenv::UserEnv, env: &[(String, String)]) -> Result<Vec<(String, String)>> {
    env.iter()
        .map(|(key, value)| {
            let value = expand(value, |name| {
                if let Some(v) = login.get(name).filter(|v| !v.is_empty()) {
                    return Ok(v.to_string());
                }
                if name == "HF_TOKEN" {
                    return hf_token(|k| login.get(k).map(str::to_string)).context("no Hugging Face token on this host: run hf auth login there, set HF_TOKEN in its login shell, or allow sending credentials in the profile");
                }
                bail!("${{{name}}} is not set in the login shell of this host; set it there, or allow sending credentials in the profile")
            })
            .with_context(|| format!("{key} of the profile environment"))?;
            Ok((key.clone(), value))
        })
        .collect()
}

/// The subscription login of a spec: the harness ID, plus each account variable (`account_env` of the
/// adapter, such as `CLAUDE_CONFIG_DIR`) that its profile sets, unexpanded. Profiles with the same key
/// share one quota monitor. Specs without a profile use the harness's default login.
pub fn account(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> String {
    match spec.profile.as_deref().and_then(|id| app.config.get().profile(id).cloned()).filter(|p| p.harness == harness) {
        Some(p) => account_of(&p),
        None => harness.as_str().to_string(),
    }
}

/// The command that signs a profile in, with its account variables (never credentials) as a prefix,
/// for example `CLAUDE_CONFIG_DIR=~/.claude-work claude auth login`.
pub fn sign_in_command(app: &App, p: &Profile) -> String {
    let vars = crate::harness::registration(p.harness).descriptor.account_env;
    let mut parts: Vec<String> = vars.iter().filter_map(|name| p.env.get(*name).map(|v| format!("{name}={v}"))).collect();
    parts.push(app.harnesses.get(p.harness).sign_in_command().to_string());
    parts.join(" ")
}

pub fn account_of(p: &Profile) -> String {
    let vars = crate::harness::registration(p.harness).descriptor.account_env;
    let mut key = p.harness.as_str().to_string();
    for name in vars {
        if let Some(value) = p.env.get(*name).filter(|v| !v.trim().is_empty()) {
            key.push_str(&format!(" {name}={value}"));
        }
    }
    key
}

/// Names referenced as `${NAME}` in a profile value; malformed references are errors.
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
/// this computer, and `HF_TOKEN` falls back to the token saved by `hf auth login`. With `host` set,
/// references are left for the host helper, so local credentials are not copied there implicitly.
fn lookup(app: &App, host: Option<&str>, model: Option<&str>, name: &str) -> Result<String> {
    if name == "LAUNCHPAD_MODEL" {
        return model.map(str::to_string).context("${LAUNCHPAD_MODEL} needs a model: choose one in the composer or a default model for the profile");
    }
    if host.is_some() {
        return Ok(format!("${{{name}}}"));
    }
    if let Some(v) = app.env.get(name).filter(|v| !v.is_empty()) {
        return Ok(v.to_string());
    }
    if name == "HF_TOKEN" {
        return hf_token(|k| app.env.get(k).map(str::to_string)).context("no Hugging Face token: run hf auth login, or set HF_TOKEN");
    }
    bail!("${{{name}}} is not set in your login shell environment")
}

/// The token file of `huggingface_hub`: `HF_TOKEN_PATH`, else `$HF_HOME/token`, else `~/.cache/huggingface/token`.
fn hf_token(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    let path = match (get("HF_TOKEN_PATH"), get("HF_HOME"), get("XDG_CACHE_HOME")) {
        (Some(p), _, _) => crate::paths::expand_home(&p),
        (None, Some(home), _) => crate::paths::expand_home(&home).join("token"),
        (None, None, Some(cache)) => std::path::Path::new(&cache).join("huggingface/token"),
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
                return Some(ModelInfo { id, display_name: String::new(), description: String::new(), efforts: vec![], providers: vec![] });
            };
            let usable: Vec<&Value> = providers.iter().filter(|p| p["status"].as_str().is_none_or(|s| s == "live") && p["supports_tools"].as_bool() != Some(false)).collect();
            if usable.is_empty() {
                return None;
            }
            let providers: Vec<ModelProvider> = usable.iter().filter_map(|p| {
                let pricing = &p["pricing"];
                let price = |keys: &[&str]| keys.iter().find_map(|k| pricing[*k].as_f64()).filter(|v| v.is_finite() && *v >= 0.0);
                Some(ModelProvider {
                    name: p["provider"].as_str()?.to_string(),
                    context_length: p["context_length"].as_u64(),
                    input_usd: price(&["input"]),
                    cached_input_usd: price(&["cached_input", "input_cache_read", "cache_read", "cached"]),
                    output_usd: price(&["output"]),
                    throughput: p["throughput"].as_f64(),
                    first_token_ms: p["first_token_latency_ms"].as_f64(),
                })
            }).collect();
            let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
            // `auto` routing may pick any provider, so the smallest window is the safe one.
            let context = providers.iter().filter_map(|p| p.context_length).min();
            let summary = format!("{} provider{}: {}", names.len(), if names.len() == 1 { "" } else { "s" }, names.join(", "));
            let description = match context {
                Some(c) => format!("{}K context · {summary}", c / 1000),
                None => summary,
            };
            Some(ModelInfo { id, display_name: String::new(), description, efforts: vec![], providers })
        })
        .collect();
    anyhow::ensure!(!models.is_empty(), "the list has no usable models");
    Ok(models)
}

/// Billed per use: subscription quota protection does not apply.
pub fn pay_as_you_go(app: &App, spec: &LaunchSpec) -> bool {
    spec.profile
        .as_deref()
        .and_then(|id| app.config.get().profile(id).map(|v| v.billing == Billing::PayAsYouGo))
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

    #[test]
    fn accounts_follow_the_login_folder_and_hosts_resolve_their_own_references() {
        let profile = |env: &[(&str, &str)]| Profile {
            id: "p".into(), name: "P".into(), harness: crate::harness::claude::ID,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            model: None, effort: None, billing: Billing::Subscription, budget_usd: None, models_url: None, spend_limits: vec![], send_credentials: false,
        };
        assert_eq!(account_of(&profile(&[])), "claude");
        // Variables other than the login folder do not change the account.
        assert_eq!(account_of(&profile(&[("DISABLE_TELEMETRY", "1")])), "claude");
        assert_eq!(account_of(&profile(&[("CLAUDE_CONFIG_DIR", "~/.claude-work")])), "claude CLAUDE_CONFIG_DIR=~/.claude-work");
        let login = crate::shellenv::UserEnv::for_host("h").with(&[("TOKEN".into(), "abc".into())]);
        let env = |v: &str| vec![("KEY".to_string(), v.to_string())];
        assert_eq!(host_values(&login, &env("Bearer ${TOKEN}")).unwrap(), env("Bearer abc"));
        assert!(host_values(&login, &env("${MISSING}")).is_err());
    }
}
