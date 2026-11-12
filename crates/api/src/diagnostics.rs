//! Diagnostic reads for sessions with admin Launchpad CLI access (`agent_auth`): the recent log of Home and
//! runners (`/api/logs`), runners, Burst runs and their HF Jobs, Home's starts, backups and update, the failure
//! feed, tools, MCP servers and connections, and `health`, a summary of everything failing now. These routes return
//! no secret: no token, vault value, runner secret, webhook URL or pairing code; log lines are redacted when they
//! are written (`launchpad_http::logs`).
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use launchpad_core::model::{Session, SessionStatus};
use launchpad_core::role::Role;
use launchpad_core::util::now_ms;
use launchpad_http::logs;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/logs", get(read_logs))
        .route("/api/context/runners", get(runners))
        .route("/api/context/burst", get(burst))
        .route("/api/context/burst/{runner}/job", get(burst_job))
        .route("/api/context/home", get(home))
        .route("/api/context/failures", get(failures))
        .route("/api/context/tools", get(tools))
        .route("/api/context/connections", get(connections))
        .route("/api/context/health", get(health))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

// ------------------------------------------------------------------ logs

#[derive(Deserialize, Default)]
struct LogQuery {
    /// `local` (this daemon), `home`, a runner id, or `all` (Home and every connected runner; the default).
    source: Option<String>,
    /// `30m`, `2h`, `1d` or Unix ms (default: the last hour).
    since: Option<String>,
    until: Option<i64>,
    level: Option<String>,
    grep: Option<String>,
    limit: Option<usize>,
    /// Lines about this session: its id, its short id and the ids of its Burst runners.
    session: Option<String>,
    /// Comma-separated words, one of which must appear (Home passes the session's words to runners).
    words: Option<String>,
}

fn local_lines(s: &Ctx, filter: logs::Filter, source: &str) -> Vec<Value> {
    logs::query(&s.app.paths.data_dir, &filter).into_iter().map(|l| json!({"at": l.at, "level": l.level, "target": l.target, "message": l.message, "source": source})).collect()
}

