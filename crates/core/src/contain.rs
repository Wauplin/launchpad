//! Contained sessions: every agent on a runner (or an `all` daemon) runs in a jail with a lease from Home.
//!
//! Before a harness starts, the runner asks Home for the session's lease, keeps its secrets in the proxy's
//! memory, writes the session's harness home (`<data_dir>/sessions/<id>/home`: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
//! Git config) with placeholders only, installs the granted tools and builds the jail. The runner never uses the
//! machine's own clones, Git config, SSH keys, gh or harness logins. Every contained process runs under the
//! network capture (`netcap.rs`), also on a Local VM or Burst where the machine itself is the boundary.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::App;
use crate::model::Session;
use crate::proxy::Access;
use crate::sessions::StartSpec;
use crate::util::now_ms;
use crate::vault::{Lease, LeaseRequest};

pub fn session_dir(app: &App, id: &str) -> PathBuf {
    app.paths.data_dir.join("sessions").join(id)
}

/// The harness home of a session: `HOME` inside its jail.
pub fn harness_home(app: &App, id: &str) -> PathBuf {
    session_dir(app, id).join("home")
}

/// Whether sessions of this daemon run contained: everywhere agents run, on Linux.
pub fn contained(app: &App) -> bool {
    app.paths.role.runs_agents() && cfg!(target_os = "linux")
}

/// Variables that place a session's harness state in its own home (also used to find native traces).
pub fn home_env(app: &App, id: &str) -> Vec<(String, String)> {
    let home = harness_home(app, id);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    vec![
        ("HOME".into(), s(&home)),
        ("CLAUDE_CONFIG_DIR".into(), s(&home.join(".claude"))),
        ("CODEX_HOME".into(), s(&home.join(".codex"))),
        ("XDG_CONFIG_HOME".into(), s(&home.join(".config"))),
        ("XDG_DATA_HOME".into(), s(&home.join(".local/share"))),
        ("XDG_STATE_HOME".into(), s(&home.join(".local/state"))),
    ]
}

/// The lease request of a session.
async fn request(app: &App, s: &Session) -> Result<LeaseRequest> {
    let workspace_id = match &s.project_id {
        Some(p) => crate::projects::get(app, p).await?.and_then(|p| p.workspace_id),
        None => s.launch_spec.workspace_id.clone(),
    };
    let login = s.launch_spec.profile.as_deref().and_then(|id| app.config.get().profile(id).and_then(|p| p.login.clone()));
    Ok(LeaseRequest { session_id: s.id.clone(), generation: s.generation, harness: s.harness, login, workspace_id, tools_off: s.launch_spec.tools_off.clone(), tools_on: s.launch_spec.tools_on.clone(), project_id: s.project_id.clone(), bot_id: s.launch_spec.bot.as_ref().map(|b| b.id.clone()) })
}

/// Sends a lease call to Home (or answers it here in an `all` daemon).
async fn call(app: &App, op: &str, body: Value) -> Result<Value> {
    if crate::vault::here(app) {
        return crate::vault::runner_call(app, None, op, body).await;
    }
    crate::home_link::home_call(op, body).await
}

static LOOP: std::sync::Once = std::sync::Once::new();

