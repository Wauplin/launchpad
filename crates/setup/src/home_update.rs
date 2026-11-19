//! Home updates on a Hugging Face Space. Home reads its Space's Dockerfile (written by `deploy/hf-home/deploy.sh`)
//! to know what it installs: a release tag, or CI's package of a `main` commit (`main-build` prerelease). Every hour
//! (and on Check for updates) it looks for a newer one on the same channel; a newer one becomes a Today card. Update
//! Home, an owner click or an admin agent's request, commits the newer Dockerfile to the Space with Home's own
//! Hugging Face login: the Space rebuilds and restarts with it. It can also update the runners: the record
//! (`home_update` in `state.db`, saved in the backup before the commit) survives the restart, and the new Home asks
//! each connected runner with another build to update itself (`self_update`), once per runner, for 24 hours.
//!
//! The update is a protocol, so that nothing Home reports as accepted is lost when the Space restores its backup:
//! 1. Draining: the record is saved (`Pending.phase`), Home enters maintenance (`launchpad_core::maintenance`: every
//!    window shows it, new writes get a retryable 503, automation waits) and the writes already running finish.
//! 2. The continuation: when an agent asked for the update, its wake-up is recorded with its sleep
//!    (`schedules::update_wake`): the new Home sends it once the agent's turn ended (the runner keeps that turn's
//!    records until then), this Home if the update stops.
//! 3. The record says Replacing, then `state.db` is frozen (read-only) and the final checkpoint is taken and checked
//!    in the backup bucket itself (`home_backup::checkpoint`). A failure stops here: Home thaws, leaves maintenance
//!    and says why (`abort`).
//! 4. The Space commit. Home stays frozen, serving reads, until the platform replaces it; a failed build, or a
//!    commit that never happened, thaws it (`resolve`); so does Resume writes, the owner's way out.
//! 5. The new Home restores that checkpoint, sees Replacing with its own commit and marks the update Done
//!    (`resume`); an older build that starts with Replacing (a restart before the build finished) enters
//!    maintenance again until `resolve` decides.
//!
//! A retry of the request after a lost answer gets the update's state, never a second update.
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::maintenance::{self, Maintenance, Phase as MaintenancePhase};
use launchpad_core::role::{Compat, Role};
use launchpad_core::today::{Filter, Item, Kind, Place};
use launchpad_core::util::now_ms;

/// The GitHub repository of Launchpad's releases and CI packages.
pub const REPO: &str = "Wauplin/launchpad";
const EVERY: Duration = Duration::from_secs(3600);
/// While the Space builds an update: check more often, so a failed build shows soon.
const BUILDING_EVERY: Duration = Duration::from_secs(120);
/// How long the new Home updates runners that connect after it started.
const RUNNERS_WINDOW_MS: i64 = 24 * 3600 * 1000;
/// The Space stages that mean its last build or start failed.
const FAILED_STAGES: &[&str] = &["BUILD_ERROR", "RUNTIME_ERROR", "CONFIG_ERROR", "NO_APP_FILE"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "HomeUpdateChannel")]
pub enum Channel {
    /// GitHub releases.
    Release,
    /// CI's package of each `main` commit.
    Main,
}

/// A version of Launchpad Home can install.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "HomeVersion")]
pub struct Version {
    /// A release tag, or a full `main` commit.
    pub version: String,
    pub commit: String,
    /// Published (release) or committed (main), Unix milliseconds.
    #[ts(type = "number")]
    pub at: i64,
    /// The release page, or the comparison with the deployed commit.
    pub url: String,
    /// Main: how many commits of `main` are newer than the deployed one (of the 30 newest; None: more).
    pub behind: Option<u32>,
}

/// An update started from Home, kept until its runners were updated (24 hours).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "HomeUpdatePending")]
pub struct Pending {
    pub version: String,
    pub commit: String,
    /// Update the runners once Home runs `commit`.
    pub update_runners: bool,
    #[ts(type = "number")]
    pub requested_at: i64,
    /// Runners asked to update (each one once).
    pub runners_updated: Vec<String>,
    /// Where the update is. None: a record of an older build (an update without maintenance).
    #[serde(default)]
    pub phase: Option<Phase>,
    /// Why it stopped before the replacement (`Phase::Failed`).
    #[serde(default)]
    pub error: Option<String>,
    /// The agent that asked for it and its wake-up (`schedules::update_wake`), sent once its turn ended.
    #[serde(default)]
    pub continuation: Option<Continuation>,
    /// The Space revision the update's commit names as its parent: the Hub applies it only on top of that revision,
    /// so once the Space's head is another revision without the target, the update can never land (`fenced`).
    #[serde(default)]
    pub parent: Option<String>,
    /// The Dockerfile before the update, which Cancel update commits back (`cancel`).
    #[serde(default)]
    #[ts(skip)]
    pub previous_dockerfile: Option<String>,
}

