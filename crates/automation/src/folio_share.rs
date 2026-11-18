//! Sharing a folio: a static snapshot of one rev, published as a Hugging Face Space under the owner's namespace
//! (`<owner>/folio-<slug>`, private by default) only on the owner's click (`folio.share`, or a ready card an agent
//! prepared). The snapshot holds the folio-viewer build (the reader's renderer and theme without Launchpad's API,
//! embedded in the binary like the UI and set by `crates/cli` with `set_viewer`), `folio.json` (the pages and files of
//! that rev; the owner's decisions as text only when asked) and a Space card (`sdk: static`, tag `launchpad-folio`).
//! No comments, no data API, no live updates. A snapshot expires after 30 days unless kept: Home's hourly task deletes
//! expired Spaces and those of deleted folios. Stop sharing deletes the Space at once.
//!
//! `LAUNCHPAD_FOLIO_SHARE_DIR` makes Home write each snapshot into that folder (`<dir>/<owner>/folio-<slug>/`) instead
//! of the Hub: the dry run that checks use. Checks never create, update or delete a real Space.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use launchpad_core::App;
use launchpad_core::util::now_ms;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::folios::{self, Folio};

/// Snapshots expire after this many days by default.
pub const DEFAULT_DAYS: u32 = 30;
const DAY_MS: i64 = 24 * 3600 * 1000;
/// Largest snapshot (the Hub's commit API takes it in one request).
const MAX_BYTES: usize = 40 << 20;

/// Files as (path, bytes).
pub type Files = Vec<(String, Vec<u8>)>;

static VIEWER: OnceLock<fn() -> Files> = OnceLock::new();

/// The folio-viewer build's files (`web/dist/folio-viewer/…`, paths relative to it), from the binary's UI assets.
pub fn set_viewer(files: fn() -> Files) {
    let _ = VIEWER.set(files);
}

/// A folio's shared snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioShare")]
pub struct Share {
    /// `owner/folio-<slug>`.
    pub space: String,
    pub url: String,
    #[ts(type = "number")]
    pub rev: i64,
    /// private or public.
    pub visibility: String,
    /// None: kept until Stop sharing.
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    /// The owner's decisions are shown as text.
    pub decisions: bool,
    #[ts(type = "number")]
    pub at: i64,
    /// shared, or failed (with `error`): Home retries a failed deletion every hour.
    pub state: String,
    pub error: Option<String>,
    /// Written to a local folder (`LAUNCHPAD_FOLIO_SHARE_DIR`), not to the Hub.
    pub dry_run: bool,
}

/// The Share dialog's choices.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[ts(rename = "FolioShareInput")]
pub struct ShareInput {
    /// private (default) or public.
    #[ts(optional)]
    pub visibility: Option<String>,
    /// Keep it until Stop sharing (no expiry).
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub keep: bool,
    /// Days before it expires (default 30, at most 365).
    #[ts(optional)]
    pub days: Option<u32>,
    /// Show the owner's decisions (choices, verdicts, ticks) as text.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub decisions: bool,
    /// The rev to share (default: the latest).
    #[ts(optional, type = "number")]
    pub rev: Option<i64>,
}

fn dry_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LAUNCHPAD_FOLIO_SHARE_DIR").filter(|v| !v.is_empty()).map(Into::into)
}

fn endpoint() -> String {
    std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".into()).trim_end_matches('/').to_string()
}

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).user_agent("launchpad-home").build().expect("HTTP client"));

async fn token(app: &App) -> Result<String> {
    launchpad_core::vault::home_token(app, launchpad_core::vault::HomeToken::Huggingface).await.context("Home has no Hugging Face login: add one in Settings → Home and runners to share folios")
}

async fn hub(app: &App, r: reqwest::RequestBuilder) -> Result<(u16, String)> {
    let res = r.bearer_auth(token(app).await?).send().await.context("reach Hugging Face")?;
    let status = res.status().as_u16();
    Ok((status, res.text().await.unwrap_or_default()))
}

fn hub_error(status: u16, text: &str) -> String {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    format!("Hugging Face answered {status}: {}", v["error"].as_str().map(str::to_string).unwrap_or_else(|| text.chars().take(200).collect()))
}

