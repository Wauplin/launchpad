//! A session owner in its own systemd user service. The daemon can disappear without stopping it.
//! Only this process writes its session row and normalizes harness output. Reconnection replays the
//! durable event log, then follows the bus (including transient output).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::{App, events::Event, model::*, paths::Paths, sessions::LiveSession};

pub fn socket(paths: &Paths, id: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(id).context("invalid session id")?;
    // /run/user is private and avoids Unix socket's 108-byte path limit.
    let dir = PathBuf::from(format!(
        "/run/user/{}/launchpad-{}",
        unsafe { libc::getuid() },
        paths.profile.name()
    ));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir.join(format!("{id}.sock")))
}

pub async fn available(paths: &Paths, id: &str) -> bool {
    let Ok(path) = socket(paths, id) else {
        return false;
    };
    UnixStream::connect(path).await.is_ok()
}

pub async fn call(paths: &Paths, id: &str, action: &str, body: Value) -> Result<Value> {
    let stream = UnixStream::connect(socket(paths, id)?)
        .await
        .context("connect to session worker")?;
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(format!("{}\n", json!({"action":action,"body":body})).as_bytes())
        .await?;
    let mut lines = BufReader::new(reader).lines();
    let line = tokio::time::timeout(Duration::from_secs(90), lines.next_line())
        .await??
        .context("session worker disconnected")?;
    let response: Value = serde_json::from_str(&line)?;
    if let Some(error) = response["error"].as_str() {
        bail!("{error}");
    }
    Ok(response)
}

