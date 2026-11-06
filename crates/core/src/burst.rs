//! Burst runners: HF Jobs that Home starts from its own Space image when a launch finds no runner (your
//! laptop is off) or when you continue a session on Burst.
//!
//! A Burst runner is an ordinary runner with three differences. Home creates its record and credential (no
//! pairing) and passes the credential as a job secret. Home dials it through the HF Jobs proxy, because the job
//! cannot reach a private Home. It holds no token: its proxy sends requests that need one to Home
//! (`token_relay`), and Home Git and plain folders go through the tunnel too. The job is the boundary: agents
//! run without a jail inside it.
//!
//! One job runs one session. When its turn ends, the runner hands the session off (branches pushed, leftovers
//! saved, conversation sent to Home), Home waits for its next complete backup, then cancels the job. A
//! follow-up starts a new job and moves the session there. Each job's minutes count toward the monthly cap
//! (Settings → Home and runners); at the cap, Home starts no new job.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::App;
use crate::model::{LaunchSpec, Session, SessionStatus};
use crate::util::now_ms;

/// The runner id a launch spec uses to ask for Burst (`LaunchSpec::runner_id`).
pub const CHOICE: &str = "burst";
/// Port of the runner daemon in the job (the runner's default), exposed through the HF Jobs proxy.
const PORT: u16 = 7320;
/// HF Jobs' own limit for one job; the runner stops sooner on its own when Home is gone (`BURST_IDLE`).
const MAX_SECONDS: u64 = 24 * 3600;
/// How long Home waits for a new job's runner to connect.
const CONNECT_TIMEOUT_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BurstState {
    /// The job is starting; Home dials it until its runner answers.
    Starting,
    Running,
    /// The session was handed off; the job stops after Home's next backup.
    Stopping,
    Stopped,
    Failed,
}

