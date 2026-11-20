//! Settings → Slack (the owner's Slack profiles, `launchpad_core::slack`) and `launchpad slack` (an agent's call,
//! `POST /api/slack/run`): Home checks that the calling session holds the profile, adds the attribution line when the
//! profile says so, and runs its own slackline with the profile's token. Runners forward every `/api/slack` call to
//! Home; they need neither slackline nor a Slack token.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_core::slack::{self, AppLink, Output, Profile, Save, TokenKind};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/slack/profiles", get(list))
        .route("/api/slack/profiles/{handle}", put(save).delete(delete))
        .route("/api/slack/profiles/{handle}/test", post(test))
        .route("/api/slack/app-link", get(app_link))
        .route("/api/slack/run", post(run).layer(DefaultBodyLimit::max(crate::agent_auth::SLACK_RUN_MAX)))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// An owner route: agents use `launchpad slack` (`/api/slack/run`) only.
fn owner(agent: &Option<axum::Extension<launchpad_http::AgentSession>>) -> Result<(), ApiError> {
    match agent {
        Some(_) => Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Slack profiles are the owner's (Settings → Slack): agents use `launchpad slack --profile <handle>`")),
        None => Ok(()),
    }
}

async fn list(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>) -> ApiResult<Vec<Profile>> {
    owner(&agent)?;
    Ok(Json(slack::list(&s.app).await.map_err(bad)?))
}
async fn save(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Path(handle): Path<String>, Json(b): Json<Save>) -> ApiResult<Profile> {
    owner(&agent)?;
    Ok(Json(slack::save(&s.app, &handle, b).await.map_err(bad)?))
}
async fn delete(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Path(handle): Path<String>) -> ApiResult<Value> {
    owner(&agent)?;
    slack::delete(&s.app, &handle).await.map_err(bad)?;
    Ok(Json(json!({})))
}
async fn test(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Path(handle): Path<String>) -> ApiResult<slack::Test> {
    owner(&agent)?;
    Ok(Json(slack::test(&s.app, &handle).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct LinkQuery {
    #[serde(default)]
    name: String,
    kind: TokenKind,
    #[serde(default)]
    write: bool,
}
async fn app_link(Query(q): Query<LinkQuery>) -> ApiResult<AppLink> {
    Ok(Json(slack::app_link(&q.name, q.kind, q.write)))
}

#[derive(Deserialize)]
struct Upload {
    /// The file's name (its folders are left out).
    name: String,
    /// Base64.
    data: String,
}

#[derive(Deserialize)]
struct Run {
    #[allow(dead_code)]
    session_id: String,
    #[serde(default)]
    profile: Option<String>,
    args: Vec<String>,
    /// The file of `files upload <path>`, read by the CLI in the agent's folder.
    #[serde(default)]
    file: Option<Upload>,
}

fn refused(why: impl Into<String>) -> ApiError {
    ApiError::new(axum::http::StatusCode::FORBIDDEN, why.into())
}

/// Pure: the profiles a session holds, in words, for an error.
fn held(profiles: &[Profile]) -> String {
    if profiles.is_empty() {
        return "this session holds no Slack profile (the owner gives them: Settings → Slack, a launch's Tools, a bot's settings)".into();
    }
    let list: Vec<String> = profiles.iter().map(|p| format!("{} ({})", p.handle, p.name)).collect();
    format!("this session holds: {}", list.join(", "))
}

/// Pure: a file name without folders, safe as one path component.
fn file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.is_empty() || clean == "." || clean == ".." { "file".into() } else { clean }
}

/// `launchpad slack --profile <handle> <slackline arguments>`: the agent's own call (Home takes the session from
/// the CLI token, never from the body).
async fn run(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(r): Json<Run>) -> ApiResult<Output> {
    use base64::Engine;
    let app = &s.app;
    let Some(axum::Extension(launchpad_http::AgentSession(session_id))) = agent else {
        return Err(ApiError::bad_request("launchpad slack runs in an agent's session"));
    };
    if !launchpad_core::vault::here(app) {
        return Err(ApiError::bad_request("Slack calls run on Home"));
    }
    let plan = slack::plan(&r.args).map_err(ApiError::bad_request)?;
    if plan.output.is_some() {
        return Err(ApiError::bad_request("-o is written by launchpad slack in your folder: Home never writes files"));
    }
    let profiles = slack::of_session(app, &session_id).await.map_err(bad)?;
    let mut args = plan.args.clone();
    let profile = match r.profile.as_deref() {
        Some(h) => Some(profiles.iter().find(|p| p.handle == h).ok_or_else(|| refused(format!("no Slack profile {h:?} for this session: {}", held(&profiles))))?),
        None if slack::help_only(&args) => None,
        None => return Err(ApiError::bad_request(format!("--profile <handle> is required: {}", held(&profiles)))),
    };
    let token = match profile {
        Some(p) => Some(slack::token(app, &p.handle).await.map_err(bad)?),
        None => None,
    };
    let dir = std::env::temp_dir().join(format!("launchpad-slack-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| bad(e.into()))?;
    let result = async {
        if let Some(i) = plan.upload {
            let f = r.file.as_ref().ok_or_else(|| ApiError::bad_request("files upload sends the file itself: use launchpad slack, which reads it in your folder"))?;
            let data = base64::engine::general_purpose::STANDARD.decode(f.data.as_bytes()).map_err(|_| ApiError::bad_request("the file is not base64"))?;
            if data.len() > slack::MAX_UPLOAD {
                return Err(ApiError::bad_request(format!("the file is larger than {} MB", slack::MAX_UPLOAD >> 20)));
            }
            let path = dir.join(file_name(&f.name));
            std::fs::write(&path, data).map_err(|e| bad(e.into()))?;
            args[i] = path.display().to_string();
        }
        // After the upload path is in place (the comment goes before `--`). The attribution line: the message text, or a comment of a file shared in a channel.
        if profile.is_some_and(|p| p.footer) {
            let s = app.sessions.get(&session_id).ok_or_else(|| ApiError::bad_request("Home has no record of this session"))?;
            let line = slack::footer_line(&s);
            match plan.text {
                Some(i) => args[i] = launchpad_core::attribution::sign(&args[i], &line, launchpad_core::attribution::Separator::Blank),
                None if plan.upload.is_some() && plan.shared => {
                    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
                    args.splice(at..at, ["--comment".to_string(), line]);
                }
                None => {}
            }
        }
        let bin = slack::binary(app, "LAUNCHPAD_SLACKLINE", "slackline").ok_or_else(|| ApiError::bad_request("slackline is not installed on Home (Home's image has it; on a dev Home put it on PATH or set LAUNCHPAD_SLACKLINE)"))?;
        Ok(slack::run(&bin, &args, token.as_deref(), &dir).await)
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(Json(result?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_stay_one_component() {
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name("dir\\report.pdf"), "report.pdf");
        assert_eq!(file_name(".."), "file");
        assert_eq!(file_name(""), "file");
        assert_eq!(file_name("a\nb.txt"), "ab.txt");
    }
}
