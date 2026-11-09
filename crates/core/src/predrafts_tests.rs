//! Local PR checks: pure functions only.
#[test]
fn fork_url_keeps_the_origin_scheme() {
    use super::fork_url;
    assert_eq!(fork_url("git@github.com:huggingface/lerobot.git", "huggingface/lerobot", "Wauplin/lerobot").as_deref(), Some("git@github.com:Wauplin/lerobot.git"));
    assert_eq!(fork_url("https://github.com/huggingface/lerobot/", "huggingface/lerobot", "Wauplin/lerobot").as_deref(), Some("https://github.com/Wauplin/lerobot"));
    assert_eq!(fork_url("https://github.com/other/name.git", "huggingface/lerobot", "Wauplin/lerobot"), None);
}

#[test]
fn change_set_links_replace_the_previous_list() {
    let once = super::with_links("Body", &["a".into()]);
    assert_eq!(super::with_links(&once, &["a".into(), "b".into()]), "Body\n\n---\nPart of a change set: a, b");
    assert_eq!(super::with_links(&once, &[]), "Body");
}

fn local_pr(id: &str, branch: &str) -> super::Predraft {
    super::Predraft {
        id: id.into(),
        project_id: "folder".into(),
        branch: branch.into(),
        base_branch: "main".into(),
        initial_base: "base0".into(),
        title: "Title".into(),
        body: String::new(),
        session_id: Some("s".into()),
        notes: vec![],
        archived: false,
        published_url: None,
        published_number: None,
        default_remote_branch: "local/x".into(),
        remote_branch: None,
        pushed_head: None,
        push_repo: None,
        created_at: 0,
        remote_sync_note: None,
        merged: None,
        change_set: None,
    }
}

#[test]
fn worktree_merge_closes_the_open_local_pr_of_its_branch() {
    let mut discarded = local_pr("discarded", "lp/a");
    discarded.archived = true;
    let mut published = local_pr("published", "lp/a");
    published.published_url = Some("https://github.com/o/r/pull/1".into());
    let other_folder = super::Predraft { project_id: "other".into(), ..local_pr("other-folder", "lp/a") };
    let list = vec![local_pr("open", "lp/a"), local_pr("other-branch", "lp/b"), discarded, published, other_folder];
    let changed = super::mark_merged(list, "folder", "lp/a", "abc123", 42);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].id, "open");
    assert_eq!(changed[0].merged, Some(super::LocalMerge { head: "abc123".into(), at: 42 }));
    assert!(!changed[0].is_open());
}

#[test]
fn a_branch_contained_in_its_base_reads_as_merged() {
    let p = local_pr("p", "lp/a");
    // Committed on, then fully merged into main: 0 commits ahead.
    assert!(super::is_contained(&p, "commit1", 0));
    // Still ahead of main: under review.
    assert!(!super::is_contained(&p, "commit1", 2));
    // Opened before any commit: an empty but open Local PR.
    assert!(!super::is_contained(&p, "base0", 0));
    // Already closed records stay as they are.
    let mut merged = local_pr("m", "lp/a");
    merged.merged = Some(super::LocalMerge { head: "commit1".into(), at: 1 });
    assert!(!super::is_contained(&merged, "commit1", 0));
    let mut published = local_pr("pub", "lp/a");
    published.published_url = Some("u".into());
    assert!(!super::is_contained(&published, "commit1", 0));
}