/// What a new job does once its runner is connected.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    Launch { spec: Box<LaunchSpec>, session_id: String, automatic: bool },
    Message { session_id: String, text: String, #[serde(default)] files: Vec<crate::model::FileAttachment>, key: Option<String> },
    /// Move to Burst / Continue on Burst, without a message.
    Move { session_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct BurstRun {
    pub runner_id: String,
    pub job_id: Option<String>,
    pub namespace: String,
    pub flavor: String,
    /// The runner's address through the HF Jobs proxy.
    pub url: Option<String>,
    pub state: BurstState,
    /// The session it runs.
    pub session_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub connected_at: Option<i64>,
    #[ts(type = "number | null")]
    pub stopped_at: Option<i64>,
    /// HF Jobs price of the flavor when it started.
    pub cost_per_minute: f64,
    pub error: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub pending: Option<Pending>,
}

impl BurstRun {
    pub fn active(&self) -> bool {
        matches!(self.state, BurstState::Starting | BurstState::Running | BurstState::Stopping)
    }

    /// Cost so far (USD): billed minutes from creation to stop.
    pub fn cost(&self, now: i64) -> f64 {
        let end = self.stopped_at.unwrap_or(now);
        ((end - self.created_at).max(0) as f64 / 60_000.0) * self.cost_per_minute
    }
}

/// Burst in Settings → Home and runners.
#[derive(Clone, Debug, Serialize, TS)]
pub struct BurstOverview {
    pub config: crate::config::BurstConfig,
    /// Why Home cannot start a Burst runner now, or None.
    pub unavailable: Option<String>,
    /// This month's Burst cost so far (UTC month).
    pub month_usd: f64,
    pub runs: Vec<BurstRun>,
}

// ------------------------------------------------------------------ records

async fn save(app: &App, r: &BurstRun) -> Result<()> {
    let (id, at, json) = (r.runner_id.clone(), r.created_at, serde_json::to_string(r)?);
    app.state_db
        .call(move |c| Ok(c.execute("INSERT INTO burst_run (runner_id, created_at, json) VALUES (?1, ?2, ?3) ON CONFLICT (runner_id) DO UPDATE SET json = ?3", rusqlite::params![id, at, json])?))
        .await?;
    app.bus.publish("burst", "changed", Value::Null);
    Ok(())
}

/// Runs, newest first.
pub async fn runs(app: &App) -> Result<Vec<BurstRun>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM burst_run ORDER BY created_at DESC LIMIT 500")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await
}

pub async fn get(app: &App, runner_id: &str) -> Result<Option<BurstRun>> {
    Ok(runs(app).await?.into_iter().find(|r| r.runner_id == runner_id))
}

async fn update(app: &App, runner_id: &str, f: impl FnOnce(&mut BurstRun)) -> Result<BurstRun> {
    let _lock = LOCK.lock().await;
    let mut r = get(app, runner_id).await?.context("no such Burst runner")?;
    f(&mut r);
    save(app, &r).await?;
    Ok(r)
}

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn month_start(now_ms: i64) -> i64 {
    let t = time::OffsetDateTime::from_unix_timestamp(now_ms / 1000).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let first = t.replace_day(1).unwrap_or(t).replace_time(time::Time::MIDNIGHT);
    first.unix_timestamp() * 1000
}

/// This UTC month's cost of Burst runs (the part of each run inside the month).
pub fn month_cost(runs: &[BurstRun], now: i64) -> f64 {
    let start = month_start(now);
    runs.iter()
        .map(|r| {
            let begin = r.created_at.max(start);
            let end = r.stopped_at.unwrap_or(now);
            ((end - begin).max(0) as f64 / 60_000.0) * r.cost_per_minute
        })
        .sum()
}

// ------------------------------------------------------------------ HF Jobs

/// Home's address, for links in the job.
pub fn home_url(app: &App) -> String {
    if let Ok(h) = std::env::var("SPACE_HOST")
        && !h.trim().is_empty()
    {
        return format!("https://{}", h.trim());
    }
    app.config.get().server.public_origin.unwrap_or_else(|| format!("http://127.0.0.1:{}", app.paths.port))
}

/// Home's own image: Burst runners run the same build as Home. `LAUNCHPAD_BURST_IMAGE` overrides it (checks).
fn image() -> Option<String> {
    std::env::var("LAUNCHPAD_BURST_IMAGE").ok().filter(|s| !s.trim().is_empty()).or_else(|| std::env::var("SPACE_ID").ok().filter(|s| s.contains('/')).map(|s| format!("hf.co/spaces/{s}")))
}

/// Where jobs run: Home's namespace.
fn namespace() -> Option<String> {
    std::env::var("LAUNCHPAD_BURST_NAMESPACE").ok().filter(|s| !s.trim().is_empty()).or_else(|| std::env::var("SPACE_ID").ok().and_then(|s| s.split('/').next().map(str::to_string)))
}

async fn token(app: &App) -> Result<String> {
    crate::vault::home_token(app, crate::vault::HomeToken::Huggingface).await.context("Home has no Hugging Face login")
}

fn endpoint() -> String {
    std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".into()).trim_end_matches('/').to_string()
}

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().expect("HF client"));

async fn hf(app: &App, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let mut r = CLIENT.request(method, format!("{}{path}", endpoint())).bearer_auth(token(app).await?);
    if let Some(b) = body {
        r = r.json(&b);
    }
    let res = r.send().await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    ensure!(status.is_success(), "HF Jobs: {}", v["error"].as_str().map(|e| format!("{e} ({status})")).unwrap_or_else(|| format!("{status} {}", text.chars().take(200).collect::<String>())));
    Ok(v)
}

/// The price of a flavor per minute (USD).
async fn price(app: &App, flavor: &str) -> Result<f64> {
    let all = hf(app, reqwest::Method::GET, "/api/jobs/hardware", None).await?;
    all.as_array().and_then(|a| a.iter().find(|h| h["name"] == flavor)).and_then(|h| h["unitCostUSD"].as_f64()).with_context(|| format!("unknown HF Jobs hardware {flavor}"))
}