/// The namespace snapshots go to: the owner's account (the dry run names it `owner`).
async fn namespace(app: &App) -> Result<String> {
    if dry_dir().is_some() {
        return Ok("owner".into());
    }
    let (status, text) = hub(app, CLIENT.get(format!("{}/api/whoami-v2", endpoint()))).await?;
    ensure!((200..300).contains(&status), "{}", hub_error(status, &text));
    let v: Value = serde_json::from_str(&text)?;
    Ok(v["name"].as_str().context("Hugging Face sent no account name")?.to_string())
}

/// Pure: the Space name of a folio's snapshot (`folio-<slug>`, at most 96 characters).
pub fn space_name(slug: &str) -> String {
    format!("folio-{slug}").chars().take(96).collect()
}

/// Pure: the Space card.
pub fn card(title: &str) -> String {
    let t = title.replace(['"', '\n'], " ");
    format!("---\ntitle: \"{t}\"\nsdk: static\napp_file: index.html\npinned: false\ntags:\n  - launchpad-folio\n---\n\nA snapshot of a Launchpad folio: read-only, without comments or data. It expires unless its owner keeps it.\n")
}

/// Pure: the decisions of the owner, as text lines for the snapshot (choices, verdicts, ticks).
pub fn decisions_text(marks: &[folios::Mark]) -> Vec<Value> {
    marks.iter().filter(|m| m.author.kind == launchpad_core::kb::AuthorKind::Owner && matches!(m.kind.as_str(), "choice" | "verdict" | "check")).map(|m| json!({"kind": m.kind, "target": m.target, "value": m.value, "page": m.page})).collect()
}

/// The snapshot's files: the viewer, `folio.json` and the card.
async fn snapshot(app: &App, f: &Folio, rev: i64, decisions: bool) -> Result<Vec<(String, Vec<u8>)>> {
    let viewer = VIEWER.get().map(|v| v()).unwrap_or_default();
    ensure!(viewer.iter().any(|(p, _)| p == "index.html"), "this build has no folio viewer (web/dist/folio-viewer): rebuild the UI with pnpm build");
    let d = folios::detail(app, &f.workspace_id, &f.slug, Some(rev)).await?;
    let mut files = BTreeMap::new();
    let mut total = 0;
    for e in &d.files {
        let data = folios::file_bytes(app, &f.id, &e.path, Some(rev)).await?;
        total += data.len();
        let value = match String::from_utf8(data) {
            Ok(t) if !e.path.ends_with(".svg") => json!({"text": t}),
            Ok(t) => json!({"data": format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(t))}),
            Err(e2) => {
                let mime = match e.path.rsplit('.').next().unwrap_or("") { "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp", _ => "application/octet-stream" };
                json!({"data": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(e2.into_bytes()))})
            }
        };
        files.insert(e.path.clone(), value);
    }
    ensure!(total <= MAX_BYTES, "this folio is {} MB: a snapshot holds at most 40 MB", total >> 20);
    let marks = if decisions { decisions_text(&d.marks) } else { vec![] };
    let pages: Vec<Value> = d.pages.iter().map(|p| json!({"path": p.path, "title": p.title, "text": p.text, "outline": p.outline})).collect();
    let folio_json = json!({"title": f.title, "slug": f.slug, "kind": f.kind, "status": f.status, "summary": f.summary, "rev": rev, "shared_at": now_ms(), "pages": pages, "files": files, "marks": marks});
    let mut out: Vec<(String, Vec<u8>)> = viewer;
    out.push(("folio.json".into(), serde_json::to_vec(&folio_json)?));
    out.push(("README.md".into(), card(&f.title).into_bytes()));
    Ok(out)
}

async fn create_space(app: &App, space: &str, private: bool) -> Result<()> {
    let (ns, name) = space.split_once('/').context("a Space id is owner/name")?;
    let (status, text) = hub(app, CLIENT.post(format!("{}/api/repos/create", endpoint())).json(&json!({"type": "space", "name": name, "organization": Value::Null, "private": private, "sdk": "static"}))).await?;
    let _ = ns;
    match status {
        200..=299 => Ok(()),
        409 => bail!("a Space {space} already exists and is not this folio's snapshot: delete or rename it on the Hub first"),
        _ => bail!("{}", hub_error(status, &text)),
    }
}

