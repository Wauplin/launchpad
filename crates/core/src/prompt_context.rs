//! Read-only, bounded composer discovery. Native locations belong to harness adapters.
use crate::{
    App, hostfs,
    hosts::loc,
    model::{HarnessKind, LaunchSpec, SkillRef},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use ts_rs::TS;

#[derive(Clone, Copy)]
pub struct SkillLocation {
    pub project: bool,
    pub config: Option<&'static str>,
    pub base: &'static str,
    pub suffix: &'static str,
}
impl SkillLocation {
    pub const fn project(base: &'static str) -> Self {
        Self {
            project: true,
            config: None,
            base,
            suffix: "",
        }
    }
    pub const fn home(base: &'static str) -> Self {
        Self {
            project: false,
            config: None,
            base,
            suffix: "",
        }
    }
    pub const fn config(config: &'static str, base: &'static str, suffix: &'static str) -> Self {
        Self {
            project: false,
            config: Some(config),
            base,
            suffix,
        }
    }
}
#[derive(Clone, Serialize, TS)]
pub struct PathHit {
    pub project_id: String,
    pub project: String,
    pub root: String,
    pub path: String,
    pub directory: bool,
}
#[derive(Serialize, TS)]
pub struct PathResults {
    pub hits: Vec<PathHit>,
    pub unavailable: Vec<String>,
    pub truncated: bool,
}
#[derive(Clone, Serialize, TS)]
pub struct SkillHit {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
}
#[derive(Serialize, TS)]
pub struct SkillResults {
    pub hits: Vec<SkillHit>,
    pub warnings: Vec<String>,
}

type PathCache = HashMap<String, (Instant, Vec<String>)>;
static PATHS: OnceLock<Mutex<PathCache>> = OnceLock::new();

pub async fn paths(
    app: &App,
    root: Option<&str>,
    query: &str,
    workspace: Option<&str>,
) -> Result<PathResults> {
    ensure!(query.len() <= 512, "Search is too long");
    let scoped = root.is_some();
    let mut projects = crate::projects::list(app).await?;
    let preferred = match workspace {
        Some(id) => crate::workspace::get(app, id).await?.project_ids,
        None => vec![],
    };
    projects.sort_by_key(|p| (!preferred.contains(&p.id), !p.pinned, p.name.to_lowercase()));
    let roots = if let Some(root) = root {
        let src = crate::code::open(app, root, None).await?;
        vec![(src.root.to_string_lossy().into_owned(), src.project)]
    } else {
        projects
            .into_iter()
            .filter(|p| !p.is_temp)
            .map(|p| (p.path.clone(), p))
            .collect()
    };
    let query = query.to_lowercase();
    let mut out = PathResults {
        hits: vec![],
        unavailable: vec![],
        truncated: false,
    };
    // Serialize cold builds to bound disk/SSH work; cache hits never enumerate the repository again.
    let mut cache = PATHS.get_or_init(Default::default).lock().await;
    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
    let started = Instant::now();
    for (root, project) in roots {
        if started.elapsed() > Duration::from_secs(6) {
            out.unavailable.push(project.name);
            continue;
        }
        if !cache.contains_key(&root) {
            let load = async {
                let src = crate::code::open(app, &root, None).await?;
                crate::code::prompt_paths(app, &src).await
            };
            match tokio::time::timeout(Duration::from_secs(3), load).await {
                Ok(Ok(paths)) => {
                    if cache.len() >= 32
                        || cache
                            .values()
                            .map(|(_, paths)| paths.iter().map(String::len).sum::<usize>())
                            .sum::<usize>()
                            > 16 * 1024 * 1024
                    {
                        cache.clear();
                    }
                    cache.insert(root.clone(), (Instant::now(), paths));
                }
                _ => {
                    out.unavailable.push(project.name);
                    continue;
                }
            }
        }
        let paths = &cache[&root].1;
        let mut matches: Vec<_> = paths
            .iter()
            .filter_map(|p| match_score(p, &query).map(|score| (score, p)))
            .collect();
        matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let limit = if scoped { 40 } else { 8 };
        // A bounded contribution from each project keeps global results useful.
        out.truncated |= matches.len() > limit;
        out.hits
            .extend(matches.into_iter().take(limit).map(|(_, path)| PathHit {
                project_id: project.id.clone(),
                project: project.name.clone(),
                root: root.clone(),
                path: path.clone(),
                directory: path.ends_with('/'),
            }));
        if out.hits.len() >= 80 {
            out.hits.truncate(80);
            out.truncated = true;
            break;
        }
    }
    Ok(out)
}

/// The profile is checked before resolving roots. Only configuration paths leave this service.
async fn skill_roots(
    app: &App,
    harness: HarnessKind,
    profile: Option<&str>,
    root: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(root) = root {
        crate::code::open(app, root, None).await?;
    }
    let host = root.and_then(loc::host_of);
    let home = match host {
        Some(h) => app.hosts.home(h).context("Machine is not connected")?,
        None => dirs::home_dir()
            .context("Home folder unavailable")?
            .to_string_lossy()
            .into_owned(),
    };
    let spec = LaunchSpec {
        profile: profile.map(str::to_string),
        ..Default::default()
    };
    let overrides = crate::profiles::env(app, harness, &spec, None, host)?;
    let env = app.env.on(host).with(&overrides);
    let mut roots = vec![];
    for location in app.harnesses.get(harness).skill_locations() {
        let base = if location.project {
            let Some(root) = root else { continue };
            std::path::PathBuf::from(root)
                .join(location.base)
                .to_string_lossy()
                .into_owned()
        } else {
            let configured = if let Some(key) = location.config {
                if host.is_some() {
                    let output = crate::procs::command(&env, "printenv", None)
                        .arg(key)
                        .output()
                        .await?;
                    if output.status.success() {
                        Some(String::from_utf8(output.stdout)?.trim().to_string())
                    } else {
                        None
                    }
                } else {
                    env.get(key).map(str::to_string)
                }
            } else {
                None
            };
            let base = configured.filter(|v| !v.is_empty()).unwrap_or_else(|| {
                std::path::Path::new(&home)
                    .join(location.base)
                    .to_string_lossy()
                    .into_owned()
            });
            ensure!(
                std::path::Path::new(&base).is_absolute(),
                "Skill configuration folder must be absolute: {base}"
            );
            loc::join(
                host,
                &std::path::Path::new(&base)
                    .join(location.suffix)
                    .to_string_lossy(),
            )
        };
        if !roots.contains(&base) {
            roots.push(base);
        }
    }
    Ok(roots)
}

#[derive(Deserialize)]
struct Frontmatter {
    name: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default = "yes", rename = "user-invocable")]
    user_invocable: bool,
}
fn yes() -> bool {
    true
}

