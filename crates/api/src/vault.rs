//! Settings → Credentials and tools: the vault and the tool registry on Home. Secrets are write-only: no route
//! returns them. Runners forward these requests to Home.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_core::tool_defaults;
use launchpad_core::vault::{self, HomeToken, VaultEntry, Kind, LeaseInfo, LoginKind, Tool, VaultOverview};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/vault", get(overview))
        .route("/api/vault/entries/{id}", put(put_entry).delete(delete_entry))
        .route("/api/vault/entries/{id}/from-home", post(entry_from_home))
        .route("/api/vault/entries/{id}/refresh", post(refresh_entry))
        .route("/api/vault/tools/{id}", put(put_tool).delete(delete_tool))
        .route("/api/vault/tools/{id}/test", post(test_tool))
        .route("/api/vault/tools.toml", get(export))
        .route("/api/vault/identity", put(identity))
        .route("/api/vault/leases", get(leases))
        .route("/api/sessions/{id}/access", get(access))
        .route("/api/tool-defaults", get(tool_defaults))
        .route("/api/tool-defaults/{scope}/{id}", get(tool_choice).put(set_tool_choice))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn overview(State(s): State<Ctx>) -> ApiResult<VaultOverview> {
    let home = s.app.config.get().home.clone();
    let logins = launchpad_core::harness::registrations()
        .iter()
        .filter_map(|r| r.descriptor.vault_login.map(|l| LoginKind { harness: r.descriptor.id.as_str().into(), name: r.descriptor.name.into(), kind: l.kind, how: l.how.into() }))
        .collect();
    Ok(Json(VaultOverview { entries: vault::entries(&s.app).await?, tools: vault::tools(&s.app).await?, templates: vault::templates(), template_help: vault::template_help(), home_tokens: vault::home_tokens(&s.app).await, logins, git_name: home.git_name, git_email: home.git_email }))
}

#[derive(Deserialize)]
struct EntryBody {
    kind: Kind,
    #[serde(default)]
    name: String,
    value: String,
}

async fn put_entry(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<EntryBody>) -> ApiResult<VaultEntry> {
    Ok(Json(vault::put(&s.app, &id, b.kind, &b.name, &b.value).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct FromHomeBody {
    source: HomeToken,
    #[serde(default)]
    name: String,
}

/// Copies Home's own GitHub or Hugging Face token into an entry. Only on Home; the token is never returned.
async fn entry_from_home(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<FromHomeBody>) -> ApiResult<VaultEntry> {
    Ok(Json(vault::put_from_home(&s.app, &id, &b.name, b.source).await.map_err(bad)?))
}

/// Refresh of a harness login (the owner's click): what it found, also when the provider refused the login.
async fn refresh_entry(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<vault::LoginCheck> {
    Ok(Json(vault::refresh_login(&s.app, &id).await.map_err(bad)?))
}

async fn delete_entry(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    vault::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({})))
}

async fn put_tool(State(s): State<Ctx>, Path(id): Path<String>, Json(t): Json<Tool>) -> ApiResult<Tool> {
    if t.id != id {
        return Err(ApiError::bad_request("the tool ID does not match the address"));
    }
    Ok(Json(vault::put_tool(&s.app, t).await.map_err(bad)?))
}

async fn delete_tool(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    vault::delete_tool(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({})))
}

async fn export(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!({"toml": vault::export_tools(&s.app).await?})))
}

#[derive(Deserialize)]
struct Identity {
    name: String,
    email: String,
}

async fn identity(State(s): State<Ctx>, Json(b): Json<Identity>) -> ApiResult<Value> {
    if !vault::here(&s.app) {
        return Err(ApiError::bad_request("the Git identity is kept on Home"));
    }
    s.app.config.update(|config| {
        config.home.git_name = b.name.trim().replace('\n', "");
        config.home.git_email = b.email.trim().replace('\n', "");
        Ok(())
    })?;
    s.app.bus.publish("vault", "changed", Value::Null);
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct LeaseQuery {
    session_id: Option<String>,
}

async fn leases(State(s): State<Ctx>, Query(q): Query<LeaseQuery>) -> ApiResult<Vec<LeaseInfo>> {
    Ok(Json(vault::leases(&s.app, q.session_id.as_deref()).await?))
}

async fn access(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<launchpad_core::proxy::Access>> {
    Ok(Json(launchpad_core::contain::access_log(&s.app, &id).await?))
}

async fn test_tool(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<vault::ToolTest> {
    Ok(Json(vault::test_tool(&s.app, &id).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

async fn tool_defaults(State(s): State<Ctx>) -> ApiResult<tool_defaults::ToolDefaults> {
    Ok(Json(tool_defaults::all(&s.app).await?))
}

#[derive(serde::Serialize, Deserialize)]
struct ChoiceBody {
    #[serde(default)]
    tools: tool_defaults::ToolChoice,
}

async fn tool_choice(State(s): State<Ctx>, Path((scope, id)): Path<(String, String)>) -> ApiResult<ChoiceBody> {
    let scope = tool_defaults::Scope::parse(&scope).map_err(bad)?;
    Ok(Json(ChoiceBody { tools: tool_defaults::get(&s.app, scope, &id).await? }))
}

/// Default tools of a workspace or a folder (`tools.defaults`).
async fn set_tool_choice(State(s): State<Ctx>, Path((scope, id)): Path<(String, String)>, Json(b): Json<ChoiceBody>) -> ApiResult<ChoiceBody> {
    let scope = tool_defaults::Scope::parse(&scope).map_err(bad)?;
    Ok(Json(ChoiceBody { tools: tool_defaults::set(&s.app, scope, &id, b.tools).await.map_err(bad)? }))
}
