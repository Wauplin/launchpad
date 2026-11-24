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
        linked_at: None,
        discarded_head: None,
        github_closed: false,
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
    assert_eq!(changed[0].merged, Some(super::LocalMerge { head: "abc123".into(), at: 42, github: false }));
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
    merged.merged = Some(super::LocalMerge { head: "commit1".into(), at: 1, github: false });
    assert!(!super::is_contained(&merged, "commit1", 0));
    let mut published = local_pr("pub", "lp/a");
    published.published_url = Some("u".into());
    assert!(!super::is_contained(&published, "commit1", 0));
}

#[test]
fn bots_publish_only_their_own_drafts() {
    use super::publish_refusal;
    let draft = serde_json::json!({"remote_branch": "fix", "draft": true, "head": "abc"});
    assert_eq!(publish_refusal(true, true, Some(&draft)), None);
    // Not turned on, or another bot's Local PR.
    assert!(publish_refusal(false, true, Some(&draft)).is_some());
    assert!(publish_refusal(true, false, Some(&draft)).is_some());
    // Ready for review or from a fork stays the owner's.
    assert!(publish_refusal(true, true, Some(&serde_json::json!({"remote_branch": "fix", "draft": false, "head": "abc"}))).is_some());
    assert!(publish_refusal(true, true, Some(&serde_json::json!({"remote_branch": "fix", "head": "abc"}))).is_some());
    assert!(publish_refusal(true, true, Some(&serde_json::json!({"remote_branch": "fix", "draft": true, "fork": true, "head": "abc"}))).is_some());
}

const HEAD: &str = "9ca18f08b5bb564a169090102b73cdc5fbe3546d";
const ORIGIN: &str = "huggingface/huggingface_hub";

/// GitHub's answer for a pull request, as `GET /repos/{repo}/pulls/{n}` gives it.
fn pr_json(merged_at: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "number": 5369,
        "html_url": "https://github.com/huggingface/huggingface_hub/pull/5369",
        "state": if merged_at.is_some() { "closed" } else { "open" },
        "merged": merged_at.is_some(),
        "merged_at": merged_at,
        "merge_commit_sha": "1111111111111111111111111111111111111111",
        "base": {"ref": "main", "repo": {"full_name": "huggingface/huggingface_hub"}},
        "head": {"ref": "fix-ty-0.0.105-hf-xet", "sha": HEAD, "repo": {"full_name": "Wauplin/huggingface_hub"}},
    })
}

#[test]
fn pull_request_references_must_name_the_folders_repository() {
    use super::parse_pr_ref;
    for ok in ["5369", "#5369", " huggingface/huggingface_hub#5369 ", "https://github.com/huggingface/huggingface_hub/pull/5369", "https://github.com/HuggingFace/huggingface_hub/pull/5369/files#diff"] {
        assert_eq!(parse_pr_ref(ok, ORIGIN).unwrap(), 5369, "{ok}");
    }
    for bad in ["other/repo#5369", "https://github.com/other/repo/pull/5369", "https://github.com/huggingface/huggingface_hub/issues/5369", "abc", "0", ""] {
        assert!(parse_pr_ref(bad, ORIGIN).is_err(), "{bad}");
    }
}

#[test]
fn github_answers_are_read_strictly() {
    let pr = super::pr_facts(&pr_json(Some("2026-11-17T13:10:48Z"))).unwrap();
    assert_eq!(pr.merged_at, Some(1_794_921_048_000));
    assert_eq!(pr.head_repo.as_deref(), Some("Wauplin/huggingface_hub"));
    assert_eq!(super::pr_facts(&pr_json(None)).unwrap().merged_at, None);
    // A malformed answer is an error, never "not merged".
    for (pointer, value) in [("/merged_at", serde_json::json!("yesterday")), ("/head/sha", serde_json::json!("abc")), ("/state", serde_json::json!("weird")), ("/base", serde_json::Value::Null)] {
        let mut v = pr_json(None);
        *v.pointer_mut(pointer).unwrap() = value;
        assert!(super::pr_facts(&v).is_err(), "{pointer}");
    }
    let mut v = pr_json(None);
    v["merged"] = serde_json::json!(true);
    assert!(super::pr_facts(&v).is_err());
}