pub async fn skills(
    app: &App,
    harness: HarnessKind,
    profile: Option<&str>,
    root: Option<&str>,
) -> Result<SkillResults> {
    let roots = skill_roots(app, harness, profile, root).await?;
    let mut result = SkillResults {
        hits: vec![],
        warnings: vec![],
    };
    let mut names = HashSet::new();
    let mut canonical = HashSet::new();
    for source in roots {
        let entries = match hostfs::read_dir(&source).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                result
                    .warnings
                    .push(format!("Could not read skill folder {source}: {e}"));
                continue;
            }
        };
        let mut entries = entries;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries.into_iter().take(500) {
            if entry.name.starts_with('.') || (entry.kind != "dir" && entry.kind != "symlink") {
                continue;
            }
            let path = std::path::Path::new(&source).join(&entry.name);
            let doc = path.join("SKILL.md");
            let read = async {
                let canonical = hostfs::canonicalize(&path).await?;
                let bytes = hostfs::read_limited(&doc, 256 * 1024).await?;
                let text = String::from_utf8(bytes)?;
                let text = text.replace("\r\n", "\n");
                let front = text
                    .strip_prefix("---\n")
                    .and_then(|s| {
                        s.split_once("\n---\n")
                            .or_else(|| s.strip_suffix("\n---").map(|f| (f, "")))
                            .map(|(f, _)| f)
                    })
                    .context("SKILL.md needs YAML frontmatter")?;
                let meta: Frontmatter = serde_yaml_ng::from_str(front)?;
                let name = meta.name.unwrap_or(entry.name.clone());
                ensure!(
                    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_whitespace),
                    "Invalid skill name"
                );
                Ok::<_, anyhow::Error>((canonical, name, meta.description, meta.user_invocable))
            };
            match read.await {
                Ok((path, name, description, invocable)) => {
                    if !invocable {
                        continue;
                    }
                    if names.insert(name.clone()) && canonical.insert(path.clone()) {
                        result.hits.push(SkillHit {
                            name,
                            description,
                            path: path.to_string_lossy().into_owned(),
                            source: source.clone(),
                        });
                    }
                }
                Err(e) => {
                    result.warnings.push(format!("{}: {e}", doc.display()));
                }
            }
        }
    }
    Ok(result)
}

/// Resolve selected skills afresh; never run a skill just because a slash word was typed.
pub async fn render_skills(
    app: &App,
    harness: HarnessKind,
    spec: &LaunchSpec,
    root: Option<&str>,
) -> Result<String> {
    if spec.skills.is_empty() {
        return Ok(String::new());
    }
    let available = tokio::time::timeout(
        Duration::from_secs(10),
        skills(app, harness, spec.profile.as_deref(), root),
    )
    .await
    .context("Selected skill resolution timed out; check the execution machine")??;
    let mut lines = vec![];
    for SkillRef { name, path } in &spec.skills {
        ensure!(
            available
                .hits
                .iter()
                .any(|s| &s.name == name && &s.path == path),
            "Selected skill {name:?} is no longer available for this folder/profile. Remove it and select it again."
        );
        lines.push(
            app.harnesses
                .get(harness)
                .skill_instruction(name, loc::native(path)),
        );
    }
    Ok(format!("\n\nSelected skills:\n{}", lines.join("\n")))
}