/// The bootstrap command of a job: the runner daemon, or `LAUNCHPAD_BURST_COMMAND` (checks with another binary).
fn command() -> Vec<String> {
    match std::env::var("LAUNCHPAD_BURST_COMMAND").ok().filter(|s| !s.trim().is_empty()) {
        Some(c) => vec!["sh".into(), "-c".into(), c],
        None => vec!["launchpad".into(), "daemon".into(), "--role".into(), "runner".into()],
    }
}

/// Volumes of a job (`LAUNCHPAD_BURST_VOLUMES`: `bucket:<namespace>/<name>:<mount>[:ro]`, comma-separated).
fn volumes() -> Vec<Value> {
    std::env::var("LAUNCHPAD_BURST_VOLUMES")
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| {
            let mut p = v.trim().splitn(4, ':');
            let (kind, source, mount) = (p.next()?, p.next()?, p.next()?);
            Some(json!({"type": kind, "source": source, "mountPath": mount, "readOnly": p.next() == Some("ro")}))
        })
        .collect()
}

// ------------------------------------------------------------------ lifecycle

/// Why Home cannot start a Burst runner now, or None.
pub async fn unavailable(app: &App) -> Option<String> {
    if app.paths.role != crate::role::Role::Home {
        return Some("Burst runners are started by Home".into());
    }
    let config = app.config.get().home.burst;
    if !config.enabled {
        return Some("Burst is off (Settings → Home and runners)".into());
    }
    if image().is_none() || namespace().is_none() {
        return Some("Burst needs Home to run in a Hugging Face Space".into());
    }
    if token(app).await.is_err() {
        return Some("Home has no Hugging Face login".into());
    }
    if let Some(why) = jobs_refused(app).await {
        return Some(why);
    }
    let spent = month_cost(&runs(app).await.unwrap_or_default(), now_ms());
    if spent >= config.monthly_cap_usd {
        return Some(format!("Burst reached this month's cap (${:.2} of ${:.2}): raise it in Settings → Home and runners", spent, config.monthly_cap_usd));
    }
    None
}

/// Why the Hugging Face account cannot run HF Jobs in Home's namespace, or None (also when it cannot tell).
/// HF Jobs need a PRO account or credits (a Team or Enterprise plan for an organization). Cached ten minutes.
async fn jobs_refused(app: &App) -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(i64, Option<String>)>> = std::sync::Mutex::new(None);
    if let Some((at, why)) = CACHE.lock().unwrap().clone()
        && now_ms() - at < 10 * 60 * 1000
    {
        return why;
    }
    let namespace = namespace()?;
    let me = hf(app, reqwest::Method::GET, "/api/whoami-v2", None).await.ok()?;
    let why = jobs_refusal(&me, &namespace);
    *CACHE.lock().unwrap() = Some((now_ms(), why.clone()));
    why
}

/// Whether a whoami-v2 answer allows HF Jobs in `namespace`, as a reason to show, or None.
fn jobs_refusal(me: &Value, namespace: &str) -> Option<String> {
    let flag = |v: &Value, k: &str| v[k].as_bool().unwrap_or(false);
    if me["name"].as_str() == Some(namespace) {
        return (!flag(me, "isPro") && !flag(me, "canPay")).then(|| format!("Burst needs a Hugging Face PRO account or credits on {namespace} to run HF Jobs: pair a runner instead, or upgrade at https://huggingface.co/pricing"));
    }
    let org = me["orgs"].as_array().and_then(|o| o.iter().find(|o| o["name"].as_str() == Some(namespace)))?;
    let plan = org["plan"].as_str().is_some_and(|p| p == "team" || p == "enterprise");
    (!plan && !flag(org, "canPay")).then(|| format!("Burst needs a Team or Enterprise plan, or credits, on the {namespace} organization to run HF Jobs: pair a runner instead"))
}

pub async fn overview(app: &App) -> Result<BurstOverview> {
    let all = runs(app).await?;
    let now = now_ms();
    Ok(BurstOverview { config: app.config.get().home.burst, unavailable: unavailable(app).await, month_usd: month_cost(&all, now), runs: all.into_iter().take(20).collect() })
}