/// `GET /api/logs`: the recent log lines, oldest first, with their source. A runner forwards every source but
/// `local` to Home (`home_client::forward_writes`); Home asks each runner for its own lines.
async fn read_logs(State(s): State<Ctx>, Query(q): Query<LogQuery>) -> ApiResult<Value> {
    let now = now_ms();
    let since = match q.since.as_deref() {
        Some(x) => Some(logs::parse_since(x, now).ok_or_else(|| ApiError::bad_request("since takes 30m, 2h, 1d or Unix milliseconds"))?),
        None => Some(now - 3_600_000),
    };
    let mut words: Vec<String> = q.words.as_deref().unwrap_or("").split(',').map(str::trim).filter(|w| !w.is_empty()).map(str::to_string).collect();
    if let Some(id) = q.session.as_deref().filter(|id| !id.is_empty()) {
        words.push(id.to_string());
        if id.len() > 8 {
            words.push(id[..8].to_string());
        }
        for r in launchpad_core::burst::runs(&s.app).await.unwrap_or_default().into_iter().filter(|r| r.session_id.as_deref() == Some(id)) {
            words.push(r.runner_id.clone());
            words.extend(r.job_id);
        }
    }
    let filter = logs::Filter { since, until: q.until, level: q.level.clone(), any_of: words.clone(), grep: q.grep.clone(), limit: q.limit };
    let source = q.source.as_deref().unwrap_or("all");
    let role = s.app.paths.role;
    let own = if role == Role::Runner { "runner" } else { "home" };
    if source == "local" || role == Role::Runner || (source == "home" && role != Role::Runner) {
        let f = filter.clone();
        let sc = s.clone();
        let lines = tokio::task::spawn_blocking(move || local_lines(&sc, f, own)).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
        return Ok(Json(json!({"lines": lines, "errors": []})));
    }
    let all = launchpad_core::runners::list(&s.app).await?;
    let targets: Vec<_> = match source {
        "all" => all.iter().filter(|r| r.connected).collect(),
        id => vec![all.iter().find(|r| r.runner.id == id || r.runner.name.eq_ignore_ascii_case(id)).ok_or_else(|| ApiError::not_found(format!("no runner {id} (see `launchpad runners list`)")))?],
    };
    let mut path = format!("/api/logs?source=local&since={}", since.unwrap_or(0));
    let mut push = |k: &str, v: &str| path.push_str(&format!("&{k}={}", urlencode(v)));
    if let Some(u) = q.until { push("until", &u.to_string()); }
    if let Some(l) = &q.level { push("level", l); }
    if let Some(g) = &q.grep { push("grep", g); }
    if let Some(l) = q.limit { push("limit", &l.to_string()); }
    if !words.is_empty() { push("words", &words.join(",")); }
    let asks = targets.iter().map(|r| {
        let (app, path, id, name) = (s.app.clone(), path.clone(), r.runner.id.clone(), r.runner.name.clone());
        async move {
            let label = format!("runner {name} ({id})");
            match app.runners.request(&id, "GET", &path, None).await {
                Ok((200, body)) => Ok(serde_json::from_str::<Value>(&body).ok().and_then(|v| v["lines"].as_array().cloned()).unwrap_or_default().into_iter().map(|mut l| { l["source"] = json!(label); l }).collect::<Vec<_>>()),
                Ok((status, body)) => Err(json!({"source": label, "error": format!("{status}: {}", body.chars().take(300).collect::<String>())})),
                Err(e) => Err(json!({"source": label, "error": format!("{e:#}")})),
            }
        }
    });
    let results = futures_util::future::join_all(asks).await;
    let mut lines = if source == "all" {
        let sc = s.clone();
        let f = filter.clone();
        tokio::task::spawn_blocking(move || local_lines(&sc, f, "home")).await.map_err(|e| ApiError::bad_request(e.to_string()))?
    } else {
        vec![]
    };
    let mut errors = vec![];
    for r in results {
        match r {
            Ok(l) => lines.extend(l),
            Err(e) => errors.push(e),
        }
    }
    if source == "all" {
        for r in all.iter().filter(|r| !r.connected && r.runner.state == launchpad_core::runners::RunnerState::Approved && !r.runner.burst) {
            errors.push(json!({"source": format!("runner {} ({})", r.runner.name, r.runner.id), "error": "offline: its log is read when it connects again"}));
        }
    }
    lines.sort_by_key(|l| l["at"].as_i64().unwrap_or(0));
    let extra = lines.len().saturating_sub(filter.limit());
    lines.drain(..extra);
    Ok(Json(json!({"lines": lines, "errors": errors})))
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// ------------------------------------------------------------------ runners and Burst

fn runner_json(r: &launchpad_core::runners::RunnerStatus, sessions: &[Session]) -> Value {
    let mine: Vec<&Session> = sessions.iter().filter(|s| s.runner_id.as_deref() == Some(&r.runner.id) && s.status != SessionStatus::Closed).collect();
    json!({
        "id": r.runner.id, "name": r.runner.name, "os": r.runner.os, "state": r.runner.state, "burst": r.runner.burst,
        "connected": r.connected, "compat": r.compat, "guard": r.guard, "commit": r.runner.commit, "protocol": r.runner.protocol,
        "capabilities": r.runner.capabilities, "created_at": r.runner.created_at, "approved_at": r.runner.approved_at, "last_seen_at": r.runner.last_seen_at,
        "network_check": r.network_check,
        "sessions": mine.iter().map(|s| json!({"id": s.id, "title": s.title, "status": s.status})).collect::<Vec<_>>(),
    })
}

/// Runners with their connection, build, guard and open sessions (no pairing code or secret).
async fn runners(State(s): State<Ctx>) -> ApiResult<Value> {
    let sessions = s.app.sessions.list();
    let list = launchpad_core::runners::list(&s.app).await?;
    Ok(Json(json!(list.iter().map(|r| runner_json(r, &sessions)).collect::<Vec<_>>())))
}

fn run_json(r: &launchpad_core::burst::BurstRun, now: i64) -> Value {
    let pending = r.pending.as_ref().map(|p| match p {
        launchpad_core::burst::Pending::Launch { session_id, .. } => json!({"kind": "launch", "session_id": session_id}),
        launchpad_core::burst::Pending::Request { session_id, action, .. } => json!({"kind": "request", "action": action, "session_id": session_id}),
        launchpad_core::burst::Pending::Move { session_id } => json!({"kind": "move", "session_id": session_id}),
    });
    json!({
        "runner_id": r.runner_id, "job_id": r.job_id, "namespace": r.namespace, "flavor": r.flavor, "state": r.state, "session_id": r.session_id,
        "created_at": r.created_at, "connected_at": r.connected_at, "stopped_at": r.stopped_at, "error": r.error, "cost_usd": r.cost(now),
        "waiting_for": pending,
    })
}

#[derive(Deserialize)]
struct BurstQuery {
    session: Option<String>,
    limit: Option<usize>,
}

/// Burst: its settings, why it cannot start now, this month's cost and the runs (newest first) with their errors.
async fn burst(State(s): State<Ctx>, Query(q): Query<BurstQuery>) -> ApiResult<Value> {
    let o = launchpad_core::burst::overview(&s.app).await.map_err(bad)?;
    let now = now_ms();
    let runs: Vec<Value> = o.runs.iter().filter(|r| q.session.as_deref().is_none_or(|id| r.session_id.as_deref() == Some(id))).take(q.limit.unwrap_or(50)).map(|r| run_json(r, now)).collect();
    Ok(Json(json!({"config": o.config, "unavailable": o.unavailable, "month_usd": o.month_usd, "runs": runs})))
}

/// The HF Job of a Burst run: stage, message and the last lines of its log (redacted).
async fn burst_job(State(s): State<Ctx>, Path(runner): Path<String>) -> ApiResult<Value> {
    let mut v = launchpad_core::burst::job_info(&s.app, &runner).await.map_err(bad)?;
    if let Some(lines) = v["log"].as_array_mut() {
        for l in lines.iter_mut() {
            *l = json!(logs::redact(l.as_str().unwrap_or_default()));
        }
    }
    if let Some(m) = v["message"].as_str() {
        v["message"] = json!(logs::redact(m));
    }
    Ok(Json(v))
}

// ------------------------------------------------------------------ Home

/// Home: when this process started, the last starts (a start after a run that did not stop cleanly is a crash,
/// a kill or a power loss), backups and the Home update state.
async fn home(State(s): State<Ctx>) -> ApiResult<Value> {
    let starts = launchpad_automation::failures::starts(&s.app).await?;
    let now = now_ms();
    let started = launchpad_automation::failures::started_at();
    let backup = s.app.home_backups.info();
    let update = launchpad_setup::home_update::current(&s.app);
    Ok(Json(json!({
        "role": s.app.paths.role.name(), "commit": launchpad_core::commit(), "started_at": started, "uptime_seconds": started.map(|t| (now - t) / 1000),
        "this_start": launchpad_automation::failures::restart_reason(starts.get(1)),
        "starts": starts.iter().take(20).map(|x| json!({"at": x.at, "commit": x.commit, "role": x.role, "stopped_at": x.stopped_at, "clean": x.stopped_at.is_some()})).collect::<Vec<_>>(),
        "backup": {"destination_set": backup.destination.is_some(), "last_complete_at": backup.last_complete_at, "error": backup.error, "traces_error": backup.traces_error, "pruned_at": backup.pruned_at, "prune": backup.prune, "prune_error": backup.prune_error, "missing_blobs": backup.missing_blobs},
        "update": {"space": update.space, "deployed": update.deployed, "running": update.running, "available": update.available, "building": update.building, "stage": update.stage, "checked_at": update.checked_at, "error": update.error},
    })))
}

#[derive(Deserialize)]
struct Since {
    since: Option<String>,
}

/// The failures Home noticed (sessions, Burst jobs, routine runs, scheduled launches, runners, restarts), oldest
/// first; `since` defaults to the last 24 hours.
async fn failures(State(_): State<Ctx>, Query(q): Query<Since>) -> ApiResult<Value> {
    let now = now_ms();
    let since = q.since.as_deref().and_then(|x| logs::parse_since(x, now)).unwrap_or(now - 86_400_000);
    Ok(Json(json!(launchpad_automation::failures::recent(since))))
}

// ------------------------------------------------------------------ tools and connections

/// Tools (no token), vault entries by name and kind only, and MCP servers with their last refresh and error.
async fn tools(State(s): State<Ctx>) -> ApiResult<Value> {
    let tools = launchpad_core::vault::tools(&s.app).await?;
    let entries = launchpad_core::vault::entries(&s.app).await?;
    let mcp = launchpad_core::mcp::list(&s.app).await?;
    Ok(Json(json!({
        "tools": tools.iter().map(|t| json!({"id": t.id, "name": t.name, "credential": t.credential, "credential_set": t.credential.as_ref().is_none_or(|c| entries.iter().any(|e| &e.id == c)), "workspaces": t.workspaces, "hosts": t.hosts.iter().map(|h| &h.host).collect::<Vec<_>>(), "installs": t.install.is_some(), "check_url": t.check_url})).collect::<Vec<_>>(),
        "credentials": entries.iter().map(|e| json!({"id": e.id, "kind": e.kind, "name": e.name, "updated_at": e.updated_at})).collect::<Vec<_>>(),
        "mcp_servers": mcp.iter().map(|m| json!({"id": m.id, "name": m.name, "host": reqwest_host(&m.url), "workspaces": m.workspaces, "tools": m.tools.len(), "tools_off": m.tools_off, "refreshed_at": m.refreshed_at, "error": m.error.as_deref().map(logs::redact)})).collect::<Vec<_>>(),
    })))
}

fn reqwest_host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or_default().rsplit('@').next().unwrap_or_default().to_string()
}

