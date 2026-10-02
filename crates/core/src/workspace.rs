//! Read-only git status and selected-file diffs for a session's own working directory.
use crate::{App, procs, sessions::truncate_output};
use anyhow::{Context, Result};
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, TS)]
#[ts(rename = "SessionGitFile")]
pub struct GitFile {
    pub path: String,
    pub status: String,
    pub previous: Option<String>,
}
#[derive(Serialize, TS)]
#[ts(rename = "SessionGitStatus")]
pub struct GitStatus {
    pub branch: String,
    pub files: Vec<GitFile>,
}
#[derive(Serialize, TS)]
#[ts(rename = "SessionGitDiff")]
pub struct GitDiff {
    pub staged: String,
    pub unstaged: String,
    pub truncated: bool,
}

pub async fn status(app: &App, id: &str) -> Result<GitStatus> {
    let s = app.sessions.get(id).context("no such session")?;
    let cwd = std::path::Path::new(&s.cwd);
    let out = procs::command(&app.env, "git", Some(cwd))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .await?;
    anyhow::ensure!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let branch = procs::output(
        &app.env,
        "git",
        &["rev-parse", "--abbrev-ref", "HEAD"],
        Some(cwd),
    )
    .await
    .unwrap_or_else(|_| "No commits".into());
    let mut parts = text.split('\0').filter(|s| !s.is_empty());
    let mut files = vec![];
    while let Some(part) = parts.next() {
        if part.len() < 4 {
            continue;
        }
        let status = &part[..2];
        let previous = if status.contains(['R', 'C']) {
            parts.next().map(str::to_string)
        } else {
            None
        };
        files.push(GitFile {
            path: part[3..].to_string(),
            status: status.to_string(),
            previous,
        });
    }
    Ok(GitStatus { branch, files })
}

pub async fn diff(app: &App, id: &str, path: &str) -> Result<GitDiff> {
    let s = app.sessions.get(id).context("no such session")?;
    let files = status(app, id).await?.files;
    let file = files
        .iter()
        .find(|f| f.path == path)
        .context("file is no longer changed; refresh status")?;
    let cwd = std::path::Path::new(&s.cwd);
    let mut paths = vec![path];
    if let Some(previous) = &file.previous {
        paths.push(previous);
    }
    let read = |staged: bool| {
        let paths = paths.clone();
        async move {
            let mut args = vec![
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
            ];
            if staged {
                args.push("--cached");
            }
            args.push("--");
            args.extend(paths);
            procs::output(&app.env, "git", &args, Some(cwd)).await
        }
    };
    let staged = read(true).await?;
    let mut unstaged = read(false).await?;
    if file.status == "??" {
        // The code viewer performs all root, path and symlink checks before reading untracked files.
        match crate::code::open(app, &s.cwd, None).await {
            Ok(src) => {
                let f = crate::code::file(app, &src, path).await?;
                unstaged = f
                    .text
                    .as_deref()
                    .map(|t| t.lines().map(|l| format!("+{l}\n")).collect())
                    .unwrap_or_else(|| "Untracked binary file. Open Files to view it.".into());
            }
            Err(_) => {
                unstaged =
                    "Untracked file. Register this folder as a project to read it in Files.".into()
            }
        }
    }
    let truncated = staged.len() > 200_000 || unstaged.len() > 200_000;
    Ok(GitDiff {
        staged: truncate_output(&staged, 200_000),
        unstaged: truncate_output(&unstaged, 200_000),
        truncated,
    })
}