#[test]
fn linking_checks_repository_base_and_the_reviewed_head() {
    let p = local_pr("p", "lp/owner-request-fix-quality-99f2ce13");
    let pr = super::pr_facts(&pr_json(None)).unwrap();
    super::check_link(&p, ORIGIN, 5369, &pr, HEAD, HEAD).unwrap();
    // Another repository (a fork's own pull request), another base, another number.
    let fork = super::PrFacts { base_repo: "Wauplin/huggingface_hub".into(), ..pr.clone() };
    assert!(super::check_link(&p, ORIGIN, 5369, &fork, HEAD, HEAD).is_err());
    let url = super::PrFacts { url: "https://github.com/Wauplin/huggingface_hub/pull/5369".into(), ..pr.clone() };
    assert!(super::check_link(&p, ORIGIN, 5369, &url, HEAD, HEAD).is_err());
    let base = super::PrFacts { base_ref: "v1.0-release".into(), ..pr.clone() };
    assert!(super::check_link(&p, ORIGIN, 5369, &base, HEAD, HEAD).is_err());
    assert!(super::check_link(&p, ORIGIN, 5370, &pr, HEAD, HEAD).is_err());
    // GitHub's head differs from the branch on Home, or the branch moved since review.
    let other = "2222222222222222222222222222222222222222";
    assert!(super::check_link(&p, ORIGIN, 5369, &pr, other, other).is_err());
    assert!(super::check_link(&p, ORIGIN, 5369, &pr, HEAD, other).is_err());
    assert!(super::check_link(&p, ORIGIN, 5369, &pr, other, HEAD).is_err());
}

#[test]
fn a_link_is_idempotent_and_refuses_conflicting_records() {
    let mut p = local_pr("p", "lp/a");
    assert!(!super::link_state(&p, ORIGIN, 5369).unwrap());
    let pr = super::pr_facts(&pr_json(None)).unwrap();
    super::apply_link(&mut p, ORIGIN, &pr, 7);
    // Linked: no push claimed, Home follows nothing, the fork is where the branch lives.
    assert_eq!((p.published_number, p.linked_at, p.pushed_head.as_deref()), (Some(5369), Some(7), None));
    assert_eq!(p.push_repo.as_deref(), Some("Wauplin/huggingface_hub"));
    assert_eq!(p.remote_branch.as_deref(), Some("fix-ty-0.0.105-hf-xet"));
    // The same link again changes nothing; another pull request is refused.
    assert!(super::link_state(&p, ORIGIN, 5369).unwrap());
    assert!(super::link_state(&p, "HUGGINGFACE/huggingface_hub", 5369).unwrap());
    assert!(super::link_state(&p, ORIGIN, 1).is_err());
    let mut started = local_pr("s", "lp/a");
    started.pushed_head = Some(HEAD.into());
    assert!(super::link_state(&started, ORIGIN, 5369).is_err());
    let mut discarded = local_pr("d", "lp/a");
    discarded.archived = true;
    assert!(super::link_state(&discarded, ORIGIN, 5369).is_err());
    let mut merged = local_pr("m", "lp/a");
    merged.merged = Some(super::LocalMerge { head: HEAD.into(), at: 1, github: false });
    assert!(super::link_state(&merged, ORIGIN, 5369).is_err());
}

#[test]
fn a_pull_request_merged_on_github_closes_its_local_pr() {
    use super::Sync;
    let mut p = local_pr("p", "lp/a");
    p.published_url = Some("https://github.com/huggingface/huggingface_hub/pull/5369".into());
    p.published_number = Some(5369);
    // Squash, rebase or merge: GitHub's merged_at decides, never ancestry on Home.
    let merged = super::pr_facts(&pr_json(Some("2026-11-17T13:10:48Z"))).unwrap();
    let Sync::Merged(m) = super::github_sync(&p, &merged, true) else { panic!("not merged") };
    assert_eq!((m.head.as_str(), m.at, m.github), ("1111111111111111111111111111111111111111", 1_794_921_048_000, true));
    // Newer commits on Home than the merged head: a note, never merged.
    let Sync::Note(note) = super::github_sync(&p, &merged, false) else { panic!("no note") };
    assert!(note.contains("newer commits"));
    p.remote_sync_note = Some(note);
    assert_eq!(super::github_sync(&p, &merged, false), Sync::Nothing);
    // Open: nothing; closed without merging: a note.
    let open = super::pr_facts(&pr_json(None)).unwrap();
    assert_eq!(super::github_sync(&p, &open, true), Sync::Nothing);
    let closed = super::PrFacts { closed: true, ..open };
    assert!(matches!(super::github_sync(&p, &closed, true), Sync::Note(n) if n.contains("without merging")));
    // Records already merged or discarded stay as they are.
    let mut done = p.clone();
    done.merged = Some(super::LocalMerge { head: HEAD.into(), at: 1, github: false });
    assert_eq!(super::github_sync(&done, &merged, true), Sync::Nothing);
}

#[test]
fn reopen_restores_the_discarded_tip_unless_the_branch_is_back() {
    let mut p = local_pr("p", "lp/a");
    assert!(super::reopen_restores(&p, Some("t1"), false).is_err(), "an open Local PR is not discarded");
    p.archived = true;
    p.discarded_head = Some("t1".into());
    assert_eq!(super::reopen_restores(&p, None, false).unwrap(), Some("t1".into()));
    // The branch is on Home again: it stays as it is.
    assert_eq!(super::reopen_restores(&p, Some("t2"), false).unwrap(), None);
    assert!(super::reopen_restores(&p, None, true).is_err(), "another open Local PR has the branch");
    p.discarded_head = None;
    assert!(super::reopen_restores(&p, None, false).is_err(), "no recorded tip and no branch");
    p.discarded_head = Some("t1".into());
    p.merged = Some(super::LocalMerge { head: "m".into(), at: 1, github: false });
    assert!(super::reopen_restores(&p, None, false).is_err());
}