/// The phases of a planned update (see the module).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "HomeUpdatePhase")]
pub enum Phase {
    Draining,
    /// The final checkpoint holds this phase: the new Home finds it after its restore.
    Replacing,
    /// The new Home runs the new commit.
    Done,
    /// Stopped before the replacement; Home kept running its build.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "HomeUpdateContinuation")]
pub struct Continuation {
    pub session_id: String,
    pub wake_id: String,
}

/// Whether an update is between its drain and its replacement.
fn in_progress(p: &Pending) -> bool {
    matches!(p.phase, Some(Phase::Draining | Phase::Replacing))
}

/// How long the drain waits for running writes before the update stops.
const DRAIN_WITHIN: Duration = Duration::from_secs(60);
/// How long a Home in Replacing waits before Today shows it with Cancel update.
const REPLACE_WITHIN_MS: i64 = 30 * 60 * 1000;
/// `Retry-After` of writes refused during an update: about a build.
const RETRY_AFTER_S: u32 = 600;

#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
pub struct HomeUpdate {
    /// Why this Home cannot update itself from here; None: it can.
    pub unsupported: Option<String>,
    /// The Space ID (`namespace/name`).
    pub space: Option<String>,
    /// The Space's head revision (what a commit names as its parent).
    pub revision: Option<String>,
    /// The revision the Space's runtime reports it runs (None when the Hub does not say).
    pub running_revision: Option<String>,
    pub channel: Option<Channel>,
    /// What the Space's Dockerfile installs: a release tag or a main commit.
    pub deployed: Option<String>,
    /// The commit this Home runs.
    pub running: String,
    /// The newest version of the channel.
    pub latest: Option<Version>,
    /// `latest` is newer than `deployed`.
    pub available: bool,
    /// The Space builds or starts another revision than the one running (an update on its way).
    pub building: bool,
    /// The Space's stage on the Hub (RUNNING, BUILDING, BUILD_ERROR…).
    pub stage: Option<String>,
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
    pub error: Option<String>,
    pub pending: Option<Pending>,
}

static STATE: Mutex<Option<HomeUpdate>> = Mutex::new(None);
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
static STARTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn space_id() -> Option<String> {
    std::env::var("SPACE_ID").ok().map(|s| s.trim().to_string()).filter(|s| s.split('/').count() == 2 && s.split('/').all(|p| !p.is_empty()))
}

/// The last check (without a network call).
pub fn current(app: &App) -> HomeUpdate {
    STATE.lock().unwrap().clone().unwrap_or_else(|| HomeUpdate { running: launchpad_core::commit().into(), unsupported: unsupported(app), ..Default::default() })
}

fn unsupported(app: &App) -> Option<String> {
    if app.paths.role != Role::Home {
        return Some("Only Home updates itself. Update this runner from Home (Settings → Home and runners).".into());
    }
    if space_id().is_none() {
        return Some("Only a Home on a Hugging Face Space updates itself. Update this one the way you installed it.".into());
    }
    None
}

fn publish(app: &App, u: HomeUpdate) {
    let changed = {
        let mut s = STATE.lock().unwrap();
        let changed = s.as_ref() != Some(&u);
        *s = Some(u);
        changed
    };
    if changed {
        app.bus.publish("home_update", "changed", Value::Null);
    }
}

// ------------------------------------------------------------------ the record

async fn load_pending(app: &App) -> Result<Option<Pending>> {
    app.state_db
        .call(|c| {
            let json: Option<String> = c.query_row("SELECT json FROM home_update WHERE id = 1", [], |r| r.get(0)).optional()?;
            Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
        })
        .await
}

async fn save_pending(app: &App, p: Option<&Pending>) -> Result<()> {
    let json = p.map(serde_json::to_string).transpose()?;
    app.state_db
        .call(move |c| {
            match json {
                Some(j) => c.execute("INSERT INTO home_update (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json", [j])?,
                None => c.execute("DELETE FROM home_update", [])?,
            };
            Ok(())
        })
        .await
}

// ------------------------------------------------------------------ Dockerfile

/// What a Space's Dockerfile installs, from its `ARG` lines.
#[derive(Debug, PartialEq)]
pub enum Deployed {
    Release(String),
    /// A `main` commit, from `LAUNCHPAD_PACKAGE=launchpad-linux-x86_64-<commit>`.
    Main(String),
    /// Built from source (`LAUNCHPAD_REVISION`).
    Source,
    /// Not written by deploy.sh.
    Unknown,
}

fn arg(dockerfile: &str, name: &str) -> Option<String> {
    let prefix = format!("ARG {name}");
    dockerfile.lines().find_map(|l| {
        let rest = l.trim().strip_prefix(&prefix)?;
        if rest.is_empty() {
            return Some(String::new());
        }
        Some(rest.strip_prefix('=')?.trim().to_string())
    })
}

pub fn deployed(dockerfile: &str) -> Deployed {
    match arg(dockerfile, "LAUNCHPAD_RELEASE") {
        Some(r) if r == "main-build" => {
            let package = arg(dockerfile, "LAUNCHPAD_PACKAGE").unwrap_or_default();
            match package.strip_prefix("launchpad-linux-x86_64-") {
                Some(c) if is_commit(c) => Deployed::Main(c.to_string()),
                _ => Deployed::Unknown,
            }
        }
        Some(r) if !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) => Deployed::Release(r),
        Some(_) => Deployed::Unknown,
        None if arg(dockerfile, "LAUNCHPAD_REVISION").is_some() => Deployed::Source,
        None => Deployed::Unknown,
    }
}

fn is_commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The target's `Dockerfile.release` with its `ARG` defaults set to `version` (what deploy.sh uploads).
pub fn rewrite(template: &str, channel: Channel, version: &str) -> Result<String> {
    let (release, package) = match channel {
        Channel::Release => (version.to_string(), None),
        Channel::Main => ("main-build".to_string(), Some(format!("launchpad-linux-x86_64-{version}"))),
    };
    let (mut has_release, mut has_package) = (false, false);
    let mut out: Vec<String> = template
        .lines()
        .map(|l| {
            if l.starts_with("ARG LAUNCHPAD_RELEASE=") {
                has_release = true;
                format!("ARG LAUNCHPAD_RELEASE={release}")
            } else if l.starts_with("ARG LAUNCHPAD_PACKAGE=") && let Some(p) = &package {
                has_package = true;
                format!("ARG LAUNCHPAD_PACKAGE={p}")
            } else {
                l.to_string()
            }
        })
        .collect();
    anyhow::ensure!(has_release, "Dockerfile.release has no LAUNCHPAD_RELEASE line");
    anyhow::ensure!(package.is_none() || has_package, "Dockerfile.release has no LAUNCHPAD_PACKAGE line");
    out.push(String::new());
    Ok(out.join("\n"))
}

/// The Space card deploy.sh writes. `tags: [launchpad-home]` lets the Chrome extension find Home after Sign in with
/// Hugging Face.
fn card(channel: Channel, version: &str) -> String {
    let note = match channel {
        Channel::Release => "Built from the GitHub release named by `LAUNCHPAD_RELEASE` in the Dockerfile. Home updates it from Settings → Home and runners, or run the deploy script again.".to_string(),
        Channel::Main => format!("Built from CI's package of main commit `{version}`. Home updates it from Settings → Home and runners, or run the deploy script again with `--main`."),
    };
    format!("---\ntitle: Launchpad Home\nemoji: 🏠\ncolorFrom: blue\ncolorTo: green\nsdk: docker\napp_port: 7860\nheader: mini\ntags:\n- launchpad-home\n---\n\n[Launchpad](https://github.com/{REPO}) Home. {note}\n")
}

