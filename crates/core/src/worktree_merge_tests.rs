//! Real repositories and PTYs, with isolated state and no background monitors.
use crate::worktrees::*;
use crate::{App, paths::{Paths, AppEnv}};
use std::sync::Arc;

async fn fixture(root: &std::path::Path) -> App {
    let paths = Arc::new(Paths { app_env: AppEnv::Dev, config_dir: root.join("config"), data_dir: root.join("data"), port: 0 });
    paths.ensure_dirs().unwrap();
    let bus = crate::events::Bus::new();
    let config = crate::config::ConfigHandle::load(&paths, bus.clone()).unwrap();
    let mut settings = config.get();
    settings.projects_root = root.to_string_lossy().into_owned();
    config.save(settings).unwrap();
    let state_db = crate::db::open_state(&paths.state_db()).unwrap();
    let cache_db = crate::db::open_cache(&paths.cache_db()).unwrap();
    let env = crate::shellenv::UserEnv::resolve().await;
    let status = crate::status::StatusHandle::new(bus.clone());
    let hosts = crate::hosts::Hosts::start(paths.clone(), env.clone(), state_db.clone(), status.clone()).await.unwrap();
    App {
        browser: Default::default(),
        trace_backup: Default::default(),
        paths: paths.clone(), config, state_db, cache_db: cache_db.clone(), bus, status, hosts,
        harnesses: crate::harness::Registry::new(&env),
        github: crate::github::GitHub::new(env.clone(), cache_db), env,
        sessions: crate::sessions::Sessions::new(), inbox: crate::inbox::Inbox::new(),
        models: Default::default(), suggestion_locks: Default::default(),
        predraft_lock: Default::default(), worktree_lock: Default::default(),
        terminals: Default::default(), push: crate::push::Push::new(paths),
    }
}

async fn git(app: &App, dir: &str, args: &[&str]) -> anyhow::Result<String> {
    crate::procs::output(&app.env, "git", args, Some(std::path::Path::new(dir))).await
}

#[tokio::test]
async fn merge_ignores_terminals_and_unrelated_target_changes() {
    let root = std::env::temp_dir().join(format!("launchpad-merge-guards-{}", uuid::Uuid::new_v4()));
    let app = fixture(&root).await;
    // Return errors before asserting so even failures close every PTY and remove the fixtures.
    let result: anyhow::Result<()> = async {
        let repo = root.join("repo");
        std::fs::create_dir(&repo)?;
        let repo = repo.to_str().unwrap();
        let path = |name: &str| std::path::Path::new(repo).join(name);
        git(&app, repo, &["init", "-b", "main"]).await?;
        git(&app, repo, &["config", "user.name", "Test"]).await?;
        git(&app, repo, &["config", "user.email", "test@example.invalid"]).await?;
        std::fs::write(path("shared.txt"), "base\n")?;
        git(&app, repo, &["add", "."]).await?;
        git(&app, repo, &["commit", "-m", "base"]).await?;
        let project = crate::projects::register(&app, std::path::Path::new(repo), false).await?;
        let (w, _) = create(&app, "test-merge", &project, &crate::model::WorktreeOptions {
            base: None, include_changes: false, merge_when_done: false,
        }, true, "merge guards").await?;
        std::fs::write(std::path::Path::new(&w.root).join("shared.txt"), "task\n")?;
        std::fs::write(std::path::Path::new(&w.root).join("added.txt"), "task\n")?;
        git(&app, &w.root, &["add", "."]).await?;
        git(&app, &w.root, &["commit", "-m", "task"]).await?;
        // Main moved on with a new commit, and its checkout has unrelated untracked and modified files.
        std::fs::write(path("other.txt"), "main\n")?;
        git(&app, repo, &["add", "other.txt"]).await?;
        git(&app, repo, &["commit", "-m", "main change"]).await?;
        std::fs::write(path("untracked.txt"), "keep\n")?;
        std::fs::write(path("other.txt"), "local edit\n")?;
        app.terminals.open_shell(&app, &w.root, Some("sleep 120"), Some("sh")).await?;
        app.terminals.open_shell(&app, repo, Some("sleep 120"), Some("sh")).await?;
        // A local edit of a file the merge changes waits, keeps the edit and retries.
        std::fs::write(path("shared.txt"), "local edit\n")?;
        let pending = request_merge(&app, &w.id, false).await?;
        anyhow::ensure!(pending.merge.as_ref().is_some_and(|m| m.phase == MergePhase::Waiting && m.reason.contains("overwritten")), "overlapping change must wait: {:?}", pending.merge);
        anyhow::ensure!(std::fs::read_to_string(path("shared.txt"))? == "local edit\n");
        git(&app, repo, &["checkout", "--", "shared.txt"]).await?;
        // An untracked file the merge would add waits too.
        std::fs::write(path("added.txt"), "local\n")?;
        cancel_merge(&app, &w.id).await?;
        let pending = request_merge(&app, &w.id, false).await?;
        anyhow::ensure!(pending.merge.as_ref().is_some_and(|m| m.phase == MergePhase::Waiting), "untracked collision must wait: {:?}", pending.merge);
        std::fs::remove_file(path("added.txt"))?;
        cancel_merge(&app, &w.id).await?;
        let merged = request_merge(&app, &w.id, false).await?;
        anyhow::ensure!(merged.merge.is_none() && merged.merged.is_some(), "merge must succeed: {:?}", merged.merge);
        anyhow::ensure!(git(&app, repo, &["rev-parse", "HEAD"]).await? == git(&app, &w.root, &["rev-parse", "HEAD"]).await?);
        anyhow::ensure!(std::fs::read_to_string(path("untracked.txt"))? == "keep\n" && std::fs::read_to_string(path("other.txt"))? == "local edit\n");
        anyhow::ensure!(std::fs::read_to_string(path("shared.txt"))? == "task\n");
        anyhow::ensure!(!app.terminals.alive_in(&w.root).is_empty() && !app.terminals.alive_in(repo).is_empty(), "merge must leave terminals running");
        Ok(())
    }.await;
    let closed = app.terminals.close_all().await;
    drop(app);
    let removed = std::fs::remove_dir_all(root);
    closed.unwrap();
    removed.unwrap();
    result.unwrap();
}