/// Prepares a contained start: lease, proxy, harness home, tools and jail. Fails visibly without a login.
pub async fn apply(app: &App, mut start: StartSpec) -> Result<StartSpec> {
    if !contained(app) {
        return Ok(start);
    }
    let s = app.sessions.get(&start.session_id).context("no such session")?;
    let port = app.proxy.ensure(app).await?;
    // A running session (a terminal next to its agent, a replaced process) keeps its lease and proxy key.
    let active = match app.proxy.for_session(&s.id) {
        Some(active) => active,
        None => {
            let lease: Lease = serde_json::from_value(call(app, "lease", serde_json::to_value(request(app, &s).await?)?).await.context("get the session's lease from Home")?)?;
            app.proxy.register(lease)
        }
    };
    let lease = active.lease.read().unwrap().clone();
    let app2 = app.clone();
    LOOP.call_once(move || spawn_renewals(app2));
    let (ca, bundle) = app.proxy.ca_files(app)?;
    let dir = session_dir(app, &s.id);
    let home = harness_home(app, &s.id);
    let tmp = dir.join("tmp");
    let workspace = request(app, &s).await?.workspace_id.unwrap_or_else(|| "none".into());
    let cache = app.paths.data_dir.join("caches").join(&workspace);
    // An archived session's folder is removed once Home holds its conversation: get it back first.
    if app.paths.role == crate::role::Role::Runner && s.harness_session_id.is_some() && !home.exists() {
        crate::handoff::restore(app, &s).await.context("get the agent's conversation back from Home")?;
    }
    for d in [&home, &tmp, &cache, &home.join(".claude"), &home.join(".codex"), &home.join(".config")] {
        std::fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
    }
    let proxy_url = format!("http://lp:{}@127.0.0.1:{port}", active.key);
    let base = format!("http://127.0.0.1:{port}/lp/{}", active.key);
    let p = |p: &Path| p.to_string_lossy().into_owned();
    let mut env: Vec<(String, String)> = home_env(app, &s.id);
    let cache_s = p(&cache);
    env.extend([
        ("XDG_CACHE_HOME".to_string(), cache_s.clone()),
        ("CARGO_HOME".into(), format!("{cache_s}/cargo")),
        ("TMPDIR".into(), "/tmp".into()),
        ("LAUNCHPAD_SESSION_ID".into(), s.id.clone()),
        ("LAUNCHPAD_ENV".into(), app.paths.app_env.name().into()),
        ("LAUNCHPAD_ROLE".into(), app.paths.role.name().into()),
        ("LAUNCHPAD_PORT".into(), app.paths.port.to_string()),
        ("LAUNCHPAD_TOKEN".into(), crate::session_token::token(&app.paths.token()?, &s.id, s.generation)?),
        ("GIT_SSH_COMMAND".into(), "ssh -o StrictHostKeyChecking=accept-new".into()),
        ("NODE_USE_ENV_PROXY".into(), "1".into()),
        ("NODE_EXTRA_CA_CERTS".into(), p(&ca)),
    ]);
    for k in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        env.push((k.into(), proxy_url.clone()));
    }
    for k in ["NO_PROXY", "no_proxy"] {
        env.push((k.into(), "127.0.0.1,localhost,::1".into()));
    }
    for k in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE", "GIT_SSL_CAINFO"] {
        env.push((k.into(), p(&bundle)));
    }
    if let Some(rustup) = dirs::home_dir().map(|h| h.join(".rustup")).filter(|r| r.is_dir())
        && app.env.get("RUSTUP_HOME").is_none()
    {
        env.push(("RUSTUP_HOME".into(), p(&rustup)));
    }
    // Tools: placeholders, then their installs on PATH.
    for t in &lease.tools {
        for v in &t.env {
            env.push((v.clone(), crate::vault::placeholder(t.credential.as_deref().unwrap_or(&t.id))));
        }
    }
    let mut ro: Vec<PathBuf> = vec![ca.clone(), bundle.clone()];
    let mut path_prefix = vec![];
    for t in lease.tools.iter().filter(|t| t.install.is_some()) {
        match crate::tool_install::ensure(app, t, &cache).await {
            Ok(bin) => {
                // Settings of the tool: its commands run through wrappers that set them (never in `env`).
                let wrapped = dir.join("tools").join(&t.id);
                match t.vars.is_empty() {
                    true => path_prefix.push(p(&bin)),
                    false => match crate::tool_install::wrappers(t, &bin, &wrapped) {
                        Ok(()) => {
                            path_prefix.push(p(&wrapped));
                            ro.push(wrapped);
                        }
                        Err(e) => {
                            app.sessions.sink(app, &s.id).emit(crate::model::EventBody::Notice { text: format!("The tool {} is left out: its settings could not be prepared on this runner: {e:#}", t.name) }).await;
                            continue;
                        }
                    },
                }
                ro.push(bin.parent().map(Path::to_path_buf).unwrap_or(bin));
            }
            Err(e) => {
                app.sessions.sink(app, &s.id).emit(crate::model::EventBody::Notice { text: format!("The tool {} could not be installed on this runner: {e:#}", t.name) }).await;
            }
        }
    }
    let path = app.env.get("PATH").unwrap_or("/usr/local/bin:/usr/bin:/bin").to_string();
    path_prefix.push(path);
    env.push(("PATH".into(), path_prefix.join(":")));
    // The harness login, set up by its adapter with placeholders only.
    if let Some(login) = &lease.login {
        let setup = crate::harness::LoginSetup { home: &home, login, placeholder: crate::vault::placeholder(&login.id), proxy_base: base.clone() };
        env.extend(app.harnesses.get(s.harness).contained_login(&setup)?);
    }
    write_git_config(&home, &lease)?;
    // The knowledge base: a read-only mirror, and the harness's own memory off.
    let harness = app.harnesses.get(s.harness);
    let kb = match request(app, &s).await?.workspace_id {
        Some(ws) => match crate::kb::sync(app, &ws).await {
            Ok(dir) => Some((ws, dir)),
            Err(e) => {
                app.sessions.sink(app, &s.id).emit(crate::model::EventBody::Notice { text: format!("The knowledge base could not be refreshed from Home: {e:#}. The agent reads the last copy on this runner.") }).await;
                crate::kb::mirror_dir(app, &ws).await.ok().filter(|d| d.is_dir()).map(|d| (ws, d))
            }
        },
        None => None,
    };
    let mut instructions = match &kb {
        Some((ws, dir)) => {
            env.push(("LAUNCHPAD_KB".into(), p(dir)));
            ro.push(dir.clone());
            crate::kb::instructions(app, ws, dir).await
        }
        None => "Your harness's own memory is off in Launchpad, and this session has no workspace, so no knowledge base: do not write memory files.\n".into(),
    };
    // The Launchpad CLI tool: what the session may do with `launchpad`, at its access level.
    if let Some(access) = lease.launchpad {
        instructions.push('\n');
        instructions.push_str(&crate::launchpad_cli::cli_reference(&app.paths.cli_command(), kb.as_ref().map(|(ws, _)| ws.as_str()), access));
    }
    env.extend(harness.contained_memory(&home, &instructions)?);
    // MCP servers of Home: a local address on the runner proxy for each one granted.
    let servers: Vec<(String, String)> = lease.mcp.iter().map(|m| (crate::mcp::harness_name(&m.id), format!("{base}/mcp/{}", m.id))).collect();
    harness.contained_mcp(&home, &servers)?;
    let vm = outside_jail(app);
    // Rust builds: the sessions of a workspace share compiled dependencies through sccache, when it is installed.
    // Each session runs its own sccache server on a socket in its private /tmp: a server started in another jail
    // cannot see this session's folders.
    if !vm && app.env.which("sccache").is_some() {
        env.extend([
            ("RUSTC_WRAPPER".to_string(), "sccache".to_string()),
            ("SCCACHE_DIR".into(), format!("{cache_s}/sccache")),
            ("SCCACHE_SERVER_UDS".into(), "/tmp/sccache.sock".into()),
        ]);
    }
    // Profile values: `${NAME}` references resolve to placeholders of granted tools, never machine values.
    let refs = crate::vault::reference_values(&lease);
    let profile_env = crate::profiles::contained_env(app, s.harness, &s.launch_spec, s.model.as_deref(), &refs)?;
    let mut all = profile_env;
    // Launchpad's variables win over a profile's.
    all.retain(|(k, _)| !env.iter().any(|(e, _)| e == k));
    all.extend(env);
    // Toolchains of the machine, read-only; the session's folders, read-write.
    if let Some(h) = dirs::home_dir() {
        ro.extend(crate::jail::HOME_TOOLCHAINS.iter().map(|d| h.join(d)).filter(|d| d.exists()));
    }
    // Folders on PATH hold programs, never credentials.
    for entry in app.env.get("PATH").unwrap_or_default().split(':') {
        let d = PathBuf::from(entry);
        if d.is_absolute() && d.is_dir() && !d.starts_with(&app.paths.data_dir) && !d.starts_with("/usr") && !ro.contains(&d) {
            ro.push(d);
        }
    }
    for program in crate::harness::registrations().iter().map(|r| app.harnesses.get(r.descriptor.id).sign_in_program()) {
        if let Some(real) = app.env.which(program).and_then(|p| std::fs::canonicalize(p).ok())
            && let Some(parent) = real.parent()
        {
            // A release folder (`…/<version>/bin/<program>`) keeps resources next to `bin`.
            let release = if parent.ends_with("bin") { parent.parent().unwrap_or(parent) } else { parent };
            ro.push(release.to_path_buf());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        ro.push(exe);
    }
    let mut rw: Vec<PathBuf> = vec![start.cwd.clone(), home.clone(), cache.clone(), app.paths.data_dir.join("worktrees"), app.paths.data_dir.join("repos")];
    rw.extend(start.add_dirs.iter().map(PathBuf::from));
    start.env = all.clone();
    // Every agent runs under the network capture; a machine where it cannot run runs no agents.
    crate::netcap::check().await?;
    let capture = crate::netcap::Capture::new(app, &active.key)?;
    // On a Local VM or Burst the machine contains the agents: no jail inside (Docker available on a VM), the
    // runner CA in the machine's own store.
    if vm {
        if !crate::netcap::system_ca(&ca) {
            app.sessions.sink(app, &s.id).emit(crate::model::EventBody::Notice { text: "The runner CA is not in this machine's system certificate store, so tools that ignore SSL_CERT_FILE get a TLS error on hosts where Launchpad adds a token. On a Local VM, run packaging/local-vm.sh again.".into() }).await;
        }
    } else {
        crate::jail::check().await?;
    }
    let jail = crate::jail::Jail { rw, ro, tmp, env: all, bwrap: !vm, capture: Some(capture), ca_bundle: Some(bundle) };
    start.jail = Some(Arc::new(jail));
    Ok(start)
}

