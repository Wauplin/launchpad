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
async fn force_merge_only_overrides_terminals() {
    let root = std::env::temp_dir().join(format!("launchpad-force-merge-{}", uuid::Uuid::new_v4()));
    let app = fixture(&root).await;
    // Return errors before asserting so even failures close every PTY and remove the fixtures.
    let result: anyhow::Result<()> = async {
        let repo = root.join("repo");
        std::fs::create_dir(&repo)?;
        let repo = repo.to_str().unwrap();
        git(&app, repo, &["init", "-b", "main"]).await?;
        git(&app, repo, &["config", "user.name", "Test"]).await?;
        git(&app, repo, &["config", "user.email", "test@example.invalid"]).await?;
        git(&app, repo, &["commit", "--allow-empty", "-m", "base"]).await?;
        let project = crate::projects::register(&app, std::path::Path::new(repo), false).await?;
        for (index, source) in [false, true].into_iter().enumerate() {
            let (w, _) = create(&app, &format!("test-{index}"), &project, &crate::model::WorktreeOptions {
                base: None, include_changes: false, merge_when_done: false,
            }, true, "terminal merge").await?;
            git(&app, &w.root, &["commit", "--allow-empty", "-m", "task"]).await?;
            let head = git(&app, &w.root, &["rev-parse", "HEAD"]).await?;
            let terminal_dir = if source { &w.root } else { repo };
            app.terminals.open_shell(&app, terminal_dir, Some("sleep 120"), Some("sh")).await?;
            let pending = request_merge(&app, &w.id, false).await?;
            anyhow::ensure!(pending.merge.as_ref().is_some_and(|m| m.terminal_blocked && !m.ignore_terminals), "normal merge must wait for the terminal");
            // Persisted records retain the override, but old records default to normal checks.
            let old: MergeRequest = serde_json::from_value(serde_json::json!({"phase":"waiting", "reason":"old", "by_agent":false, "agent_retries":0, "requested_at":0}))?;
            anyhow::ensure!(!old.ignore_terminals && !old.terminal_blocked);
            std::fs::write(std::path::Path::new(repo).join("dirty"), "keep")?;
            let pending = force_merge(&app, &w.id).await?;
            anyhow::ensure!(pending.merge.as_ref().is_some_and(|m| m.ignore_terminals && !m.terminal_blocked && m.reason.contains("uncommitted")), "override must keep the dirty-target guard");
            let persisted = get(&app, &w.id).await?;
            anyhow::ensure!(persisted.merge.unwrap().ignore_terminals);
            std::fs::remove_file(std::path::Path::new(repo).join("dirty"))?;
            // Cancellation and a fresh request remove the override.
            cancel_merge(&app, &w.id).await?;
            let pending = request_merge(&app, &w.id, false).await?;
            anyhow::ensure!(pending.merge.unwrap().terminal_blocked);
            std::fs::write(std::path::Path::new(&w.root).join("dirty-source"), "keep")?;
            let blocked = force_merge(&app, &w.id).await?;
            anyhow::ensure!(blocked.merge.as_ref().is_some_and(|m| m.phase == MergePhase::Conflict && m.reason.contains("uncommitted")), "override must keep the dirty-source guard");
            std::fs::remove_file(std::path::Path::new(&w.root).join("dirty-source"))?;
            request_merge(&app, &w.id, false).await?;
            let merged = force_merge(&app, &w.id).await?;
            anyhow::ensure!(merged.merge.is_none() && merged.merged.is_some());
            anyhow::ensure!(git(&app, repo, &["rev-parse", "HEAD"]).await? == head);
            anyhow::ensure!(!app.terminals.alive_in(terminal_dir).is_empty(), "force merge must leave the terminal running");
            anyhow::ensure!(force_merge(&app, &w.id).await.is_err(), "no override without a pending merge");
            app.terminals.close_all().await?;
        }
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