#[tokio::test]
async fn committed_history_survives_merge_rebase_and_cleanup() {
    let root = std::env::temp_dir().join(format!("launchpad-commit-review-{}", uuid::Uuid::new_v4()));
    let app = fixture(&root).await;
    let result: anyhow::Result<()> = async {
        let repo = root.join("repo");
        std::fs::create_dir(&repo)?;
        let repo = repo.to_str().unwrap();
        git(&app, repo, &["init", "-b", "main"]).await?;
        git(&app, repo, &["config", "user.name", "Test"]).await?;
        git(&app, repo, &["config", "user.email", "test@example.invalid"]).await?;
        git(&app, repo, &["commit", "--allow-empty", "-m", "base"]).await?;
        let original_base = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        let project = crate::projects::register(&app, std::path::Path::new(repo), false).await?;
        let (w, _) = create(&app, "review-task", &project, &crate::model::WorktreeOptions::default(), true, "commit review").await?;
        std::fs::write(std::path::Path::new(&w.root).join("task.txt"), "first\n")?;
        git(&app, &w.root, &["add", "."]).await?;
        git(&app, &w.root, &["commit", "-m", "first task change"]).await?;
        std::fs::write(std::path::Path::new(&w.root).join("task.txt"), "first\nsecond\n")?;
        git(&app, &w.root, &["commit", "-am", "second task change"]).await?;
        let before = crate::session_git::history(&app, "review-task", None).await?.unwrap();
        anyhow::ensure!(before.commits.len() == 2 && before.merged_into.is_none());
        // A concurrent main commit forces a rebase; it must not enter the task's review range.
        std::fs::write(std::path::Path::new(repo).join("unrelated.txt"), "other work\n")?;
        git(&app, repo, &["add", "."]).await?;
        git(&app, repo, &["commit", "-m", "unrelated main change"]).await?;
        let target = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        let merged = request_merge(&app, &w.id, false).await?;
        anyhow::ensure!(merged.merged.as_ref().and_then(|m| m.base.as_ref()) == Some(&target));
        let all = crate::session_git::history(&app, "review-task", None).await?.unwrap();
        anyhow::ensure!(all.commits.len() == 2 && all.merged_into.as_deref() == Some("main"));
        anyhow::ensure!(all.files.len() == 1 && all.files[0].path == "task.txt");
        let patch = crate::session_git::committed_diff(&app, "review-task", "task.txt", None).await?;
        anyhow::ensure!(patch.unstaged.contains("+first") && patch.unstaged.contains("+second"));
        let second = &all.commits[0].id;
        let patch = crate::session_git::committed_diff(&app, "review-task", "task.txt", Some(second)).await?;
        anyhow::ensure!(patch.unstaged.contains("+second") && !patch.unstaged.contains("+first"));
        let patch = crate::session_git::committed_diff(&app, "review-task", "task.txt", Some(&all.commits[1].id)).await?;
        anyhow::ensure!(patch.unstaged.contains("+first") && !patch.unstaged.contains("+second"));
        anyhow::ensure!(crate::session_git::history(&app, "review-task", Some(&original_base)).await.is_err());
        anyhow::ensure!(crate::session_git::committed_diff(&app, "review-task", "unrelated.txt", None).await.is_err());
        // Advancing main and removing the worktree/branch must not erase its committed review.
        git(&app, repo, &["commit", "--allow-empty", "-m", "later main change"]).await?;
        remove(&app, &w.id).await?;
        let after = crate::session_git::history(&app, "review-task", None).await?.unwrap();
        anyhow::ensure!(after.head == all.head && after.base == all.base && after.commits.len() == 2);
        let patch = crate::session_git::committed_diff(&app, "review-task", "task.txt", None).await?;
        anyhow::ensure!(patch.unstaged.contains("+second"));
        Ok(())
    }.await;
    drop(app);
    let removed = std::fs::remove_dir_all(root);
    removed.unwrap();
    result.unwrap();
}

