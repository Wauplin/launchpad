//! Turns a launch spec into the prompt sent to the agent (ARCHITECTURE.md, section 9.7).
//! Deterministic: the composer preview and the launch use the same function.

use crate::config::Preset;
use crate::model::{GitHubRef, LaunchSpec};

/// A context project, resolved from the registry.
pub struct ContextInfo {
    pub path: String,
    pub description: String,
    pub branch: Option<String>,
}

/// Order: free text, then context, then presets (in the order defined in settings).
pub fn render_prompt(spec: &LaunchSpec, context: &[ContextInfo], presets: &[Preset]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let text = spec.text.trim();
    if !text.is_empty() {
        parts.push(text.to_string());
    }

    let mut lines: Vec<String> = Vec::new();
    for r in &spec.github_refs {
        lines.push(format!("- {}", github_line(r)));
    }
    for c in context {
        let desc = if c.description.trim().is_empty() { String::new() } else { format!(" ({})", c.description.trim()) };
        lines.push(match &c.branch {
            Some(b) => format!(
                "- Reference project: {path}{desc}, branch `{b}`. Read it without changing its checkout, for example with `git -C {path} show {b}:<path>` or a temporary worktree. Do not modify it.",
                path = c.path
            ),
            None => format!("- Reference project: {}{desc}. Read it, but do not modify it.", c.path),
        });
    }
    if !lines.is_empty() {
        let mut block = format!("Context:\n{}", lines.join("\n"));
        if !spec.github_refs.is_empty() {
            block.push_str("\nUse `gh` to read issues, pull requests and comments.");
        }
        parts.push(block);
    }

    let chosen: Vec<&str> =
        presets.iter().filter(|p| spec.presets.iter().any(|id| id == &p.id)).map(|p| p.text.trim()).filter(|t| !t.is_empty()).collect();
    if !chosen.is_empty() {
        parts.push(chosen.join(" "));
    }
    parts.join("\n\n")
}

fn github_line(r: &GitHubRef) -> String {
    let title = r.title.as_deref().map(|t| format!(" (\"{}\")", t.replace('"', "'"))).unwrap_or_default();
    match r.kind.as_str() {
        "pr" => format!("Pull request: {}{title}", r.url),
        "comment" | "review_comment" => format!("Comment: {}{}", r.url, r.title.as_deref().map(|t| format!(" (on \"{}\")", t.replace('"', "'"))).unwrap_or_default()),
        _ => format!("Issue: {}{title}", r.url),
    }
}

/// Session title: the first line of the free text, cut at a word boundary.
pub fn title_from(text: &str, fallback: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.is_empty() {
        return fallback.to_string();
    }
    let max = 60;
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > 20 => &cut[..i],
        _ => &cut[..],
    };
    format!("{}…", cut.trim_end_matches(|c: char| !c.is_alphanumeric()))
}

/// Slug for a temporary folder: the first words of the prompt (`tmp-1002-xet-upload-bench`).
pub fn slug_from(text: &str) -> String {
    const SKIP: &[&str] = &["a", "an", "the", "to", "of", "in", "on", "for", "and", "or", "is", "it", "if", "please", "can", "you", "with", "my", "me", "i", "we"];
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !SKIP.contains(&w.as_str()))
        .take(4)
        .collect();
    let slug = words.join("-");
    let slug: String = slug.chars().take(32).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() { "scratch".into() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Target;

    fn presets() -> Vec<Preset> {
        crate::config::Config::default().presets
    }

    #[test]
    fn full_prompt() {
        let spec = LaunchSpec {
            target: Target::Project { project_id: "p".into() },
            text: "  Check if #3412 is still relevant and whether #3420 fixes it.\n".into(),
            // Selected out of order: the settings order wins.
            presets: vec!["no-unit-test".into(), "worktree".into()],
            github_refs: vec![
                GitHubRef {
                    url: "https://github.com/huggingface/huggingface_hub/issues/3412".into(),
                    kind: "issue".into(),
                    repo: "huggingface/huggingface_hub".into(),
                    number: 3412,
                    title: Some("hf download ignores --revision when used with --include".into()),
                },
                GitHubRef {
                    url: "https://github.com/huggingface/huggingface_hub/pull/3420".into(),
                    kind: "pr".into(),
                    repo: "huggingface/huggingface_hub".into(),
                    number: 3420,
                    title: Some("Fix \"revision\" handling".into()),
                },
            ],
            ..Default::default()
        };
        let ctx = vec![ContextInfo {
            path: "/home/u/projects/moon-landing".into(),
            description: "server-side codebase".into(),
            branch: Some("xet-refresh-token".into()),
        }];
        let expected = "Check if #3412 is still relevant and whether #3420 fixes it.

Context:
- Issue: https://github.com/huggingface/huggingface_hub/issues/3412 (\"hf download ignores --revision when used with --include\")
- Pull request: https://github.com/huggingface/huggingface_hub/pull/3420 (\"Fix 'revision' handling\")
- Reference project: /home/u/projects/moon-landing (server-side codebase), branch `xet-refresh-token`. Read it without changing its checkout, for example with `git -C /home/u/projects/moon-landing show xet-refresh-token:<path>` or a temporary worktree. Do not modify it.
Use `gh` to read issues, pull requests and comments.

Please use a local git worktree and delete it once you're done. No need to add any unit tests for that PR.";
        assert_eq!(render_prompt(&spec, &ctx, &presets()), expected);
    }

    #[test]
    fn text_only_and_empty_parts() {
        let spec = LaunchSpec { text: "Fix the bug".into(), ..Default::default() };
        assert_eq!(render_prompt(&spec, &[], &presets()), "Fix the bug");
        let spec = LaunchSpec { presets: vec!["draft-pr".into()], ..Default::default() };
        assert_eq!(
            render_prompt(&spec, &[], &presets()),
            "Open a draft PR once you are done and check CI if anything related to the PR is failing."
        );
        let ctx = vec![ContextInfo { path: "/p/js".into(), description: " ".into(), branch: None }];
        assert_eq!(
            render_prompt(&LaunchSpec::default(), &ctx, &[]),
            "Context:\n- Reference project: /p/js. Read it, but do not modify it."
        );
    }

    #[test]
    fn titles_and_slugs() {
        assert_eq!(title_from("\n  Fix the flaky test\nmore", "x"), "Fix the flaky test");
        assert_eq!(title_from("   ", "New session"), "New session");
        let long = "Benchmark upload throughput for 64, 128 and 256 MB chunks on a 2 GB file in a private repo";
        assert_eq!(title_from(long, "x"), "Benchmark upload throughput for 64, 128 and 256 MB chunks…");
        assert_eq!(slug_from("Please benchmark the Xet upload bench for me"), "benchmark-xet-upload-bench");
        assert_eq!(slug_from("!!!"), "scratch");
    }
}