/// The session's Git config: Home's identity, and placeholders for hosts where the proxy adds a Basic token.
fn write_git_config(home: &Path, lease: &Lease) -> Result<()> {
    let mut text = String::new();
    if lease.git_name.is_some() || lease.git_email.is_some() {
        text.push_str("[user]\n");
        if let Some(n) = &lease.git_name {
            text.push_str(&format!("\tname = {}\n", n.replace(['\n', '"', '\\'], "")));
        }
        if let Some(e) = &lease.git_email {
            text.push_str(&format!("\temail = {}\n", e.replace(['\n', '"', '\\'], "")));
        }
    }
    for c in &lease.credentials {
        for h in &c.hosts {
            if let crate::vault::Auth::Basic { user } = &h.auth
                && !h.host.starts_with("*.")
            {
                let ph = crate::vault::placeholder(&c.id);
                text.push_str(&format!("[credential \"https://{}\"]\n\thelper = \"!f() {{ echo username={user}; echo password={ph}; }}; f\"\n", h.host));
            }
        }
    }
    std::fs::write(home.join(".gitconfig"), text)?;
    Ok(())
}

/// Stop or Archive: the proxy forgets the session's tokens and Home closes the lease.
pub async fn release(app: &App, session_id: &str) {
    let Some(active) = app.proxy.drop_session(session_id) else { return };
    let lease_id = active.lease.read().unwrap().id.clone();
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = call(&app, "release", json!({"lease_id": lease_id})).await {
            tracing::info!("release lease {lease_id}: {e:#}");
        }
    });
}