/// Wait for explicit archive to release its unit before allowing the same id to resume.
pub async fn wait_stopped(app: &App, id: &str) -> Result<()> {
    let unit = format!("launchpad-{}-session-{id}", app.paths.profile.name());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let out = crate::procs::command(&app.env, "systemctl", None)
            .args(["--user", "show", "--property=ActiveState", "--value", &unit])
            .output()
            .await?;
        let state = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || matches!(state.trim(), "inactive" | "failed" | "") {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "session worker is still stopping"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn attach(app: &App, id: &str, fresh: bool) -> Result<Arc<dyn LiveSession>> {
    if !available(&app.paths, id).await {
        let exe = std::env::current_exe()?;
        let unit = format!("launchpad-{}-session-{id}", app.paths.profile.name());
        let mut cmd = crate::procs::command(&app.env, "systemd-run", None);
        cmd.args([
            "--user",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=10",
        ])
        .arg(format!("--unit={unit}"))
        .arg(format!(
            "--setenv=LAUNCHPAD_PROFILE={}",
            app.paths.profile.name()
        ))
        .arg(exe)
        .args(["session-worker", id]);
        if fresh {
            cmd.arg("--fresh");
        }
        let out = cmd.output().await.context("start session worker service")?;
        anyhow::ensure!(
            out.status.success(),
            "start session worker: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !available(&app.paths, id).await {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "session worker did not start; inspect journalctl --user -u {unit}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    // The listener is bound only after the adapter has attached successfully.
    let value = call(&app.paths, id, "snapshot", json!({})).await?;
    let s: Session = serde_json::from_value(value["session"].clone())?;
    app.sessions.accept_snapshot(app, s);
    Ok(Arc::new(Remote {
        paths: (*app.paths).clone(),
        id: id.into(),
    }))
}

struct Remote {
    paths: Paths,
    id: String,
}
#[async_trait]
impl LiveSession for Remote {
    async fn send(&self, text: &str, _client_id: &str, images: &[ImageAttachment]) -> Result<()> {
        call(
            &self.paths,
            &self.id,
            "send",
            json!({"text":text,"images":images}),
        )
        .await?;
        Ok(())
    }
    async fn interrupt(&self) -> Result<()> {
        call(&self.paths, &self.id, "interrupt", json!({})).await?;
        Ok(())
    }
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        call(
            &self.paths,
            &self.id,
            "answer",
            json!({"request_id":request_id,"answers":answers}),
        )
        .await?;
        Ok(())
    }
    async fn stop(&self) -> Result<()> {
        call(&self.paths, &self.id, "close", json!({})).await?;
        Ok(())
    }
}

/// Follow snapshots and the event bus. Durable events are backfilled from the requested sequence.
pub fn watch(app: &App, id: &str, instance: u64, after: i64) {
    let app = app.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        let result = follow(&app, &id, after, instance).await;
        if let Err(e) = result {
            tracing::warn!("session worker {id}: {e:#}");
        }
        if let Err(e) = app.sessions.load_worker_exit(&app, &id, instance).await {
            tracing::warn!("reload stopped worker: {e:#}");
        }
        app.sessions.process_exited(&app, &id, instance, None).await;
    });
}

async fn follow(app: &App, id: &str, mut after: i64, instance: u64) -> Result<()> {
    loop {
        let stream = UnixStream::connect(socket(&app.paths, id)?).await?;
        let (reader, mut writer) = stream.into_split();
        writer
            .write_all(format!("{}\n", json!({"action":"watch","body":{"after":after}})).as_bytes())
            .await?;
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await? {
            if !app.sessions.worker_current(id, instance) {
                return Ok(());
            }
            let v: Value = serde_json::from_str(&line)?;
            if let Some(s) = v.get("session") {
                app.sessions
                    .accept_snapshot(app, serde_json::from_value(s.clone())?);
            } else if let Some(e) = v.get("event") {
                let event = serde_json::from_value::<Event>(e.clone())?;
                if event.topic == format!("session/{id}") && event.seq > 0 {
                    if event.seq as i64 <= after {
                        continue;
                    }
                    after = event.seq as i64;
                }
                app.bus.send(event);
            }
        }
        if !available(&app.paths, id).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn run(paths: Paths, id: &str, fresh: bool) -> Result<()> {
    let path = socket(&paths, id)?;
    // Never unlink a socket belonging to an existing owner.
    anyhow::ensure!(!available(&paths, id).await, "session already has a worker");
    let _ = std::fs::remove_file(&path);
    let app = App::start_worker(paths).await?;
    app.sessions.load_one(&app, id).await?;
    app.sessions.attach_worker(&app, id, fresh).await?;
    if fresh {
        let prompt = app.sessions.get(id).context("no such session")?.prompt;
        app.sessions.send(&app, id, &prompt, vec![]).await?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let (shutdown, mut stop) = tokio::sync::watch::channel(false);
    let commands = Arc::new(tokio::sync::Mutex::new(()));
    loop {
        tokio::select! {
            connection = listener.accept() => {
                let (stream, _) = connection?;
                let (app, id, shutdown, commands) = (app.clone(), id.to_string(), shutdown.clone(), commands.clone());
                tokio::spawn(async move {
                    if let Err(e) = serve(&app, &id, stream, shutdown, commands).await { tracing::debug!("session socket: {e:#}"); }
                });
            }
            _ = stop.changed() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    // Kill the harness on explicit worker shutdown, not on UI daemon shutdown.
    let _ = std::fs::remove_file(path);
    Ok(())
}

async fn serve(
    app: &App,
    id: &str,
    stream: UnixStream,
    shutdown: tokio::sync::watch::Sender<bool>,
    commands: Arc<tokio::sync::Mutex<()>>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let Some(line) = lines.next_line().await? else {
        return Ok(());
    };
    let req: Value = serde_json::from_str(&line)?;
    let action = req["action"].as_str().context("missing action")?;
    let body = &req["body"];
    if action == "watch" {
        let mut rx = app.bus.subscribe();
        let after = body["after"].as_i64().unwrap_or(0);
        writer
            .write_all(format!("{}\n", json!({"session":app.sessions.get(id)})).as_bytes())
            .await?;
        for ev in app.sessions.events(app, id, after).await? {
            let event = Event {
                seq: ev.seq as u64,
                topic: format!("session/{id}"),
                kind: "event".into(),
                payload: serde_json::to_value(ev)?,
            };
            writer
                .write_all(format!("{}\n", json!({"event":event})).as_bytes())
                .await?;
        }
        let resync = Event {
            seq: 0,
            topic: format!("terminal/{id}"),
            kind: "resync".into(),
            payload: json!({}),
        };
        writer
            .write_all(format!("{}\n", json!({"event":resync})).as_bytes())
            .await?;
        let mut stopped = shutdown.subscribe();
        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Ok(event) if event.topic == "sessions" => {
                            writer.write_all(format!("{}\n", json!({"session":app.sessions.get(id)})).as_bytes()).await?;
                        }
                        Ok(event) if event.topic == format!("session/{id}") || event.topic == format!("terminal/{id}") => writer.write_all(format!("{}\n", json!({"event":event})).as_bytes()).await?,
                        Ok(_) => {}
                        Err(_) => break, // reconnect and replay if this watcher falls behind
                    }
                }
                _ = stopped.changed() => break,
                _ = lines.next_line() => break,
            }
        }
        return Ok(());
    }
    let _lock = commands.lock().await;
    let result: Result<Value> = async {
        match action {
            "snapshot" => {}
            "embedded-terminal" => app.sessions.embedded_terminal(app, id).await?,
            "terminal-snapshot" => return Ok(app.pty.snapshot()),
            "terminal-input" => {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(body["data"].as_str().context("terminal data")?)?;
                app.pty.input(&data).await?;
            }
            "terminal-resize" => {
                let rows = u16::try_from(body["rows"].as_u64().context("rows")?)?;
                let cols = u16::try_from(body["cols"].as_u64().context("cols")?)?;
                app.pty.resize(rows, cols)?;
            }
            "terminal-stop" => app.pty.stop().await?,
            "send" => {
                app.sessions
                    .send(
                        app,
                        id,
                        body["text"].as_str().unwrap_or(""),
                        serde_json::from_value(body["images"].clone())?,
                    )
                    .await?
            }
            "interrupt" => app.sessions.interrupt(app, id).await?,
            "answer" => {
                app.sessions
                    .answer(
                        app,
                        id,
                        body["request_id"].as_str().context("request id")?,
                        serde_json::from_value(body["answers"].clone())?,
                    )
                    .await?
            }
            "close" => app.sessions.close(app, id).await?,
            "resume" => app.sessions.resume(app, id).await?,
            "release-terminal" => app.sessions.release_terminal(app, id).await?,
            "open-terminal" => app.sessions.open_terminal(app, id).await?,
            "move-cwd" => {
                app.sessions
                    .move_cwd(
                        app,
                        body["old"].as_str().context("old cwd")?,
                        body["new"].as_str().context("new cwd")?,
                    )
                    .await?
            }
            "presence" => app.sessions.set_watched(
                body["focused"].as_bool().unwrap_or(false),
                serde_json::from_value(body["sessions"].clone())?,
            ),
            "rename" => {
                app.sessions
                    .rename(app, id, body["title"].as_str().context("title")?)
                    .await?;
            }
            _ => bail!("unknown worker action"),
        }
        Ok(json!({"session":app.sessions.get(id)}))
    }
    .await;
    let exit = result.is_ok() && action == "close";
    let response = result.unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
    writer.write_all(format!("{response}\n").as_bytes()).await?;
    if exit {
        // Let the watch connection deliver the final snapshot before leaving.
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.send_replace(true);
    }
    Ok(())
}