/// Creates the job of a new Burst runner. The runner connects once Home dials it (`server::burst`).
async fn start(app: &App, session_id: Option<String>, pending: Pending) -> Result<BurstRun> {
    if let Some(reason) = unavailable(app).await {
        bail!("{reason}");
    }
    let config = app.config.get().home.burst;
    let (image, namespace) = (image().context("no image")?, namespace().context("no namespace")?);
    let cost_per_minute = price(app, &config.flavor).await?;
    let (runner, secret) = crate::runners::create_burst(app).await?;
    let mut spec = json!({
        "command": command(),
        "arguments": [],
        "environment": {"LAUNCHPAD_HOME_URL": home_url(app), "LAUNCHPAD_ENV": app.paths.app_env.name(), "LAUNCHPAD_PORT": PORT.to_string()},
        "secrets": {"LAUNCHPAD_BURST_CREDENTIAL": format!("{}.{secret}", runner.id)},
        "flavor": config.flavor,
        "timeoutSeconds": MAX_SECONDS,
        "labels": {"launchpad-burst": runner.id},
        "expose": {"ports": [PORT]},
    });
    match image.strip_prefix("hf.co/spaces/").or_else(|| image.strip_prefix("https://huggingface.co/spaces/")) {
        Some(space) => spec["spaceId"] = json!(space),
        None => spec["dockerImage"] = json!(image),
    }
    let v = volumes();
    if !v.is_empty() {
        spec["volumes"] = json!(v);
    }
    let mut run = BurstRun {
        runner_id: runner.id.clone(),
        job_id: None,
        namespace: namespace.clone(),
        flavor: config.flavor.clone(),
        url: None,
        state: BurstState::Starting,
        session_id,
        created_at: now_ms(),
        connected_at: None,
        stopped_at: None,
        cost_per_minute,
        error: None,
        pending: Some(pending),
    };
    let created = hf(app, reqwest::Method::POST, &format!("/api/jobs/{namespace}"), Some(spec)).await.map_err(|e| {
        let text = format!("{e:#}");
        if text.contains("402 Payment") || text.contains("403 Forbidden") {
            anyhow::anyhow!("Burst could not start an HF Job in {namespace}: HF Jobs need a Hugging Face PRO account or credits. Pair a runner instead. ({text})")
        } else {
            e
        }
    });
    match created {
        Ok(job) => {
            run.job_id = job["id"].as_str().map(str::to_string);
            run.url = job["status"]["exposeUrls"].as_array().and_then(|u| u.iter().filter_map(Value::as_str).find(|u| u.contains(&format!("--{PORT}."))).map(str::to_string));
            save(app, &run).await?;
            tracing::info!("Burst: started job {:?} for runner {}", run.job_id, run.runner_id);
            tokio::spawn(driven(app.clone(), run.runner_id.clone()));
            Ok(run)
        }
        Err(e) => {
            run.state = BurstState::Failed;
            run.error = Some(format!("{e:#}"));
            run.stopped_at = Some(now_ms());
            run.pending = None;
            save(app, &run).await?;
            let _ = crate::runners::remove(app, &run.runner_id).await;
            Err(e)
        }
    }
}

/// Cancels a run's job. Its sessions stay on Home.
pub async fn stop(app: &App, runner_id: &str, error: Option<String>) -> Result<()> {
    let r = get(app, runner_id).await?.context("no such Burst runner")?;
    // What the job was started for never happened: the placeholder fails, a message shows it was not sent.
    let reason = error.clone().or_else(|| r.pending.as_ref().map(|_| "the Burst runner was stopped before it started".to_string()));
    if let Some(e) = &reason {
        match &r.pending {
            Some(Pending::Launch { session_id, .. }) => fail_placeholder(app, session_id).await,
            Some(Pending::Message { session_id, .. }) => fail_continuation(app, session_id, e, true).await,
            Some(Pending::Move { session_id }) => fail_continuation(app, session_id, e, false).await,
            None => {}
        }
    }
    if let Some(job) = &r.job_id
        && r.active()
        && let Err(e) = hf(app, reqwest::Method::POST, &format!("/api/jobs/{}/{job}/cancel", r.namespace), None).await
    {
        tracing::warn!("Burst: cancel {job}: {e:#}");
    }
    update(app, runner_id, |r| {
        if r.active() {
            r.state = if error.is_some() { BurstState::Failed } else { BurstState::Stopped };
            r.stopped_at = Some(now_ms());
        }
        if error.is_some() {
            r.error = error;
        }
        r.pending = None;
    })
    .await?;
    // Its credential ends with the job; the run record keeps what the UI shows.
    crate::runners::remove(app, runner_id).await?;
    Ok(())
}

