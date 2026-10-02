use std::{path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    // Git emits the actual paths, including linked worktree metadata.
    for name in ["HEAD", "refs/heads/main", "packed-refs", "index"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            &reference,
        ])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=LAUNCHPAD_COMMIT={commit}");
    let worktrees = git(&["worktree", "list", "--porcelain"]).unwrap_or_default();
    let main = worktrees.split("\n\n").find_map(|block| {
        if block.lines().any(|line| line == "branch refs/heads/main") {
            block
                .lines()
                .find_map(|line| line.strip_prefix("worktree "))
        } else {
            None
        }
    });
    println!(
        "cargo:rustc-env=LAUNCHPAD_SOURCE={}",
        main.map(PathBuf::from).unwrap_or(root).display()
    );
}