/// Home revoked a lease: its tokens stop working at once.
pub async fn revoked(app: &App, lease_id: &str) {
    let session = app.proxy.all().into_iter().find(|a| a.lease.read().unwrap().id == lease_id).map(|a| a.session_id.clone());
    app.proxy.drop_lease(lease_id);
    if let Some(id) = session {
        app.sessions.sink(app, &id).emit(crate::model::EventBody::Notice { text: "Credentials revoked on Home: requests of this session no longer receive tokens. Stop and resume the session to get a new lease.".into() }).await;
    }
}

/// Renews every lease every ten minutes; a lease that Home refuses, or that could not be renewed for an hour,
/// is dropped.
fn spawn_renewals(app: App) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            for active in app.proxy.all() {
                let age = now_ms() - active.renewed_at.load(std::sync::atomic::Ordering::Relaxed);
                if age < crate::vault::RENEW_EVERY_MS {
                    continue;
                }
                let Some(s) = app.sessions.get(&active.session_id) else {
                    app.proxy.drop_session(&active.session_id);
                    continue;
                };
                let lease_id = active.lease.read().unwrap().id.clone();
                let result = match request(&app, &s).await {
                    Ok(req) => call(&app, "renew", json!({"lease_id": lease_id, "request": req})).await,
                    Err(e) => Err(e),
                };
                match result.and_then(|v| Ok(serde_json::from_value::<Lease>(v)?)) {
                    Ok(lease) => {
                        // The adapter refreshes its files (Codex keeps `last_refresh` recent so it never
                        // refreshes with the placeholder).
                        if let Some(login) = &lease.login {
                            let port = app.proxy.ensure(&app).await.unwrap_or_default();
                            let home = harness_home(&app, &s.id);
                            let setup = crate::harness::LoginSetup { home: &home, login, placeholder: crate::vault::placeholder(&login.id), proxy_base: format!("http://127.0.0.1:{port}/lp/{}", active.key) };
                            let _ = app.harnesses.get(s.harness).contained_login(&setup);
                        }
                        *active.lease.write().unwrap() = lease;
                        active.renewed_at.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(e) => {
                        let text = format!("{e:#}");
                        let refused = ["revoked", "another runner", "moved", "unknown lease"].iter().any(|w| text.contains(w));
                        if refused || age > crate::vault::LEASE_TTL_MS {
                            tracing::info!("lease of {}: dropped ({text})", active.session_id);
                            app.proxy.drop_session(&active.session_id);
                        }
                    }
                }
            }
        }
    });
}