// ------------------------------------------------------------------ Hugging Face and GitHub

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("launchpad-home").build().expect("HTTP client")
});

fn hf_endpoint() -> String {
    std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".into()).trim_end_matches('/').to_string()
}

async fn hf_token(app: &App) -> Result<String> {
    launchpad_core::vault::home_token(app, launchpad_core::vault::HomeToken::Huggingface).await.context("Home has no Hugging Face login")
}

async fn hf_send(app: &App, r: reqwest::RequestBuilder) -> Result<String> {
    let res = r.bearer_auth(hf_token(app).await?).send().await.context("reach Hugging Face")?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let why = v["error"].as_str().map(str::to_string).unwrap_or_else(|| text.chars().take(200).collect());
        return Err(anyhow::anyhow!("Hugging Face answered {status}: {why}").context(HubAnswered(status.as_u16())));
    }
    Ok(text)
}

/// The Hub answered with this HTTP status (an error answer, not a transport failure).
#[derive(Debug)]
struct HubAnswered(u16);

impl std::fmt::Display for HubAnswered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hugging Face answered {}", self.0)
    }
}

impl std::error::Error for HubAnswered {}

/// Whether a failed Space commit certainly did not happen (pure): only a 4xx answer of the Hub says so (refused, or
/// another parent revision). A 5xx answer, a timeout or a lost connection may hide a commit that went through.
fn commit_refused(e: &anyhow::Error) -> bool {
    e.chain().find_map(|c| c.downcast_ref::<HubAnswered>()).is_some_and(|a| (400..500).contains(&a.0))
}

/// GitHub's REST API: with Home's GitHub login when it has one, else anonymously.
async fn gh(app: &App, path: &str) -> Result<Value> {
    if let Ok(v) = app.github.get(path).await {
        return Ok(v);
    }
    let res = CLIENT.get(format!("https://api.github.com{path}")).header("Accept", "application/vnd.github+json").send().await.context("reach GitHub")?;
    let status = res.status();
    anyhow::ensure!(status.is_success(), "GitHub answered {status} for {path}");
    Ok(res.json().await?)
}

fn ms(date: &str) -> i64 {
    launchpad_core::today::parse_time(date)
}

async fn latest_release(app: &App) -> Result<Version> {
    let r = gh(app, &format!("/repos/{REPO}/releases/latest")).await?;
    let tag = r["tag_name"].as_str().context("GitHub sent no release tag")?.to_string();
    let has_package = r["assets"].as_array().is_some_and(|a| a.iter().any(|x| x["name"] == "launchpad-linux-x86_64.tar.gz.sha256"));
    anyhow::ensure!(has_package, "release {tag} has no Linux package yet");
    let commit = gh(app, &format!("/repos/{REPO}/commits/{tag}")).await?["sha"].as_str().context("GitHub sent no commit for the release")?.to_string();
    Ok(Version { at: ms(r["published_at"].as_str().unwrap_or_default()), url: r["html_url"].as_str().unwrap_or_default().to_string(), version: tag, commit, behind: None })
}

/// The newest `main` commit that CI has packaged (deploy.sh `--main`).
async fn latest_main(app: &App, deployed: &str) -> Result<Version> {
    let commits = gh(app, &format!("/repos/{REPO}/commits?sha=main&per_page=30")).await?;
    let build = gh(app, &format!("/repos/{REPO}/releases/tags/main-build")).await?;
    let assets: Vec<&str> = build["assets"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect()).unwrap_or_default();
    let list = commits.as_array().context("GitHub sent no commits")?;
    let sha = |c: &Value| c["sha"].as_str().unwrap_or_default().to_string();
    let newest = list.iter().find(|c| assets.contains(&format!("launchpad-linux-x86_64-{}.tar.gz.sha256", sha(c)).as_str())).context("no recent main commit has a CI package yet")?;
    let commit = sha(newest);
    let behind = list.iter().position(|c| sha(c) == deployed).map(|i| i as u32);
    Ok(Version {
        at: ms(newest["commit"]["committer"]["date"].as_str().unwrap_or_default()),
        url: if is_commit(deployed) { format!("https://github.com/{REPO}/compare/{deployed}...{commit}") } else { format!("https://github.com/{REPO}/commit/{commit}") },
        version: commit.clone(),
        commit,
        behind,
    })
}

/// The Space's current revision and Dockerfile.
struct Space {
    id: String,
    sha: String,
    dockerfile: String,
}

async fn inspect(app: &App, u: &mut HomeUpdate) -> Result<Option<Space>> {
    let Some(id) = space_id() else { return Ok(None) };
    u.space = Some(id.clone());
    let info: Value = serde_json::from_str(&hf_send(app, CLIENT.get(format!("{}/api/spaces/{id}", hf_endpoint()))).await?)?;
    let sha = info["sha"].as_str().context("the Hub sent no Space revision")?.to_string();
    u.revision = Some(sha.clone());
    u.stage = info["runtime"]["stage"].as_str().map(str::to_string);
    let running_sha = info["runtime"]["sha"].as_str().unwrap_or_default();
    u.running_revision = Some(running_sha.to_string()).filter(|r| !r.is_empty());
    u.building = !running_sha.is_empty() && running_sha != sha && !u.stage.as_deref().is_some_and(|s| FAILED_STAGES.contains(&s));
    let dockerfile = hf_send(app, CLIENT.get(format!("{}/spaces/{id}/resolve/{sha}/Dockerfile", hf_endpoint()))).await.context("read the Space's Dockerfile")?;
    let (channel, version) = match deployed(&dockerfile) {
        Deployed::Release(t) => (Channel::Release, t),
        Deployed::Main(c) => (Channel::Main, c),
        Deployed::Source => {
            u.unsupported = Some("This Space builds Launchpad from source. Update it with deploy.sh (a release or --main), then Home updates itself.".into());
            return Ok(None);
        }
        Deployed::Unknown => {
            u.unsupported = Some("The Space's Dockerfile was not written by deploy.sh: Home never replaces an edited Dockerfile. Run deploy.sh with --force to replace it.".into());
            return Ok(None);
        }
    };
    u.channel = Some(channel);
    u.deployed = Some(version.clone());
    let latest = match channel {
        Channel::Release => latest_release(app).await?,
        Channel::Main => latest_main(app, &version).await?,
    };
    // Main: a deployed commit missing from the 30 newest is older than all of them.
    u.available = latest.version != version && (channel == Channel::Release || latest.behind != Some(0));
    u.latest = Some(latest);
    Ok(Some(Space { id, sha, dockerfile }))
}