/// Home dialed the runner (`server::burst`).
pub async fn connected(app: &App, runner_id: &str) {
    let _ = update(app, runner_id, |r| {
        if r.state == BurstState::Starting {
            r.state = BurstState::Running;
            r.connected_at = Some(now_ms());
        }
    })
    .await;
}

/// Runs Home should keep dialing: their job is starting or running.
pub async fn to_dial(app: &App) -> Vec<(String, String)> {
    runs(app).await.unwrap_or_default().into_iter().filter(|r| matches!(r.state, BurstState::Starting | BurstState::Running | BurstState::Stopping)).filter_map(|r| r.url.map(|u| (r.runner_id, u))).collect()
}

/// The credential header Home dials a Burst runner with, and its HF token for the Jobs proxy.
pub async fn dial_auth(app: &App, runner_id: &str) -> Result<(String, String)> {
    let secret = crate::runners::burst_secret(app, runner_id).await?.context("no credential for this Burst runner")?;
    Ok((format!("{runner_id}.{secret}"), token(app).await?))
}

/// `drive` behind a box: a message to a stopped Burst session starts a job that sends a message.
fn driven(app: App, id: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move { drive(&app, &id).await })
}

/// Waits for the runner, then does what the job was started for: the launch, or the move and the message.
async fn drive(app: &App, runner_id: &str) {
    let deadline = now_ms() + CONNECT_TIMEOUT_MS;
    while !app.runners.connected(runner_id) {
        if now_ms() > deadline {
            let _ = stop(app, runner_id, Some("the Burst runner did not start within five minutes".into())).await;
            return;
        }
        if get(app, runner_id).await.ok().flatten().is_none_or(|r| !r.active()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let Ok(Some(run)) = get(app, runner_id).await else { return };
    let result = match run.pending.clone() {
        Some(Pending::Launch { spec, session_id, .. }) => async {
            let (status, body) = app.runners.request_with(runner_id, "POST", "/api/sessions", Some(serde_json::to_string(&spec)?), vec![("idempotency-key".into(), session_id)]).await?;
            ensure!((200..300).contains(&status), "{}", error_of(&body));
            Ok(())
        }
        .await,
        Some(Pending::Message { session_id, text, files, key }) => async {
            crate::handoff::move_to(app, &session_id, runner_id).await?;
            crate::runners::send_anywhere(app, &session_id, &text, files, key).await
        }
        .await,
        Some(Pending::Move { session_id }) => crate::handoff::move_to(app, &session_id, runner_id).await.map(|_| ()),
        None => Ok(()),
    };
    match result {
        Ok(()) => {
            let _ = update(app, runner_id, |r| r.pending = None).await;
        }
        Err(e) => {
            tracing::warn!("Burst {runner_id}: {e:#}");
            let _ = stop(app, runner_id, Some(format!("{e:#}"))).await;
        }
    }
}

fn error_of(body: &str) -> String {
    serde_json::from_str::<Value>(body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_else(|| body.to_string())
}

/// Home: a launch that found no runner starts on Burst. The session shows at once (Starting, on its Burst
/// runner); the runner creates it with the same ID once the job is up, and its record replaces this one.
pub async fn launch(app: &App, spec: LaunchSpec, id: String, automatic: bool) -> Result<Session> {
    let harness = spec.harness.context("pick a harness")?;
    ensure!(app.sessions.get(&id).is_none(), "a session with this id already exists");
    if let crate::model::Target::Project { project_id } | crate::model::Target::Predraft { project_id } = &spec.target {
        let p = crate::projects::get(app, project_id).await?.context("no such folder")?;
        ensure!(p.home_git || p.plain, "{} exists only on a runner's disk: Burst cannot open it", p.name);
    }
    let pending = Pending::Launch { spec: Box::new(spec.clone()), session_id: id.clone(), automatic };
    let run = start(app, Some(id.clone()), pending).await?;
    let now = now_ms();
    let title: String = spec.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New agent").chars().take(80).collect();
    let s = Session {
        id: id.clone(),
        harness,
        runner_id: Some(run.runner_id.clone()),
        generation: 1,
        harness_session_id: None,
        project_id: match &spec.target {
            crate::model::Target::Project { project_id } | crate::model::Target::Predraft { project_id } => Some(project_id.clone()),
            _ => None,
        },
        cwd: String::new(),
        title,
        title_pending: true,
        status: SessionStatus::Starting,
        archived_done: false,
        status_since: now,
        live: false,
        open_question: false,
        model: spec.model.clone(),
        effort: spec.effort.clone(),
        prompt: spec.text.clone(),
        usage: Default::default(),
        started_at: now,
        ended_at: None,
        last_seq: 0,
        launch_spec: spec,
        quota_pause: None,
        capacity_retry_count: 0,
        capacity_retry_at: None,
        seen_at: now,
        last_message: None,
    };
    app.sessions.placeholder(app, s.clone()).await?;
    Ok(s)
}

/// A placeholder whose launch never reached a runner.
async fn fail_placeholder(app: &App, id: &str) {
    if app.sessions.get(id).is_some_and(|s| s.last_seq == 0 && s.status == SessionStatus::Starting) {
        let _ = app.sessions.set_replica_status(app, id, SessionStatus::Error).await;
    }
}

/// A message or a move to Burst that failed after the request returned: the session shows Failed with the
/// reason (Home adds no event to a replicated session), and the job's error stays in Settings → Home and runners.
async fn fail_continuation(app: &App, id: &str, error: &str, message: bool) {
    let Some(s) = app.sessions.get(id) else { return };
    if s.status.is_running() || s.status == SessionStatus::Closed {
        return;
    }
    let lost = if message { " Your message was not sent: send it again." } else { "" };
    let text = format!("Burst could not continue this session: {error}.{lost}");
    if let Err(e) = app.sessions.fail_replica(app, id, text).await {
        tracing::warn!("Burst: {id}: {e:#}");
    }
}

/// Home: a message for a session whose Burst job has stopped: a new job takes the session, then the message.
pub async fn continue_session(app: &App, id: &str, text: String, files: Vec<crate::model::FileAttachment>, key: Option<String>) -> Result<()> {
    let s = app.sessions.get(id).context("no such session")?;
    if let Some(reason) = crate::handoff::blocked(&s) {
        bail!("{reason}");
    }
    ensure!(!runs(app).await?.iter().any(|r| r.active() && r.session_id.as_deref() == Some(id) && r.pending.is_some()), "Burst is already starting for this session");
    if s.runner_id.as_ref().is_some_and(|r| !app.runners.connected(r)) {
        crate::handoff::offline_copy(app, &s).await?;
    }
    start(app, Some(id.to_string()), Pending::Message { session_id: id.to_string(), text, files, key }).await.map(|_| ())
}

/// Home: Move to Burst / Continue on Burst: a new job takes the idle session (its next message runs there).
pub async fn move_session(app: &App, id: &str) -> Result<()> {
    let s = app.sessions.get(id).context("no such session")?;
    if let Some(reason) = crate::handoff::blocked(&s) {
        bail!("{reason}");
    }
    ensure!(s.runner_id.is_some(), "this session has no runner");
    ensure!(!runs(app).await?.iter().any(|r| r.active() && r.session_id.as_deref() == Some(id) && r.pending.is_some()), "Burst is already starting for this session");
    start(app, Some(id.to_string()), Pending::Move { session_id: id.to_string() }).await.map(|_| ())
}

/// Whether a message to this session should start Burst: its runner is a Burst runner that is gone, or that
/// is stopping (the session was handed off and Home cancels the job soon, even while it is still connected).
pub async fn needs_new_job(app: &App, s: &Session) -> bool {
    let Some(r) = &s.runner_id else { return false };
    get(app, r).await.ok().flatten().is_some_and(|run| run.state == BurstState::Stopping || (!run.active() && !app.runners.connected(r)))
}

/// Every 15 s on Home: jobs that ended on their own, finished sessions handed off, and stopped after the
/// next complete backup.
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        // A start interrupted by a Home restart is never retried: its job is cancelled.
        for r in runs(&app).await.unwrap_or_default().into_iter().filter(|r| r.active() && r.pending.is_some()) {
            let _ = stop(&app, &r.runner_id, Some("Home restarted while this Burst runner was starting".into())).await;
        }
        let mut handed_off: std::collections::HashMap<String, i64> = Default::default();
        let mut stuck: std::collections::HashMap<String, Stuck> = Default::default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            if let Err(e) = tick(&app, &mut handed_off, &mut stuck).await {
                tracing::warn!("Burst: {e:#}");
            }
        }
    });
}