/// Records a request that received a token: in the access log on Home (through the outbox on a runner), and
/// as a notice in the session for actions such as opening a pull request.
/// Agents here run without a jail: a Local VM (the VM is the boundary) or a Burst job (the job is).
pub fn outside_jail(app: &App) -> bool {
    crate::home_link::burst().is_some() || app.config.get().runner.boundary == crate::config::Boundary::Vm
}

pub fn record_access(app: &App, access: Access) {
    let app = app.clone();
    tokio::spawn(async move {
        let notable = access.action.as_ref().filter(|a| !a.starts_with("GraphQL query")).cloned();
        let id = access.session_id.clone();
        if let Err(e) = store_access(&app, access).await {
            tracing::warn!("access log: {e:#}");
        }
        if !crate::replica::enabled() {
            app.bus.publish("vault", "access", serde_json::json!({"session_id": id}));
        }
        if let Some(action) = notable
            && app.sessions.get(&id).is_some()
        {
            // On Home (relayed leases), the notice goes to the session's runner, which owns its history.
            crate::sessions::notice_anywhere(&app, &id, &format!("With your token: {action}")).await;
        }
    });
}

async fn store_access(app: &App, access: Access) -> Result<()> {
    app.state_db
        .call(move |c| {
            if crate::replica::enabled() {
                crate::replica::queue(c, &crate::replica::Item::Access { access })
            } else {
                insert_access(c, &access)
            }
        })
        .await
}

pub(crate) fn insert_access(c: &rusqlite::Connection, a: &Access) -> Result<()> {
    c.execute(
        "INSERT INTO access_log (session_id, at, json) VALUES (?1, ?2, ?3)",
        rusqlite::params![a.session_id, a.at, serde_json::to_string(a)?],
    )?;
    Ok(())
}

/// The access log of a session, newest first (Home).
pub async fn access_log(app: &App, session_id: &str) -> Result<Vec<Access>> {
    let id = session_id.to_string();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT json FROM access_log WHERE session_id = ?1 ORDER BY at DESC LIMIT 500")?;
            let rows = st.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await
}

/// Access entries older than 30 days are deleted.
pub async fn prune(app: &App) -> Result<()> {
    let before = now_ms() - 30 * 24 * 3600 * 1000;
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM access_log WHERE at < ?1", [before])?)).await?;
    Ok(())
}

/// Native harness state of a contained session (traces): its own home.
pub fn trace_overrides(app: &App, session: &Session) -> Vec<(String, String)> {
    if contained(app) && session.runner_id.is_none() {
        home_env(app, &session.id)
    } else {
        vec![]
    }
}

/// The environment and jail of an auxiliary process of a session (title requests), sharing its lease.
pub async fn auxiliary(app: &App, session: &Session, cwd: &Path) -> Result<Option<(Vec<(String, String)>, Option<Arc<crate::jail::Jail>>)>> {
    if !contained(app) {
        return Ok(None);
    }
    let spec = StartSpec {
        browser: false,
        browser_tools: false,
        permission_mode: session.launch_spec.permission_mode,
        session_id: session.id.clone(),
        harness_session_id: None,
        resume: false,
        cwd: cwd.to_path_buf(),
        title: session.title.clone(),
        model: session.model.clone(),
        effort: session.effort.clone(),
        add_dirs: vec![],
        env: vec![],
        jail: None,
    };
    let spec = apply(app, spec).await?;
    Ok(Some((spec.env, spec.jail)))
}