#[test]
fn a_branch_keeps_one_local_pr_and_the_published_one_wins() {
    use super::pick_for_branch;
    let url = Some("https://github.com/o/r/pull/1".to_string());
    let published = super::Predraft { published_url: url.clone(), created_at: 1, ..local_pr("published", "lp/a") };
    let duplicate = super::Predraft { created_at: 2, ..local_pr("duplicate", "lp/a") };
    let discarded = super::Predraft { archived: true, created_at: 3, ..local_pr("discarded", "lp/a") };
    let list = vec![published.clone(), duplicate.clone(), discarded];
    // `work finish` and `pr create` continue the published one; `pr show|update` read it too.
    assert_eq!(pick_for_branch(&list, "folder", "lp/a", true).map(|p| p.id.as_str()), Some("published"));
    assert_eq!(pick_for_branch(&list, "folder", "lp/a", false).map(|p| p.id.as_str()), Some("published"));
    assert!(pick_for_branch(&list, "other", "lp/a", true).is_none());
    // Its pull request closed on GitHub: a new Local PR, and the open one wins for reads.
    let closed = super::Predraft { github_closed: true, ..published.clone() };
    assert!(pick_for_branch(std::slice::from_ref(&closed), "folder", "lp/a", true).is_none());
    assert_eq!(pick_for_branch(std::slice::from_ref(&closed), "folder", "lp/a", false).map(|p| p.id.as_str()), Some("published"));
    assert_eq!(pick_for_branch(&[closed, duplicate], "folder", "lp/a", true).map(|p| p.id.as_str()), Some("duplicate"));
    // Merged records are never continued.
    let merged = super::Predraft { merged: Some(super::LocalMerge { head: "h".into(), at: 1, github: true }), ..published };
    assert!(pick_for_branch(&[merged], "folder", "lp/a", false).is_none());
}

#[test]
fn only_pull_requests_launchpad_published_change_on_github() {
    use super::{published_refusal, published_text_refusal};
    let open = local_pr("p", "lp/a");
    assert!(published_refusal(&open).is_none());
    assert!(published_text_refusal(&open).is_some());
    let published = super::Predraft { published_url: Some("https://github.com/o/r/pull/1".into()), published_number: Some(1), ..open };
    // The record alone never changes: `--github` changes both.
    assert!(published_refusal(&published).unwrap().contains("--github"));
    assert_eq!(published_text_refusal(&published), None);
    let linked = super::Predraft { linked_at: Some(1), ..published.clone() };
    assert!(published_refusal(&linked).unwrap().contains("outside Launchpad"));
    assert!(published_text_refusal(&linked).is_some());
    let discarded = super::Predraft { archived: true, ..published };
    assert!(published_refusal(&discarded).is_none() && published_text_refusal(&discarded).is_some());
    // A bot changes its own pull request's text; the draft rule is publication's.
    assert_eq!(super::publish_refusal(true, true, None), None);
    assert!(super::publish_refusal(true, false, None).is_some());
}

#[test]
fn published_sync_state_reads_in_words() {
    use crate::home_git::FollowStatus;
    let url = "https://github.com/o/r/pull/1";
    let open = local_pr("p", "lp/a");
    assert_eq!(super::sync_words(&open, None), None);
    let p = super::Predraft { published_url: Some(url.into()), ..open };
    let follow = |head: &str, forwarded: &str, error: Option<&str>| FollowStatus { url: url.into(), head: Some(head.into()), forwarded: Some(forwarded.into()), behind: head != forwarded, error: error.map(str::to_string) };
    assert!(super::sync_words(&p, Some(&follow("aaa", "aaa", None))).unwrap().starts_with("Up to date"));
    assert!(super::sync_words(&p, Some(&follow("bbb", "aaa", None))).unwrap().contains("not on GitHub yet"));
    assert!(super::sync_words(&p, Some(&follow("bbb", "aaa", Some("rejected")))).unwrap().contains("rejected"));
    assert!(super::sync_words(&p, None).unwrap().contains("no follow"));
    let linked = super::Predraft { linked_at: Some(1), ..p.clone() };
    assert!(super::sync_words(&linked, Some(&follow("aaa", "aaa", None))).unwrap().contains("pushes nothing"));
    let closed = super::Predraft { github_closed: true, ..p };
    assert!(super::sync_words(&closed, None).unwrap().contains("closed on GitHub"));
}