/// Connections by name, kind and host, and each workspace's Telegram link: its state and last error (no URL, token
/// or code).
async fn connections(State(s): State<Ctx>) -> ApiResult<Value> {
    let list = launchpad_channels::connections::list(&s.app).await.unwrap_or_default();
    let telegram = launchpad_channels::telegram::statuses(&s.app).await;
    Ok(Json(json!({
        "connections": list.iter().map(|c| json!({"id": c.id, "name": c.name, "kind": c.kind, "host": c.host})).collect::<Vec<_>>(),
        "telegram": telegram.iter().map(|t| json!({"workspace": t.workspace_id, "configured": t.configured, "bound": t.bound, "bot_username": t.bot_username, "notify": t.notify, "error": t.error.as_deref().map(logs::redact)})).collect::<Vec<_>>(),
    })))
}

// ------------------------------------------------------------------ health

/// Everything failing now, by area, each problem with the ids to investigate it. Empty `problems`: all is well.
async fn health(State(s): State<Ctx>) -> ApiResult<Value> {
    let app = &s.app;
    let now = now_ms();
    let day = now - 86_400_000;
    let mut problems: Vec<Value> = vec![];
    let mut add = |area: &str, what: String, detail: Option<String>, ids: Value| problems.push(json!({"area": area, "what": what, "detail": detail, "ids": ids}));

    // Home.
    let starts = launchpad_automation::failures::starts(app).await.unwrap_or_default();
    for (i, st) in starts.iter().enumerate().filter(|(_, st)| st.at > day && st.role != "runner") {
        if let Some(prev) = starts.get(i + 1).filter(|p| p.stopped_at.is_none()) {
            add("home", format!("Home started at {} after a run that did not stop cleanly", st.at), Some(format!("previous build {}", prev.commit)), json!({"start": st.at}));
        }
    }
    let backup = app.home_backups.info();
    if let Some(e) = backup.error.clone() { add("backup", "Home backup failed".into(), Some(e), json!({})); }
    if let Some(e) = backup.traces_error.clone() { add("backup", "Trace copies failed".into(), Some(e), json!({})); }
    if let Some(e) = backup.prune_error.clone() { add("backup", "Pruning old Home backups was skipped".into(), Some(e), json!({})); }
    if let Some(e) = backup.missing_blobs.clone() { add("backup", "Files missing from Home backups".into(), Some(e), json!({})); }
    let update = launchpad_setup::home_update::current(app);
    if let Some(e) = update.error.clone() { add("home_update", "Home update check failed".into(), Some(e), json!({})); }

    // Sessions.
    let sessions = app.sessions.list();
    for x in sessions.iter().filter(|x| matches!(x.status, SessionStatus::Error | SessionStatus::Interrupted) && x.status_since > now - 7 * 86_400_000).take(15) {
        let reason = launchpad_automation::failures::session_reason(app, x).await;
        add("sessions", format!("“{}” is {}", x.title, x.status.as_str()), reason, json!({"session": x.id, "runner": x.runner_id, "since": x.status_since}));
    }
    for x in sessions.iter().filter(|x| x.status == SessionStatus::Starting && now - x.status_since > 5 * 60_000) {
        add("sessions", format!("“{}” has been starting for {} min", x.title, (now - x.status_since) / 60_000), None, json!({"session": x.id, "runner": x.runner_id}));
    }
    for x in sessions.iter().filter(|x| x.quota_pause.is_some() && x.status != SessionStatus::Closed) {
        add("sessions", format!("“{}” is paused by quota", x.title), None, json!({"session": x.id}));
    }

    // Runners and Burst.
    let runners = launchpad_core::runners::list(app).await.unwrap_or_default();
    for r in &runners {
        let open: Vec<&str> = sessions.iter().filter(|x| x.runner_id.as_deref() == Some(&r.runner.id) && x.status != SessionStatus::Closed).map(|x| x.id.as_str()).collect();
        if !r.connected && r.runner.state == launchpad_core::runners::RunnerState::Approved && !open.is_empty() && !r.runner.burst {
            add("runners", format!("Runner {} is offline with {} open session(s)", r.runner.name, open.len()), r.runner.last_seen_at.map(|t| format!("last seen {t}")), json!({"runner": r.runner.id, "sessions": open}));
        }
        if matches!(r.compat, Some(launchpad_core::role::Compat::Outdated | launchpad_core::role::Compat::Incompatible)) {
            add("runners", format!("Runner {} runs another build than Home", r.runner.name), Some(format!("{:?}, runner build {}", r.compat, r.runner.commit)), json!({"runner": r.runner.id}));
        }
        if r.guard.as_ref().is_some_and(|g| g.paused) {
            add("runners", format!("Runner {} is paused", r.runner.name), None, json!({"runner": r.runner.id}));
        }
    }
    if let Ok(o) = launchpad_core::burst::overview(app).await {
        for r in o.runs.iter().filter(|r| r.state == launchpad_core::burst::BurstState::Failed && r.stopped_at.unwrap_or(r.created_at) > day) {
            add("burst", format!("Burst job {} failed", r.job_id.as_deref().unwrap_or("(not created)")), r.error.clone(), json!({"runner": r.runner_id, "session": r.session_id, "job": r.job_id}));
        }
        // Only a Home on a Space (or a test image) can start Burst runners: elsewhere this is no problem.
        let possible = app.paths.role == Role::Home && (std::env::var_os("SPACE_ID").is_some() || std::env::var_os("LAUNCHPAD_BURST_IMAGE").is_some());
        if o.config.enabled && possible && let Some(why) = o.unavailable.clone() {
            add("burst", "Burst cannot start a runner now".into(), Some(why), json!({}));
        }
    }

    // Routines, schedules and proposals.
    for r in launchpad_automation::crons::list(app).await.unwrap_or_default() {
        if let Some(run) = r.history.last().filter(|x| (x.missed || matches!(x.outcome, launchpad_automation::crons::RunOutcome::Failed | launchpad_automation::crons::RunOutcome::Interrupted)) && x.at > day) {
            add("routines", format!("Routine “{}”: last run {:?}", r.title, run.outcome).to_lowercase(), run.detail.clone(), json!({"routine": r.id, "session": run.session_id, "bot": r.bot_id}));
        }
    }
    for l in launchpad_automation::schedules::list(app).await.unwrap_or_default().into_iter().filter(|l| l.status == launchpad_automation::schedules::ScheduleStatus::Failed) {
        add("schedules", format!("Scheduled launch “{}” failed", l.title), l.error.clone(), json!({"schedule": l.id}));
    }
    for p in launchpad_automation::proposals::list(app).await.unwrap_or_default().into_iter().filter(|p| matches!(p.state, launchpad_automation::proposals::ProposalState::Failed | launchpad_automation::proposals::ProposalState::Uncertain) && p.updated_at > now - 7 * 86_400_000) {
        add("proposals", format!("Card “{}” is {:?}", p.title, p.state).to_lowercase(), p.error.clone(), json!({"proposal": p.id, "session": p.session_id}));
    }

    // Quota, harnesses, tools, connections, GitHub.
    let status = app.status.get();
    for (login, q) in &status.quotas {
        if let Some(e) = &q.quota_error { add("quota", format!("Quota of {login} ({}) cannot be read", q.harness), Some(e.clone()), json!({"profiles": q.profiles})); }
    }
    for (login, hold) in launchpad_core::quota::holds(now).into_iter().filter(|(_, h)| now - h.since >= launchpad_automation::failures::QUOTA_HOLD_MS) {
        let q = status.quotas.get(&login).cloned().unwrap_or_default();
        let what = format!("Automatic work (routines, scheduled launches) has waited {} min for a quota reading of {}", (now - hold.since) / 60_000, launchpad_core::quota::login_label(&login, &q));
        add("quota", what, Some(launchpad_core::quota::reading_detail(&q, now)), json!({"login": login, "profiles": q.profiles, "since": hold.since}));
    }
    for (h, st) in &status.harnesses {
        if let Some(e) = &st.version_error { add("harnesses", format!("{h} version check failed"), Some(e.clone()), json!({})); }
    }
    for m in launchpad_core::mcp::list(app).await.unwrap_or_default() {
        if let Some(e) = &m.error { add("mcp", format!("MCP server {} failed its last refresh", m.name), Some(logs::redact(e)), json!({"mcp": m.id})); }
    }
    for t in launchpad_channels::telegram::statuses(app).await {
        if let Some(e) = t.error { add("connections", "Telegram failed".into(), Some(logs::redact(&e)), json!({"workspace": t.workspace_id})); }
    }
    if let Ok(snap) = launchpad_github::inbox::of(app).snapshot(app).await {
        if let Some(e) = snap.status.error { add("github", "Inbox sync failed".into(), Some(e), json!({"last_sync": snap.status.last_sync})); }
        for (host, l) in snap.limits.iter().filter(|(_, l)| l.remaining <= 0 && l.reset_at * 1000 > now) {
            add("github", format!("GitHub rate limit reached ({host})"), Some(format!("resets at {}", l.reset_at)), json!({}));
        }
    }

    let started = launchpad_automation::failures::started_at();
    Ok(Json(json!({
        "ok": problems.is_empty(), "checked_at": now, "home": {"commit": launchpad_core::commit(), "started_at": started, "uptime_seconds": started.map(|t| (now - t) / 1000)},
        "runners": {"connected": runners.iter().filter(|r| r.connected).count(), "total": runners.len()},
        "problems": problems, "recent_failures": launchpad_automation::failures::recent(day).len(),
    })))
}

