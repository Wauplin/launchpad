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