/// Validate UTF-16 ranges without slicing Rust UTF-8 at browser offsets.
pub fn validate_mentions(text: &str, mentions: &[crate::model::PromptMention]) -> Result<()> {
    ensure!(mentions.len() <= 512, "Too many prompt mentions");
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut end = 0;
    let mut sorted: Vec<_> = mentions.iter().collect();
    sorted.sort_by_key(|m| m.start);
    for m in sorted {
        ensure!(
            m.start >= end && m.end > m.start && (m.end as usize) <= units.len(),
            "Invalid prompt mention range; edit or remove the mention"
        );
        ensure!(
            String::from_utf16(&units[m.start as usize..m.end as usize])
                .ok()
                .as_deref()
                == Some(&m.text),
            "Prompt mention text changed; select it again"
        );
        end = m.end;
    }
    Ok(())
}

/// Keep selected slash labels readable in the editor without asking a transport to interpret an
/// unverified native slash command. Invocation is the adapter's explicit selected-skill instruction.
pub fn delivery_text(spec: &LaunchSpec) -> String {
    let mut units: Vec<u16> = spec.text.encode_utf16().collect();
    let mut mentions: Vec<_> = spec
        .mentions
        .iter()
        .filter(|m| m.key.starts_with("skill:"))
        .collect();
    mentions.sort_by_key(|m| std::cmp::Reverse(m.start));
    for m in mentions {
        if m.end as usize <= units.len()
            && String::from_utf16(&units[m.start as usize..m.end as usize])
                .ok()
                .as_deref()
                == Some(&m.text)
        {
            units.splice(
                m.start as usize..m.end as usize,
                format!("skill {}", m.text.trim_start_matches('/')).encode_utf16(),
            );
        }
    }
    String::from_utf16_lossy(&units)
}

/// Search the persistent GitHub cache without issuing one remote request per registered repository.
pub async fn github(app: &App, query: &str) -> Result<Vec<crate::model::GitHubRef>> {
    ensure!(query.len() <= 512, "Search is too long");
    let query = query.trim_start_matches('#').to_lowercase();
    let rows: Vec<(String, String)> = app.cache_db.call(|c| {
        let mut st = c.prepare("SELECT url,json FROM gh_response WHERE url GLOB 'project-items:*' OR url GLOB 'item:*' LIMIT 2000")?;
        Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    let mut result = vec![];
    let mut seen = HashSet::new();
    for (key, json) in rows {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
            continue;
        };
        let repo = key
            .strip_prefix("project-items:")
            .or_else(|| {
                key.strip_prefix("item:")
                    .and_then(|s| s.rsplit_once(':').map(|(r, _)| r))
            })
            .unwrap_or_default();
        let items: Vec<&serde_json::Value> = if key.starts_with("item:") {
            vec![&value]
        } else {
            value["prs"]
                .as_array()
                .into_iter()
                .chain(value["issues"].as_array())
                .flatten()
                .collect()
        };
        for item in items {
            let Some(url) = item["url"].as_str() else {
                continue;
            };
            let number = item["number"].as_u64();
            let title = item["title"].as_str().unwrap_or_default();
            if !format!("{repo}#{} {title}", number.unwrap_or_default())
                .to_lowercase()
                .contains(&query)
                || !seen.insert(url.to_string())
            {
                continue;
            }
            result.push(crate::model::GitHubRef {
                url: url.into(),
                repo: repo.into(),
                number,
                title: Some(title.into()),
                kind: if item["__typename"] == "PullRequest" || url.contains("/pull/") {
                    "pr"
                } else {
                    "issue"
                }
                .into(),
            });
        }
    }
    result.truncate(30);
    Ok(result)
}

/// Contiguous matches first, then characters in order with shorter gaps preferred.
fn match_score(value: &str, query: &str) -> Option<usize> {
    let value = value.to_lowercase();
    if let Some(at) = value.find(query) {
        return Some(at);
    }
    let mut remaining = value.as_str();
    let mut score = value.len();
    for ch in query.chars() {
        let at = remaining.find(ch)?;
        score += at;
        remaining = &remaining[at + ch.len_utf8()..];
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PromptMention;
    #[test]
    fn browser_offsets_are_utf16_and_selected_skills_are_explicit() {
        let mut spec = LaunchSpec {
            text: "🌍 /review".into(),
            mentions: vec![PromptMention {
                id: "one".into(),
                start: 3,
                end: 10,
                text: "/review".into(),
                key: "skill:/tmp/review".into(),
                owned: true,
            }],
            ..Default::default()
        };
        validate_mentions(&spec.text, &spec.mentions).unwrap();
        assert_eq!(delivery_text(&spec), "🌍 skill review");
        spec.mentions[0].start = 2;
        assert!(validate_mentions(&spec.text, &spec.mentions).is_err());
    }
}