async fn upload(app: &App, space: &str, files: &[(String, Vec<u8>)], summary: &str) -> Result<()> {
    let mut body = serde_json::to_string(&json!({"key": "header", "value": {"summary": summary, "description": ""}}))?;
    for (path, data) in files {
        body.push('\n');
        body.push_str(&serde_json::to_string(&json!({"key": "file", "value": {"path": path, "content": base64::engine::general_purpose::STANDARD.encode(data), "encoding": "base64"}}))?);
    }
    let (status, text) = hub(app, CLIENT.post(format!("{}/api/spaces/{space}/commit/main", endpoint())).header("Content-Type", "application/x-ndjson").body(body)).await?;
    ensure!((200..300).contains(&status), "{}", hub_error(status, &text));
    Ok(())
}

async fn delete_space(app: &App, space: &str) -> Result<()> {
    if let Some(dir) = dry_dir() {
        let path = dir.join(space);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        return Ok(());
    }
    let (ns, name) = space.split_once('/').context("a Space id is owner/name")?;
    let mine = namespace(app).await?;
    ensure!(ns == mine, "the Space {space} is not in your namespace");
    let (status, text) = hub(app, CLIENT.delete(format!("{}/api/repos/delete", endpoint())).json(&json!({"type": "space", "name": name}))).await?;
    ensure!((200..300).contains(&status) || status == 404, "{}", hub_error(status, &text));
    Ok(())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Share> {
    let space: String = r.get(0)?;
    Ok(Share { url: format!("{}/spaces/{space}", endpoint()), space, rev: r.get(1)?, visibility: r.get(2)?, expires_at: r.get(3)?, decisions: r.get::<_, i64>(4)? != 0, at: r.get(5)?, state: r.get(6)?, error: r.get(7)?, dry_run: dry_dir().is_some() })
}

/// The folio's shared snapshot, if any.
pub async fn get(app: &App, folio_id: &str) -> Result<Option<Share>> {
    let id = folio_id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT space, rev, visibility, expires_at, decisions, at, state, error FROM folio_share WHERE folio_id = ?1", [id], row).optional()?)).await
}

/// One share at a time per Home: two clicks never create two Spaces.
static SHARES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Share (or update) the snapshot of a folio: the owner's click. Claimed before the Hub calls (a `sharing` row),
/// never retried on its own: an uncertain result stays visible as failed.
pub async fn share(app: &App, ws: &str, slug: &str, input: ShareInput) -> Result<Share> {
    let _one = SHARES.lock().await;
    let f = folios::get(app, ws, slug).await?;
    let rev = input.rev.unwrap_or(f.rev);
    ensure!((1..=f.rev).contains(&rev), "no rev {rev} in this folio");
    let visibility = input.visibility.clone().unwrap_or_else(|| "private".into());
    ensure!(visibility == "private" || visibility == "public", "visibility is private or public");
    let days = input.days.unwrap_or(DEFAULT_DAYS);
    ensure!((1..=365).contains(&days), "a snapshot expires after 1 to 365 days, or is kept");
    let expires_at = (!input.keep).then(|| now_ms() + i64::from(days) * DAY_MS);
    let files = snapshot(app, &f, rev, input.decisions).await?;
    let existing = get(app, &f.id).await?;
    let space = match &existing {
        Some(s) => s.space.clone(),
        None => format!("{}/{}", namespace(app).await?, space_name(&f.slug)),
    };
    let (id, sp, vis, at, dec) = (f.id.clone(), space.clone(), visibility.clone(), now_ms(), input.decisions);
    app.state_db.call(move |c| {
        c.execute("INSERT INTO folio_share (folio_id, space, rev, visibility, expires_at, decisions, at, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sharing') ON CONFLICT (folio_id) DO UPDATE SET rev = ?3, visibility = ?4, expires_at = ?5, decisions = ?6, at = ?7, state = 'sharing', error = NULL", params![id, sp, rev, vis, expires_at, dec, at])?;
        Ok(())
    }).await?;
    let result: Result<()> = async {
        if let Some(dir) = dry_dir() {
            let root = dir.join(&space);
            for (path, data) in &files {
                let p = root.join(path);
                std::fs::create_dir_all(p.parent().context("a folder")?)?;
                std::fs::write(p, data)?;
            }
            return Ok(());
        }
        if existing.is_none() {
            create_space(app, &space, visibility == "private").await?;
        } else {
            // A changed visibility (the Hub's settings API).
            let (status, text) = hub(app, CLIENT.put(format!("{}/api/spaces/{space}/settings", endpoint())).json(&json!({"private": visibility == "private"}))).await?;
            ensure!((200..300).contains(&status), "{}", hub_error(status, &text));
        }
        upload(app, &space, &files, &format!("Snapshot of rev {rev}")).await
    }.await;
    let (id, state, error) = (f.id.clone(), if result.is_ok() { "shared" } else { "failed" }, result.as_ref().err().map(|e| format!("{e:#}")));
    app.state_db.call(move |c| Ok(c.execute("UPDATE folio_share SET state = ?2, error = ?3 WHERE folio_id = ?1", params![id, state, error])?)).await?;
    app.bus.publish("folios", "changed", json!({"workspace_id": f.workspace_id, "id": f.id, "slug": f.slug}));
    result?;
    get(app, &f.id).await?.context("the share record is gone")
}

/// Extend (days from now) or keep the snapshot: no Hub call.
pub async fn extend(app: &App, ws: &str, slug: &str, days: Option<u32>, keep: bool) -> Result<Share> {
    let f = folios::get(app, ws, slug).await?;
    ensure!(get(app, &f.id).await?.is_some(), "this folio is not shared");
    let days = days.unwrap_or(DEFAULT_DAYS);
    ensure!((1..=365).contains(&days), "extend by 1 to 365 days, or keep it");
    let expires = (!keep).then(|| now_ms() + i64::from(days) * DAY_MS);
    let id = f.id.clone();
    app.state_db.call(move |c| Ok(c.execute("UPDATE folio_share SET expires_at = ?2 WHERE folio_id = ?1", params![id, expires])?)).await?;
    get(app, &f.id).await?.context("the share record is gone")
}

/// Stop sharing: deletes the Space, then the record (a failed deletion stays, retried hourly).
pub async fn stop(app: &App, folio_id: &str) -> Result<()> {
    let Some(s) = get(app, folio_id).await? else { return Ok(()) };
    let id = folio_id.to_string();
    match delete_space(app, &s.space).await {
        Ok(()) => {
            app.state_db.call(move |c| Ok(c.execute("DELETE FROM folio_share WHERE folio_id = ?1", [id])?)).await?;
            Ok(())
        }
        Err(e) => {
            let why = format!("Stop sharing failed: {e:#}");
            app.state_db.call(move |c| Ok(c.execute("UPDATE folio_share SET state = 'failed', error = ?2 WHERE folio_id = ?1", params![id, why])?)).await?;
            Err(e)
        }
    }
}

/// Pure: whether a share goes now: expired, or its folio is gone.
pub fn due(expires_at: Option<i64>, folio_exists: bool, now: i64) -> bool {
    !folio_exists || expires_at.is_some_and(|t| t <= now)
}

/// Hourly on Home: deletes expired snapshots and those of deleted folios.
pub async fn expire(app: &App) -> Result<usize> {
    let rows: Vec<(String, Option<i64>, bool)> = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT s.folio_id, s.expires_at, EXISTS (SELECT 1 FROM folio f WHERE f.id = s.folio_id) FROM folio_share s")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut n = 0;
    for (id, expires, exists) in rows {
        if due(expires, exists, now_ms()) {
            match stop(app, &id).await {
                Ok(()) => n += 1,
                Err(e) => tracing::warn!("folio snapshot of {id}: {e:#}"),
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cards_and_expiry() {
        assert_eq!(space_name("roadmap-winter"), "folio-roadmap-winter");
        assert!(space_name(&"a".repeat(200)).len() <= 96);
        let c = card("Roadmap \"winter\"\nx");
        assert!(c.contains("sdk: static") && c.contains("launchpad-folio") && c.contains("title: \"Roadmap  winter  x\""));
        assert!(due(Some(10), true, 10) && !due(Some(11), true, 10) && !due(None, true, 10) && due(None, false, 10));
    }
}