/// Checks now and publishes the result.
pub async fn check(app: &App) -> HomeUpdate {
    check_space(app).await.0
}

async fn check_space(app: &App) -> (HomeUpdate, Option<Space>) {
    let mut u = HomeUpdate { running: launchpad_core::commit().into(), unsupported: unsupported(app), ..Default::default() };
    u.pending = load_pending(app).await.unwrap_or_else(|e| {
        tracing::warn!("Home update record: {e:#}");
        None
    });
    let mut space = None;
    if u.unsupported.is_none() {
        match inspect(app, &mut u).await {
            Ok(s) => space = s,
            Err(e) => u.error = Some(format!("{e:#}")),
        }
        u.checked_at = Some(now_ms());
    }
    publish(app, u.clone());
    (u, space)
}

/// Update Home: the owner's click with the version it showed, an admin agent's request (`launchpad home update
/// [--update_runners true]`) or a ready card. Without `version`, the newest of the channel; `channel` switches between releases and main.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub version: Option<String>,
    pub channel: Option<Channel>,
    /// Then update the runners: the owner's button, or an admin agent's request when the owner asked for it.
    #[serde(default)]
    pub update_runners: bool,
}

/// Whether to install `latest`: the version asked for (if any) is the newest, and it differs from what is deployed.
fn target_ok(b: &Start, deployed: Channel, latest: &Version, available: bool) -> Result<()> {
    if let Some(v) = &b.version {
        anyhow::ensure!(*v == latest.version, "a newer version came out ({}): check it, then update", short(&latest.version));
    }
    let switch = b.channel.is_some_and(|c| c != deployed);
    anyhow::ensure!(switch || available, "Home's Space already installs {}", short(&latest.version));
    Ok(())
}

/// What a retry of Update Home gets while an update is in progress (its answer was lost): the update's state when it
/// asks for the same version (or none), else a refusal (pure).
fn retry_answer(p: &Pending, b: &Start) -> Result<()> {
    anyhow::ensure!(b.version.as_deref().is_none_or(|v| v == p.version), "Home is updating to {} now: check `launchpad home`, then update again once it runs", short(&p.version));
    Ok(())
}

/// Commits the newer Dockerfile to Home's Space with the protocol of the module: drain, continuation, frozen and
/// verified checkpoint, then the commit. It returns once the Hub accepted the commit: the restart comes minutes
/// later, after the build, with Home in maintenance until then. `caller`: the agent session that asked (its
/// continuation), None for the owner.
pub async fn start(app: &App, b: Start, caller: Option<String>) -> Result<HomeUpdate> {
    // A retry after a lost answer: the update in progress answers, never a second one.
    if let Some(p) = load_pending(app).await?.filter(in_progress) {
        retry_answer(&p, &b)?;
        return Ok(current(app));
    }
    let _one = STARTING.try_lock().map_err(|_| anyhow::anyhow!("an update is already starting"))?;
    let (u, space) = check_space(app).await;
    if let Some(why) = &u.unsupported {
        anyhow::bail!("{why}");
    }
    if let Some(e) = &u.error {
        anyhow::bail!("{e}");
    }
    let (space, deployed) = (space.context("no Space")?, u.channel.context("no channel")?);
    let channel = b.channel.unwrap_or(deployed);
    let latest = match (channel == deployed, &u.latest) {
        (true, Some(l)) => l.clone(),
        (true, None) => anyhow::bail!("no newer version"),
        (false, _) => match channel {
            Channel::Release => latest_release(app).await?,
            Channel::Main => latest_main(app, u.deployed.as_deref().unwrap_or_default()).await?,
        },
    };
    target_ok(&b, deployed, &latest, u.available)?;
    anyhow::ensure!(!u.building, "the Space is building another revision: wait until it runs, then check again");
    // The template of the target version (deploy.sh does the same), else the current one.
    let raw = |r: &str| format!("https://raw.githubusercontent.com/{REPO}/{r}/deploy/hf-home/Dockerfile.release");
    let template = match fetch(&raw(&latest.commit)).await {
        Ok(t) => t,
        Err(_) => fetch(&raw("main")).await.context("download Dockerfile.release")?,
    };
    let dockerfile = rewrite(&template, channel, &latest.version)?;
    anyhow::ensure!(dockerfile != space.dockerfile, "the Space's Dockerfile already installs {}", short(&latest.version));
    let mut pending = Pending {
        version: latest.version.clone(),
        commit: latest.commit.clone(),
        update_runners: b.update_runners,
        requested_at: now_ms(),
        runners_updated: vec![],
        phase: Some(Phase::Draining),
        error: None,
        continuation: None,
        parent: Some(space.sha.clone()),
        previous_dockerfile: Some(space.dockerfile.clone()),
    };
    save_pending(app, Some(&pending)).await?;
    let reason = format!("Home updates to {}", short(&latest.version));
    let maintenance = |phase| Maintenance { phase, reason: reason.clone(), target: Some(latest.version.clone()), since: pending.requested_at, retry_after: RETRY_AFTER_S };
    // 1. No new writes; the running ones finish.
    maintenance::enter(app, maintenance(MaintenancePhase::Draining));
    if let Err(e) = maintenance::drained(DRAIN_WITHIN).await {
        return Err(abort(app, &mut pending, format!("Home could not finish the requests in progress: {e:#}")).await);
    }
    // 2. The continuation of the agent that asked, stored with its sleep before the checkpoint.
    if let Some(session) = &caller {
        let text = continuation_text(&latest.version);
        match launchpad_core::ext::extensions().update_wake(app, session, text).await {
            Ok(wake_id) => pending.continuation = Some(Continuation { session_id: session.clone(), wake_id }),
            Err(e) => tracing::warn!("Home update: no wake-up for {session}: {e:#}"),
        }
    }
    // 3. What the new Home restores: the update in Replacing. Then nothing else changes, and the checkpoint.
    pending.phase = Some(Phase::Replacing);
    if let Err(e) = save_pending(app, Some(&pending)).await {
        return Err(abort(app, &mut pending, format!("could not save the update's record: {e:#}")).await);
    }
    maintenance::enter(app, maintenance(MaintenancePhase::Checkpointing));
    if let Err(e) = maintenance::freeze(app).await {
        return Err(abort(app, &mut pending, format!("could not stop the database's writes: {e:#}")).await);
    }
    if let Err(e) = launchpad_core::home_backup::checkpoint(app).await {
        return Err(abort(app, &mut pending, format!("the final backup failed, so Home was not updated (it would restart from an older backup): {e:#}")).await);
    }
    // 4. The commit: the Space builds, then replaces this Home.
    let summary = format!("Update Launchpad Home to {}", match channel { Channel::Release => latest.version.clone(), Channel::Main => format!("main {}", latest.version) });
    if let Err(e) = commit(app, &space, &summary, &[("Dockerfile", dockerfile), ("README.md", card(channel, &latest.version))]).await {
        if commit_refused(&e) {
            return Err(abort(app, &mut pending, format!("{e:#}")).await);
        }
        // A timeout or a lost answer: the commit may have gone through, and the Space may still replace this Home
        // with the checkpoint. Writes stay paused until the Space shows either the update or another head without
        // it (`resolve`, `fenced`); Cancel update makes the second certain.
        tracing::warn!("Home update: the Space commit's outcome is unknown ({e:#}); writes stay paused until the Space shows it");
        maintenance::enter(app, maintenance(MaintenancePhase::Replacing));
        WAKE.notify_one();
        anyhow::bail!("Home could not tell whether the Space accepted the update ({e:#}). Changes stay paused until the Space shows the update (Home then restarts with it) or the owner cancels it; check `launchpad home`.");
    }
    tracing::info!("Home update: committed {} to {}; writes stay paused until the new Home runs", latest.version, space.id);
    maintenance::enter(app, maintenance(MaintenancePhase::Replacing));
    WAKE.notify_one();
    Ok(check(app).await)
}