/// A session with what diagnosis needs: runner, generation, launch spec, the last error, its Burst runs and its
/// latest handoff (names and hashes of the native files, never their content).
pub async fn session_detail(s: &Ctx, x: &Session, base: Value) -> Value {
    let mut v = base;
    let light = x.light();
    v["launch_spec"] = json!(light.launch_spec);
    v["title_pending"] = json!(x.title_pending);
    v["capacity_retry_count"] = json!(x.capacity_retry_count);
    v["capacity_retry_at"] = json!(x.capacity_retry_at);
    v["quota_pause"] = json!(x.quota_pause);
    v["last_seq"] = json!(x.last_seq);
    v["failure"] = json!(launchpad_automation::failures::session_reason(&s.app, x).await);
    if let Some(r) = &x.runner_id {
        let runners = launchpad_core::runners::list(&s.app).await.unwrap_or_default();
        v["runner"] = runners.iter().find(|x| &x.runner.id == r).map(|r| json!({"id": r.runner.id, "name": r.runner.name, "burst": r.runner.burst, "connected": r.connected})).unwrap_or(json!({"id": r, "removed": true}));
    }
    let now = now_ms();
    let runs: Vec<Value> = launchpad_core::burst::runs(&s.app).await.unwrap_or_default().iter().filter(|r| r.session_id.as_deref() == Some(&x.id)).map(|r| run_json(r, now)).collect();
    if !runs.is_empty() {
        v["burst_runs"] = json!(runs);
    }
    let id = x.id.clone();
    let row: Option<(u32, String, i64, String)> = s.app.state_db.call(move |c| Ok(c.query_row("SELECT generation, runner_id, at, json FROM session_handoff WHERE session_id = ?1", [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).ok())).await.ok().flatten();
    if let Some((generation, runner, at, json)) = row {
        let h: Value = serde_json::from_str(&json).unwrap_or_default();
        v["handoff"] = json!({
            "generation": generation, "runner_id": runner, "at": at, "final": h["final"], "cwd": h["cwd"],
            "files": h["files"].as_array().map(|f| f.iter().map(|f| json!({"name": f["name"], "sha256": f["sha256"]})).collect::<Vec<_>>()),
            "worktrees": h["worktrees"].as_array().map(|w| w.iter().map(|w| json!({"project_id": w["project_id"], "branch": w["branch"], "state": w["state"]})).collect::<Vec<_>>()),
        });
    }
    v
}