/// How long a job may keep running after its final handoff starts failing.
const STUCK_LIMIT_MS: i64 = 2 * 3600 * 1000;
/// How long a job waits on an open question before Home says that it keeps billing.
const QUESTION_NOTICE_MS: i64 = 3600 * 1000;

/// A job that cannot stop yet: since when, and whether its session was told.
struct Stuck {
    since: i64,
    told: bool,
}

async fn tick(app: &App, handed_off: &mut std::collections::HashMap<String, i64>, stuck: &mut std::collections::HashMap<String, Stuck>) -> Result<()> {
    let active: Vec<BurstRun> = runs(app).await?.into_iter().filter(BurstRun::active).collect();
    stuck.retain(|id, _| active.iter().any(|r| &r.runner_id == id));
    for r in active {
        // Jobs that ended without Home (timeout, error, cancelled on the Hub).
        if let Some(job) = &r.job_id
            && let Ok(j) = hf(app, reqwest::Method::GET, &format!("/api/jobs/{}/{job}", r.namespace), None).await
            && let Some(stage) = j["status"]["stage"].as_str()
            && matches!(stage, "COMPLETED" | "ERROR" | "CANCELED" | "DELETED")
        {
            let message = j["status"]["message"].as_str().map(str::to_string);
            let error = match stage {
                "ERROR" => Some(message.unwrap_or_else(|| "the job failed".into())),
                _ if r.pending.is_some() => Some(format!("the job ended ({}) before its runner connected", stage.to_lowercase())),
                _ => None,
            };
            stop(app, &r.runner_id, error).await?;
            continue;
        }
        let sessions: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.runner_id.as_deref() == Some(r.runner_id.as_str())).collect();
        if r.state == BurstState::Stopping {
            // A message reached the job between its handoff and Stopping: it runs again, so it is not stopped.
            if sessions.iter().any(|s| s.status.is_running() || s.status == SessionStatus::Starting) {
                handed_off.remove(&r.runner_id);
                update(app, &r.runner_id, |r| r.state = BurstState::Running).await?;
                continue;
            }
            // Stopped once Home's backup holds the handoff (or right away without a backup).
            let since = handed_off.get(&r.runner_id).copied().unwrap_or(0);
            let info = app.home_backups.info();
            let backed_up = app.config.get().home.backup.trim().is_empty() || info.last_complete_at.is_some_and(|t| t > since);
            if backed_up {
                handed_off.remove(&r.runner_id);
                stop(app, &r.runner_id, None).await?;
            }
            continue;
        }
        if r.state != BurstState::Running || r.pending.is_some() || !app.runners.connected(&r.runner_id) {
            continue;
        }
        // An open question keeps the job (its answer goes to this process); say once that it keeps billing.
        if let Some(q) = sessions.iter().find(|s| s.open_question) {
            let entry = stuck.entry(r.runner_id.clone()).or_insert(Stuck { since: now_ms(), told: false });
            if !entry.told && now_ms() - entry.since > QUESTION_NOTICE_MS {
                entry.told = true;
                crate::sessions::notice_anywhere(app, &q.id, &format!("This agent's Burst runner keeps running (about ${:.2} per hour) while it waits for your answer. Answer it, or Stop it, so the runner can stop.", r.cost_per_minute * 60.0)).await;
            }
            continue;
        }
        let idle = !sessions.is_empty() && sessions.iter().all(|s| !s.status.is_running() && s.status != SessionStatus::Starting);
        let unused = sessions.is_empty() && r.connected_at.is_some_and(|t| now_ms() - t > CONNECT_TIMEOUT_MS);
        if !(idle || unused) {
            stuck.remove(&r.runner_id);
            continue;
        }
        let mut failure = None;
        for s in sessions.iter().filter(|s| s.status != SessionStatus::Closed) {
            let why = match app.runners.request(&r.runner_id, "POST", &format!("/api/sessions/{}/handoff", s.id), Some(json!({"final": true}).to_string())).await {
                Ok((status, _)) if (200..300).contains(&status) => continue,
                Ok((_, body)) => error_of(&body),
                Err(e) => format!("{e:#}"),
            };
            tracing::info!("Burst {}: {} kept running: {why}", r.runner_id, s.id);
            failure = Some((s.id.clone(), why));
        }
        match failure {
            None => {
                stuck.remove(&r.runner_id);
                handed_off.insert(r.runner_id.clone(), now_ms());
                update(app, &r.runner_id, |r| r.state = BurstState::Stopping).await?;
            }
            // The branches are on Home after every turn: a job that cannot hand off is told, then stopped.
            Some((session, why)) => {
                let entry = stuck.entry(r.runner_id.clone()).or_insert(Stuck { since: now_ms(), told: false });
                if !entry.told {
                    entry.told = true;
                    crate::sessions::notice_anywhere(app, &session, &format!("This agent's Burst runner cannot stop yet: {why}. Ask the agent to fix it. Otherwise the runner stops in two hours, and files not committed there are lost.")).await;
                }
                if now_ms() - entry.since > STUCK_LIMIT_MS {
                    stuck.remove(&r.runner_id);
                    stop(app, &r.runner_id, Some(format!("stopped after two hours without a handoff: {why}"))).await?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_need_pro_credits_or_a_paid_org() {
        let me = json!({"name": "ada", "isPro": false, "canPay": false, "orgs": [{"name": "team", "plan": "team"}, {"name": "free", "canPay": false}]});
        assert!(jobs_refusal(&me, "ada").is_some());
        assert!(jobs_refusal(&json!({"name": "ada", "isPro": true}), "ada").is_none());
        assert!(jobs_refusal(&json!({"name": "ada", "canPay": true}), "ada").is_none());
        assert!(jobs_refusal(&me, "team").is_none());
        assert!(jobs_refusal(&me, "free").is_some());
        // Not a namespace of this account: Home cannot tell, the job creation answers.
        assert!(jobs_refusal(&me, "other").is_none());
    }

    fn run(created: i64, stopped: Option<i64>) -> BurstRun {
        BurstRun { runner_id: "r".into(), job_id: None, namespace: "n".into(), flavor: "cpu-basic".into(), url: None, state: BurstState::Stopped, session_id: None, created_at: created, connected_at: None, stopped_at: stopped, cost_per_minute: 0.01, error: None, pending: None }
    }

    #[test]
    fn month_cost_counts_only_this_month() {
        // 2026-11-05 12:00 UTC; the month starts 2026-11-01 00:00 UTC.
        let now = 1_793_880_000_000;
        let start = month_start(now);
        assert_eq!(start, 1_793_491_200_000);
        let runs = [run(start - 60_000, Some(start + 60_000)), run(start + 600_000, None), run(start - 120_000, Some(start - 60_000))];
        let cost = month_cost(&runs, start + 1_200_000);
        // One minute of the first run, ten of the running one, none of the last.
        assert!((cost - 0.11).abs() < 1e-9, "{cost}");
    }
}