/// The continuation's message to the agent that asked for an update.
fn continuation_text(version: &str) -> String {
    format!("Home update · You asked Launchpad to update Home to {}. Launchpad sends this message once Home accepts work again: on the new Home after its restart, or on the current one if the update stopped (its Space build failed, its final backup failed, or the owner resumed writes). Check which build runs with `launchpad home` (`running`, and `pending.phase`: done or failed with its error) before you continue.", short(version))
}

/// Stops an update that can no longer replace this Home: `state.db` writable again, maintenance left, the record Failed with
/// `why` (shown in Settings and Today). The continuation stays: the agent is woken on this Home.
async fn abort(app: &App, p: &mut Pending, why: String) -> anyhow::Error {
    tracing::warn!("Home update to {}: stopped: {why}", p.version);
    if maintenance::frozen()
        && let Err(e) = maintenance::thaw(app).await
    {
        tracing::error!("Home update: the database stays read-only: {e:#}");
    }
    maintenance::leave(app);
    p.phase = Some(Phase::Failed);
    p.error = Some(why.clone());
    if let Err(e) = save_pending(app, Some(p)).await {
        tracing::warn!("Home update record: {e:#}");
    }
    check(app).await;
    anyhow::anyhow!("{why}")
}

/// What a starting Home does with the update record its restored `state.db` holds (pure).
#[derive(Debug, PartialEq)]
enum OnStart {
    Nothing,
    /// The new build runs: the update is done.
    Completed,
    /// An older build started with the update in Replacing (a restart before the build finished): maintenance again
    /// until `resolve` decides.
    StillReplacing,
    /// A Home that stopped while it drained (no checkpoint was taken): the update stopped.
    Stopped,
}

fn on_start(p: &Pending, running: &str) -> OnStart {
    match p.phase {
        Some(Phase::Replacing) if p.commit == running => OnStart::Completed,
        Some(Phase::Replacing) => OnStart::StillReplacing,
        Some(Phase::Draining) => OnStart::Stopped,
        _ => OnStart::Nothing,
    }
}

/// At start, before Home serves requests: completes the update the restored record names, or keeps writes paused
/// while its replacement may still come (`on_start`).
pub async fn resume(app: &App) -> Result<()> {
    if unsupported(app).is_some() {
        return Ok(());
    }
    let Some(mut p) = load_pending(app).await? else { return Ok(()) };
    match on_start(&p, launchpad_core::commit()) {
        OnStart::Nothing => {}
        OnStart::Completed => {
            p.phase = Some(Phase::Done);
            save_pending(app, Some(&p)).await?;
            tracing::info!("Home update: running {} as planned{}", short(&p.version), p.continuation.as_ref().map_or_else(String::new, |c| format!("; {} is woken once its turn ended", c.session_id)));
        }
        OnStart::StillReplacing => {
            tracing::warn!("Home update to {}: this Home runs {} and the update was being replaced: writes stay paused until the Space's build finishes or fails", short(&p.version), short(launchpad_core::commit()));
            maintenance::enter(app, Maintenance { phase: MaintenancePhase::Replacing, reason: format!("Home updates to {}", short(&p.version)), target: Some(p.version.clone()), since: p.requested_at, retry_after: RETRY_AFTER_S });
            maintenance::freeze(app).await?;
        }
        OnStart::Stopped => {
            p.phase = Some(Phase::Failed);
            p.error = Some("Home stopped while it finished the requests in progress; nothing was committed to the Space.".into());
            save_pending(app, Some(&p)).await?;
        }
    }
    Ok(())
}

