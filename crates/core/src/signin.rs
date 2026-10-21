//! Read-only sign-in checks for Settings → Profiles, so a server shows which logins still need an SSH session.
//!
//! Each adapter runs its own status command (never a login or a token refresh). Results keep only a short,
//! non-secret description; command output that could contain a credential is not returned.
use std::time::Duration;

use serde::Serialize;
use ts_rs::TS;

use crate::{App, harness::REGISTRATIONS, shellenv::UserEnv};

/// What an adapter reports about its native sign-in.
pub struct SignIn {
    /// `None` when the CLI cannot tell (old version, unexpected output).
    pub signed_in: Option<bool>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct SignInCheck {
    /// Harness ID, or `github`.
    pub id: String,
    /// Profile ID when the check used a profile environment.
    pub profile: Option<String>,
    pub name: String,
    pub cli_found: bool,
    pub signed_in: Option<bool>,
    pub detail: Option<String>,
    /// Command to run once on the daemon host (over SSH on a server).
    pub command: String,
}

/// Runs a status command and returns (success, stdout, stderr). A missing program is an error.
pub async fn status_output(env: &UserEnv, program: &str, args: &[&str]) -> anyhow::Result<(bool, String, String)> {
    let mut cmd = crate::procs::command(env, program, None);
    cmd.args(args).stdin(crate::exec::Io::Null);
    let child = cmd.output();
    let out = tokio::time::timeout(Duration::from_secs(20), child).await.map_err(|_| anyhow::anyhow!("{program} did not answer within 20 s"))??;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// Removes ANSI escape sequences from CLI output.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() { break; }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

async fn check(app: &App, env: UserEnv, id: crate::model::HarnessKind, profile: Option<(String, String)>) -> SignInCheck {
    let registration = crate::harness::registration(id);
    let adapter = app.harnesses.get(id);
    let program = adapter.sign_in_program();
    let cli_found = env.which(program).is_some_and(|p| p.is_file());
    let result = if cli_found { adapter.sign_in(&env).await } else { SignIn { signed_in: None, detail: None } };
    let (profile, name) = match profile {
        Some((vid, vname)) => (Some(vid), vname),
        None => (None, registration.descriptor.name.to_string()),
    };
    SignInCheck { id: id.as_str().into(), profile, name, cli_found, signed_in: result.signed_in, detail: result.detail, command: adapter.sign_in_command().into() }
}

async fn github(env: &UserEnv) -> SignInCheck {
    let cli_found = env.which("gh").is_some_and(|p| p.is_file());
    let mut c = SignInCheck { id: "github".into(), profile: None, name: "GitHub CLI".into(), cli_found, signed_in: None, detail: None, command: "gh auth login".into() };
    if !cli_found { return c; }
    // `gh auth status` masks the token; only the account line is kept.
    match status_output(env, "gh", &["auth", "status", "--hostname", "github.com"]).await {
        Ok((ok, stdout, stderr)) => {
            c.signed_in = Some(ok);
            let text = format!("{stdout}\n{stderr}");
            c.detail = text.lines().find_map(|l| l.split_once("Logged in to github.com account ").map(|(_, rest)| format!("Account {}", rest.trim())));
        }
        Err(e) => c.detail = Some(format!("{e:#}")),
    }
    c
}

/// Every registered harness (its default login), each profile with its own environment, and the GitHub CLI.
pub async fn check_all(app: &App) -> Vec<SignInCheck> {
    let defaults = futures_util::future::join_all(REGISTRATIONS.iter().map(|r| check(app, app.env.clone(), r.descriptor.id, None)));
    let profiles = futures_util::future::join_all(app.config.get().profiles.into_iter().filter(|v| !v.env.is_empty()).map(|v| async move {
        let command = crate::profiles::sign_in_command(app, &v);
        // The model only matters for model references; any value checks the sign-in.
        match crate::profiles::profile_env(app, &v, Some(v.model.as_deref().unwrap_or("default")), None) {
            Ok(env) => SignInCheck { command, ..check(app, app.env.with(&env), v.harness, Some((v.id.clone(), v.name.clone()))).await },
            Err(e) => SignInCheck {
                id: v.harness.as_str().into(), profile: Some(v.id.clone()), name: v.name.clone(), cli_found: true, signed_in: Some(false),
                detail: Some(format!("{e:#}")), command,
            },
        }
    }));
    let (mut out, profiles, github) = futures_util::join!(defaults, profiles, github(&app.env));
    out.extend(profiles);
    out.push(github);
    out
}