#[tokio::test]
async fn remote_sync_pulls_before_branching_and_keeps_merges_when_push_fails() {
    let root = std::env::temp_dir().join(format!("launchpad-remote-sync-{}", uuid::Uuid::new_v4()));
    let app = fixture(&root).await;
    let result: anyhow::Result<()> = async {
        let repo_path = root.join("repo");
        std::fs::create_dir(&repo_path)?;
        let repo = repo_path.to_str().unwrap();
        let remote_path = root.join("remote.git");
        let remote = remote_path.to_str().unwrap();
        git(&app, repo, &["init", "-b", "main"]).await?;
        git(&app, repo, &["config", "user.name", "Test"]).await?;
        git(&app, repo, &["config", "user.email", "test@example.invalid"]).await?;
        git(&app, repo, &["commit", "--allow-empty", "-m", "base"]).await?;
        let old = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        git(&app, repo, &["init", "--bare", remote]).await?;
        git(&app, repo, &["remote", "add", "origin", remote]).await?;
        // The local main tracks a differently named remote branch.
        git(&app, repo, &["push", "-u", "origin", "main:official"]).await?;
        git(&app, repo, &["switch", "-c", "remote-change"]).await?;
        git(&app, repo, &["commit", "--allow-empty", "-m", "remote advance"]).await?;
        let latest = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        git(&app, repo, &["push", "origin", "HEAD:official"]).await?;
        git(&app, repo, &["switch", "main"]).await?;
        anyhow::ensure!(git(&app, repo, &["rev-parse", "HEAD"]).await? == old);
        let project = crate::projects::register(&app, &repo_path, false).await?;
        let (w, _) = create(&app, "sync-task", &project, &crate::model::WorktreeOptions::default(), true, "remote sync").await?;
        anyhow::ensure!(w.base_commit == latest, "new task must start from the pulled upstream");
        git(&app, &w.root, &["commit", "--allow-empty", "-m", "task"]).await?;
        let merged = request_merge(&app, &w.id, false).await?;
        anyhow::ensure!(merged.merged.is_some() && merged.merge.is_none());
        let note = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let current = get(&app, &w.id).await.unwrap();
                if let Some(note) = current.remote_sync_note && note.contains("pushed main") { break note; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await?;
        anyhow::ensure!(note.contains("origin"));
        let head = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        anyhow::ensure!(git(&app, remote, &["rev-parse", "refs/heads/official"]).await? == head);
        anyhow::ensure!(git(&app, remote, &["show-ref", "--verify", &format!("refs/heads/{}", w.branch)]).await.is_err());
        std::fs::write(repo_path.join("untracked.txt"), "keep")?;
        let (dirty, _) = create(&app, "dirty-task", &project, &crate::model::WorktreeOptions::default(), true, "dirty").await?;
        anyhow::ensure!(dirty.base_commit == head && dirty.remote_sync_note.unwrap().contains("uncommitted changes"));
        anyhow::ensure!(std::fs::read_to_string(repo_path.join("untracked.txt"))? == "keep");
        std::fs::remove_file(repo_path.join("untracked.txt"))?;
        // A competing remote commit rejects the push without undoing the local merge.
        git(&app, repo, &["switch", "remote-change"]).await?;
        git(&app, repo, &["commit", "--allow-empty", "-m", "competing remote advance"]).await?;
        git(&app, repo, &["push", "origin", "+HEAD:official"]).await?;
        let competing = git(&app, repo, &["rev-parse", "HEAD"]).await?;
        git(&app, repo, &["switch", "main"]).await?;
        let note = crate::remote_sync::push(&app, repo, "main", &head).await;
        anyhow::ensure!(note.contains("pushing failed"), "{note}");
        anyhow::ensure!(git(&app, repo, &["rev-parse", "HEAD"]).await? == head);
        anyhow::ensure!(git(&app, remote, &["rev-parse", "refs/heads/official"]).await? == competing);
        // Divergence and a missing remote still allow a task to start from local main.
        let (diverged, _) = create(&app, "diverged-task", &project, &crate::model::WorktreeOptions::default(), true, "diverged").await?;
        anyhow::ensure!(diverged.base_commit == head && diverged.remote_sync_note.unwrap().contains("could not pull"));
        std::fs::remove_dir_all(&remote_path)?;
        let (offline, _) = create(&app, "offline-task", &project, &crate::model::WorktreeOptions::default(), true, "offline").await?;
        anyhow::ensure!(offline.base_commit == head && offline.remote_sync_note.unwrap().contains("could not pull"));
        Ok(())
    }.await;
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
    result.unwrap();
}