/// Whether the update can no longer replace this Home (pure): the Space's head is another revision than the
/// update's parent and does not install the target (the Hub applies the update's commit only on top of its parent,
/// so it can never land now), and no build is in progress (none of the target, none that would restart this Home).
/// Never from a timeout, a failed check or a failed build: a failed build of the target can still be built again.
/// Only on positive evidence: the Hub reports the runtime RUNNING exactly that head revision, so no build of it is
/// left to replace this Home. A missing runtime revision, another stage (building, a failed build) or a check that
/// failed keeps writes paused.
fn fenced(u: &HomeUpdate, p: &Pending) -> bool {
    u.error.is_none()
        && u.stage.as_deref() == Some("RUNNING")
        && u.revision.is_some()
        && u.running_revision == u.revision
        && u.revision != p.parent
        && u.deployed.as_deref().is_some_and(|d| d != p.version)
}

/// After each check while Home waits for its replacement: once the update is fenced (`fenced`), Home accepts writes
/// again; otherwise it waits, however long (a failed build or a long one shows on Today with Cancel update).
async fn resolve(app: &App, u: &HomeUpdate) {
    let Some(mut p) = u.pending.clone().filter(|p| p.phase == Some(Phase::Replacing) && p.commit != u.running) else { return };
    if !maintenance::active() || !fenced(u, &p) {
        return;
    }
    let why = format!("the Space installs {} again, so the update can no longer replace this Home", short(u.deployed.as_deref().unwrap_or_default()));
    let _ = abort(app, &mut p, why).await;
}

/// Cancel update (`home.update-cancel`, the owner's): commits the Dockerfile from before the update back to the
/// Space, on top of its head now (the Hub refuses it if the head moved meanwhile). From then on the update's commit
/// can never land; Home accepts writes once the Space shows that head without a build in progress (`resolve`). The
/// Space usually rebuilds and restarts with its previous version: that restart restores the checkpoint, which holds
/// everything (writes were paused since), so nothing is lost.
pub async fn cancel(app: &App) -> Result<HomeUpdate> {
    let p = load_pending(app).await?.filter(|p| p.phase == Some(Phase::Replacing)).context("no update waits for its replacement")?;
    anyhow::ensure!(STARTING.try_lock().is_ok(), "the update is still saving its final backup: wait a moment");
    let previous = p.previous_dockerfile.clone().context("this update's record has no previous Dockerfile: run deploy.sh to install a version")?;
    let (u, space) = check_space(app).await;
    if let Some(e) = &u.error {
        anyhow::bail!("{e}");
    }
    let space = space.context("no Space")?;
    let (channel, version) = match deployed(&previous) {
        Deployed::Release(v) => (Channel::Release, v),
        Deployed::Main(c) => (Channel::Main, c),
        _ => anyhow::bail!("the previous Dockerfile was not written by deploy.sh"),
    };
    // The card always changes (it names the cancel), so the commit moves the head even when the Dockerfile did not.
    let note = format!("{}\nThe update to {} was cancelled at {}.\n", card(channel, &version), short(&p.version), now_ms());
    commit(app, &space, &format!("Cancel the update of Launchpad Home to {}", short(&p.version)), &[("Dockerfile", previous), ("README.md", note)]).await?;
    tracing::info!("Home update to {}: cancelled; writes resume once the Space shows its previous version", p.version);
    WAKE.notify_one();
    let u = check(app).await;
    resolve(app, &u).await;
    Ok(current(app))
}

fn short(v: &str) -> &str {
    if is_commit(v) { &v[..8] } else { v }
}

async fn fetch(url: &str) -> Result<String> {
    let res = CLIENT.get(url).send().await?;
    anyhow::ensure!(res.status().is_success(), "{url}: {}", res.status());
    Ok(res.text().await?)
}

/// One commit to the Space's `main` branch (the Hub's commit API), on top of the revision Home checked.
async fn commit(app: &App, space: &Space, summary: &str, files: &[(&str, String)]) -> Result<()> {
    use base64::Engine;
    let mut body = serde_json::to_string(&serde_json::json!({"key": "header", "value": {"summary": summary, "description": "", "parentCommit": space.sha}}))?;
    for (path, content) in files {
        let content = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        body.push('\n');
        body.push_str(&serde_json::to_string(&serde_json::json!({"key": "file", "value": {"path": path, "content": content, "encoding": "base64"}}))?);
    }
    let url = format!("{}/api/spaces/{}/commit/main", hf_endpoint(), space.id);
    hf_send(app, CLIENT.post(url).header("Content-Type", "application/x-ndjson").body(body))
        .await
        .map_err(|e| e.context(format!("Home's Hugging Face login needs write access to the Space {} (or update with deploy.sh)", space.id)))?;
    Ok(())
}

// ------------------------------------------------------------------ runners after the restart

/// The new Home asks each connected runner with another build to update itself, once per runner.
async fn update_runners(app: &App) -> Result<()> {
    let Some(mut p) = load_pending(app).await? else { return Ok(()) };
    if now_ms() > p.requested_at + RUNNERS_WINDOW_MS {
        save_pending(app, None).await?;
        check(app).await;
        return Ok(());
    }
    if !p.update_runners || launchpad_core::commit() != p.commit {
        return Ok(());
    }
    for r in launchpad_core::runners::list(app).await? {
        let id = r.runner.id.clone();
        let ready = r.runner.state == launchpad_core::runners::RunnerState::Approved && r.connected && r.compat == Some(Compat::Outdated) && r.runner.capabilities.self_update && r.runner.os == std::env::consts::OS;
        if !ready || p.runners_updated.contains(&id) {
            continue;
        }
        // Claimed before the request: a runner is asked once, never again after a crash.
        p.runners_updated.push(id.clone());
        save_pending(app, Some(&p)).await?;
        match app.runners.request(&id, "POST", "/api/runner/update", None).await {
            Ok((status, _)) if status < 300 => tracing::info!("Home update: {} updates itself", r.runner.name),
            Ok((status, body)) => tracing::warn!("Home update: {} refused the update ({status}): {}", r.runner.name, body.chars().take(200).collect::<String>()),
            Err(e) => tracing::warn!("Home update: {}: {e:#}", r.runner.name),
        }
        app.bus.publish("runners", "changed", Value::Null);
    }
    let mut u = current(app);
    if u.pending.as_ref() != Some(&p) {
        u.pending = Some(p);
        publish(app, u);
    }
    Ok(())
}

