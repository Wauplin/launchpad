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
    /// Commits absent from locally cached remote refs; no fetch is performed.
    pub unpushed_commits: Option<u32>,
    /// Working tree compared with main, including staged and unstaged tracked changes.
    pub main_changes: Option<GitChanges>,
    pub main_files: Vec<GitFile>,
}
#[derive(Serialize, TS)]
#[ts(rename = "SessionGitChanges")]
pub struct GitChanges {
    pub files: u32,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
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
    let base = main_branch(app, cwd).await;
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
    let unpushed_commits = if procs::output(&app.env, "git", &["remote"], Some(cwd))
        .await
        .is_ok_and(|s| !s.is_empty())
    {
        procs::output(
            &app.env,
            "git",
            &["rev-list", "--count", "HEAD", "--not", "--remotes"],
            Some(cwd),
        )
        .await
        .ok()
        .and_then(|s| s.parse().ok())
    } else {
        None
    };
    let main_stat = procs::output(
        &app.env,
        "git",
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            base.as_str(),
            "--",
        ],
        Some(cwd),
    )
    .await
    .ok();
    let main_changes = main_stat.map(|stat| {
        let mut changes = GitChanges {
            files: 0,
            additions: Some(0),
            deletions: Some(0),
        };
        for line in stat.lines() {
            let mut parts = line.split('\t');
            changes.files += 1;
            changes.additions = changes
                .additions
                .zip(parts.next().and_then(|s| s.parse::<u32>().ok()))
                .map(|(a, b)| a + b);
            changes.deletions = changes
                .deletions
                .zip(parts.next().and_then(|s| s.parse::<u32>().ok()))
                .map(|(a, b)| a + b);
        }
        changes
    });
    let main_names = procs::command(&app.env, "git", Some(cwd))
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "-z",
            base.as_str(),
            "--",
        ])
        .output()
        .await
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let main_files = main_names
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|path| GitFile {
            path: path.to_string(),
            status: "Δ".into(),
            previous: None,
        })
        .collect();
    Ok(GitStatus {
        branch,
        files,
        unpushed_commits,
        main_changes,
        main_files,
    })
}

pub async fn diff(app: &App, id: &str, path: &str, main: bool) -> Result<GitDiff> {
    let s = app.sessions.get(id).context("no such session")?;
    let status = status(app, id).await?;
    let files = if main {
        status.main_files
    } else {
        status.files
    };
    let file = files
        .iter()
        .find(|f| f.path == path)
        .context("file is no longer changed; refresh status")?;
    let cwd = std::path::Path::new(&s.cwd);
    let base = main_branch(app, cwd).await;
    let mut paths = vec![path];
    if let Some(previous) = &file.previous {
        paths.push(previous);
    }
    if main {
        let patch = procs::output(
            &app.env,
            "git",
            &[
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                base.as_str(),
                "--",
                path,
            ],
            Some(cwd),
        )
        .await?;
        return Ok(GitDiff {
            staged: String::new(),
            truncated: patch.len() > 200_000,
            unstaged: truncate_output(&patch, 200_000),
        });
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

/// The branch "compared with main" uses: the local default branch (`main`, `master` or origin's HEAD),
/// where task worktrees merge.
async fn main_branch(app: &App, cwd: &std::path::Path) -> String {
    let default = crate::code::default_branch(&app.env, cwd).await.unwrap_or_else(|| "main".into());
    let local = default.strip_prefix("origin/").unwrap_or(&default).to_string();
    if procs::output(&app.env, "git", &["show-ref", "--verify", "--quiet", &format!("refs/heads/{local}")], Some(cwd)).await.is_ok() { local } else { default }
}