/// Home on a Space: checks every hour (every two minutes while the Space builds), and updates the runners after
/// an update it started.
pub fn spawn(app: &App) {
    if unsupported(app).is_some() {
        return;
    }
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let u = check(&a).await;
            resolve(&a, &u).await;
            let wait = if u.building || u.pending.as_ref().is_some_and(|p| p.commit != u.running) { BUILDING_EVERY } else { EVERY };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = update_runners(&a).await {
                tracing::warn!("Home update: runners: {e:#}");
            }
        }
    });
}

// ------------------------------------------------------------------ Today

/// A newer Launchpad for Home, or an update whose build failed.
pub fn today_items(app: &App) -> Vec<Item> {
    let u = current(app);
    if u.unsupported.is_some() {
        return vec![];
    }
    if let Some(p) = u.pending.as_ref().filter(|p| p.phase == Some(Phase::Failed) && p.version != u.deployed.clone().unwrap_or_default()) {
        let item = Item::new(Kind::HomeUpdate, Filter::Fix, "home", format!("Home's update to {} stopped", short(&p.version)), Place::Everywhere, p.requested_at)
            .detail(format!("{} Home runs {} and accepts writes.", p.error.clone().unwrap_or_default(), short(&u.running)))
            .action("home.update");
        return vec![item];
    }
    if let Some(p) = u.pending.as_ref().filter(|p| p.phase == Some(Phase::Replacing) && p.commit != u.running && now_ms() - p.requested_at > REPLACE_WITHIN_MS / 3) {
        let item = Item::new(Kind::HomeUpdate, Filter::Fix, "home", format!("Home waits for its update to {}", short(&p.version)), Place::Everywhere, p.requested_at)
            .detail(match u.stage.as_deref().filter(|s| FAILED_STAGES.contains(s)) {
                Some(stage) => format!("The Space's build stopped in {stage}. Writes stay paused: a new build of the update would still restart Home from its checkpoint. Cancel update puts the previous version back; Home accepts writes once the Space shows it."),
                None => "Writes stay paused until the Space restarts with the new build. Cancel update puts the previous version back; Home accepts writes once the Space shows it.".to_string(),
            })
            .action("home.update-cancel");
        return vec![item];
    }
    let failed = u.stage.as_deref().filter(|s| FAILED_STAGES.contains(s));
    if let (Some(p), Some(stage)) = (&u.pending, failed)
        && p.commit != u.running
    {
        let item = Item::new(Kind::HomeUpdate, Filter::Fix, "home", format!("Home's update to {} did not start", short(&p.version)), Place::Everywhere, p.requested_at)
            .detail(format!("The Space stopped in {stage}; Home still runs {}. Read the Space's logs on the Hub.", short(&u.running)));
        return vec![item];
    }
    let Some(latest) = u.latest.filter(|_| u.available && !u.building) else { return vec![] };
    if u.pending.as_ref().is_some_and(|p| p.version == latest.version) {
        return vec![];
    }
    let detail = match (u.channel, latest.behind) {
        (Some(Channel::Main), Some(n)) => format!("{n} newer commits on main; Home installs {}", short(u.deployed.as_deref().unwrap_or_default())),
        _ => format!("Home installs {}", short(u.deployed.as_deref().unwrap_or_default())),
    };
    vec![Item::new(Kind::HomeUpdate, Filter::Fix, "home", format!("Update Home to {}", short(&latest.version)), Place::Everywhere, latest.at).detail(detail).action("home.update")]
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn reads_what_the_dockerfile_installs() {
        assert_eq!(deployed("FROM x\nARG LAUNCHPAD_RELEASE=v2026.11.06.1\nARG LAUNCHPAD_PACKAGE=launchpad-linux-x86_64\n"), Deployed::Release("v2026.11.06.1".into()));
        assert_eq!(deployed(&format!("ARG LAUNCHPAD_RELEASE=main-build\nARG LAUNCHPAD_PACKAGE=launchpad-linux-x86_64-{C}\n")), Deployed::Main(C.into()));
        assert_eq!(deployed("ARG LAUNCHPAD_RELEASE=main-build\nARG LAUNCHPAD_PACKAGE=launchpad-linux-x86_64\n"), Deployed::Unknown);
        assert_eq!(deployed("FROM rust\nARG LAUNCHPAD_REVISION\n"), Deployed::Source);
        assert_eq!(deployed("FROM debian\n"), Deployed::Unknown);
        assert_eq!(deployed("ARG LAUNCHPAD_RELEASE=v1;rm -rf\n"), Deployed::Unknown);
    }

    #[test]
    fn installs_the_newest_or_switches_channel() {
        let latest = Version { version: C.into(), commit: C.into(), at: 0, url: String::new(), behind: Some(3) };
        let start = |version: Option<&str>, channel| Start { version: version.map(str::to_string), channel, update_runners: false };
        // An agent's request: the newest of the deployed channel, when it is newer.
        assert!(target_ok(&start(None, None), Channel::Main, &latest, true).is_ok());
        assert!(target_ok(&start(None, Some(Channel::Main)), Channel::Main, &latest, false).is_err());
        // The owner's click names what it showed; a newer one came out since.
        assert!(target_ok(&start(Some(C), None), Channel::Main, &latest, true).is_ok());
        assert!(target_ok(&start(Some("v1"), None), Channel::Main, &latest, true).is_err());
        // Another channel always changes the Dockerfile.
        assert!(target_ok(&start(None, Some(Channel::Main)), Channel::Release, &latest, false).is_ok());
        // Agents and ready cards cannot ask to update the runners; the owner's button sets it.
        let b: Start = serde_json::from_str(r#"{"channel": "main"}"#).unwrap();
        assert!(!b.update_runners && b.channel == Some(Channel::Main) && b.version.is_none());
        assert!(serde_json::from_str::<Start>(r#"{"source": "main"}"#).is_err());
    }

    #[test]
    fn rewrites_the_arg_defaults() {
        let t = "# c\nARG LAUNCHPAD_RELEASE=v1\nARG LAUNCHPAD_PACKAGE=launchpad-linux-x86_64\nFROM x\nARG LAUNCHPAD_RELEASE\n";
        let r = rewrite(t, Channel::Release, "v2").unwrap();
        assert_eq!(r, "# c\nARG LAUNCHPAD_RELEASE=v2\nARG LAUNCHPAD_PACKAGE=launchpad-linux-x86_64\nFROM x\nARG LAUNCHPAD_RELEASE\n");
        let m = rewrite(t, Channel::Main, C).unwrap();
        assert_eq!(deployed(&m), Deployed::Main(C.into()));
        assert!(rewrite("FROM x\n", Channel::Release, "v2").is_err());
        assert!(rewrite("ARG LAUNCHPAD_RELEASE=v1\n", Channel::Main, C).is_err());
    }

    fn pending(phase: Option<Phase>) -> Pending {
        Pending { version: C.into(), commit: C.into(), update_runners: false, requested_at: 0, runners_updated: vec![], phase, error: None, continuation: None, parent: Some("parent".into()), previous_dockerfile: Some("ARG LAUNCHPAD_RELEASE=v1\n".into()) }
    }

    /// The update's answer was lost (the CLI timed out after the commit): asking again returns the update in progress,
    /// never a second update; asking for another version is refused while one runs.
    #[test]
    fn a_retry_after_a_lost_answer_gets_the_update_in_progress() {
        let p = pending(Some(Phase::Replacing));
        assert!(in_progress(&p) && in_progress(&pending(Some(Phase::Draining))));
        assert!(!in_progress(&pending(Some(Phase::Done))) && !in_progress(&pending(Some(Phase::Failed))) && !in_progress(&pending(None)));
        let start = |version: Option<&str>| Start { version: version.map(str::to_string), channel: None, update_runners: false };
        assert!(retry_answer(&p, &start(None)).is_ok());
        assert!(retry_answer(&p, &start(Some(C))).is_ok());
        assert!(retry_answer(&p, &start(Some("v2"))).is_err());
    }

    /// At start, from the restored checkpoint: the new build completes the update; an older build that starts while
    /// the record says Replacing (a restart before the build finished) keeps writes paused; a drain that never
    /// reached its checkpoint stopped.
    #[test]
    fn a_starting_home_completes_or_keeps_waiting_for_its_update() {
        assert_eq!(on_start(&pending(Some(Phase::Replacing)), C), OnStart::Completed);
        assert_eq!(on_start(&pending(Some(Phase::Replacing)), "old"), OnStart::StillReplacing);
        assert_eq!(on_start(&pending(Some(Phase::Draining)), "old"), OnStart::Stopped);
        assert_eq!(on_start(&pending(Some(Phase::Done)), C), OnStart::Nothing);
        assert_eq!(on_start(&pending(None), "old"), OnStart::Nothing);
    }

    /// While the old Home waits for its replacement, it accepts writes again only once the update can never land:
    /// never after a timeout, a failed check, or a failed build (a later build of the target would still restart Home
    /// from its checkpoint), only once the Space's head is another revision without the target and nothing builds.
    #[test]
    fn writes_resume_only_once_the_update_is_fenced() {
        let p = pending(Some(Phase::Replacing));
        let landed = HomeUpdate { running: "old".into(), revision: Some("update".into()), deployed: Some(C.into()), ..Default::default() };
        // The update landed and builds, or its build failed, or the check failed: wait.
        assert!(!fenced(&HomeUpdate { building: true, ..landed.clone() }, &p));
        assert!(!fenced(&HomeUpdate { stage: Some("BUILD_ERROR".into()), ..landed.clone() }, &p));
        assert!(!fenced(&HomeUpdate { error: Some("Hub down".into()), ..landed.clone() }, &p));
        assert!(!fenced(&landed, &p));
        // A lost commit answer: the head is still the parent, so the commit may still land: wait, however long.
        let unknown = HomeUpdate { running: "old".into(), revision: Some("parent".into()), deployed: Some("v1".into()), ..Default::default() };
        assert!(!fenced(&unknown, &p));
        // Cancel update committed the previous Dockerfile on top of the head; while that builds (a late build), wait.
        let cancelled = HomeUpdate { running: "old".into(), revision: Some("cancel".into()), deployed: Some("v1".into()), stage: Some("RUNNING".into()), running_revision: Some("cancel".into()), ..Default::default() };
        assert!(!fenced(&HomeUpdate { running_revision: Some("parent".into()), building: true, ..cancelled.clone() }, &p));
        // No runtime revision, or a failed build of the cancel: not proof that nothing will replace this Home.
        assert!(!fenced(&HomeUpdate { running_revision: None, ..cancelled.clone() }, &p));
        assert!(!fenced(&HomeUpdate { stage: Some("BUILD_ERROR".into()), ..cancelled.clone() }, &p));
        // The runtime runs the cancel revision: the update's commit (parent `parent`) can never land.
        assert!(fenced(&cancelled, &p));
        // Only the Hub's 4xx answer proves a commit did not happen; a 5xx, a timeout or a lost connection does not.
        assert!(commit_refused(&anyhow::anyhow!("x").context(HubAnswered(409))));
        assert!(!commit_refused(&anyhow::anyhow!("x").context(HubAnswered(502))));
        assert!(!commit_refused(&anyhow::anyhow!("reach Hugging Face: operation timed out")));
    }
}
