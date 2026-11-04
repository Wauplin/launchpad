//! Installs registry tools on a runner, in its own folder (`<data_dir>/tools/<id>-<hash>/bin`), once per install
//! recipe. Package-manager installs run in a jail that can write only that folder and the workspace cache.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::App;
use crate::vault::{InstallKind, Tool};

fn arch() -> (&'static str, &'static str) {
    match std::env::consts::ARCH {
        "aarch64" => ("arm64", "aarch64"),
        _ => ("amd64", "x86_64"),
    }
}

fn expand(s: &str) -> String {
    let (a, u) = arch();
    s.replace("{arch}", a).replace("{uarch}", u)
}

/// `*` matches any run of characters.
pub fn glob(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let mut rest = name;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else { return false };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

static LOCKS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = std::sync::LazyLock::new(Default::default);

/// Installs a tool (if needed) and returns its `bin` folder.
pub async fn ensure(app: &App, tool: &Tool, cache: &Path) -> Result<PathBuf> {
    let install = tool.install.as_ref().context("nothing to install")?;
    let recipe = serde_json::to_string(install)?;
    let hash = hex::encode(&Sha256::digest(recipe.as_bytes())[..6]);
    let dir = app.paths.data_dir.join("tools").join(format!("{}-{hash}", tool.id));
    let bin = dir.join("bin");
    let lock = LOCKS.lock().unwrap().entry(dir.to_string_lossy().into_owned()).or_default().clone();
    let _guard = lock.lock().await;
    if dir.join(".installed").exists() {
        return Ok(bin);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&bin)?;
    let result = match install.kind {
        InstallKind::Url => download(&expand(&install.source), &dir, &tool.id).await,
        InstallKind::GithubRelease => {
            let url = github_asset(&install.source, &expand(&install.asset)).await?;
            download(&url, &dir, &tool.id).await
        }
        InstallKind::Cargo => jailed(app, &dir, cache, "cargo", &["install", "--root", &dir.to_string_lossy(), &install.source]).await,
        InstallKind::Uv => {
            let tools = dir.join("uv");
            jailed(app, &dir, cache, "env", &[&format!("UV_TOOL_DIR={}", tools.display()), &format!("UV_TOOL_BIN_DIR={}", bin.display()), "uv", "tool", "install", &install.source]).await
        }
        InstallKind::Npm => jailed(app, &dir, cache, "npm", &["install", "--global", "--prefix", &dir.to_string_lossy(), &install.source]).await,
    };
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    std::fs::write(dir.join(".installed"), recipe)?;
    Ok(bin)
}

/// The download URL of the newest release asset matching `pattern`.
async fn github_asset(repo: &str, pattern: &str) -> Result<String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).user_agent("launchpad").build()?;
    let v: serde_json::Value = client.get(format!("https://api.github.com/repos/{repo}/releases/latest")).send().await?.error_for_status()?.json().await?;
    v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| glob(pattern, n)))
        .and_then(|a| a["browser_download_url"].as_str().map(str::to_string))
        .with_context(|| format!("no asset of {repo} matches {pattern}"))
}

async fn download(url: &str, dir: &Path, id: &str) -> Result<()> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).user_agent("launchpad").build()?;
    let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
    let name = url.rsplit('/').next().unwrap_or(id).to_string();
    let file = dir.join(&name);
    std::fs::write(&file, &bytes)?;
    let unpack = dir.join("pkg");
    std::fs::create_dir_all(&unpack)?;
    let archive = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(("tar", vec!["-xzf".to_string(), file.to_string_lossy().into(), "-C".into(), unpack.to_string_lossy().into()]))
    } else if name.ends_with(".tar.xz") {
        Some(("tar", vec!["-xJf".to_string(), file.to_string_lossy().into(), "-C".into(), unpack.to_string_lossy().into()]))
    } else if name.ends_with(".zip") {
        Some(("unzip", vec!["-q".to_string(), file.to_string_lossy().into(), "-d".into(), unpack.to_string_lossy().into()]))
    } else {
        None
    };
    let bin = dir.join("bin");
    match archive {
        Some((program, args)) => {
            let out = tokio::process::Command::new(program).args(&args).output().await.with_context(|| format!("run {program}"))?;
            anyhow::ensure!(out.status.success(), "{program}: {}", String::from_utf8_lossy(&out.stderr).trim());
            let _ = std::fs::remove_file(&file);
            let found = executables(&unpack, 4);
            anyhow::ensure!(!found.is_empty(), "the archive has no executable");
            for exe in found {
                let link = bin.join(exe.file_name().context("name")?);
                if !link.exists() {
                    std::os::unix::fs::symlink(&exe, &link)?;
                }
            }
        }
        None => {
            let target = bin.join(id);
            std::fs::rename(&file, &target)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Executable files of an unpacked archive, preferring those in a `bin` folder.
fn executables(root: &Path, depth: usize) -> Vec<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let mut all = vec![];
    let mut stack = vec![(root.to_path_buf(), depth)];
    while let Some((dir, d)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
            if meta.is_dir() && d > 0 {
                stack.push((p, d - 1));
            } else if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                all.push(p);
            }
        }
    }
    let in_bin: Vec<PathBuf> = all.iter().filter(|p| p.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "bin")).cloned().collect();
    if in_bin.is_empty() { all } else { in_bin }
}

/// Runs an install command in a jail that can write only the tool folder and the cache.
async fn jailed(app: &App, dir: &Path, cache: &Path, program: &str, args: &[&str]) -> Result<()> {
    let tmp = dir.join(".tmp");
    std::fs::create_dir_all(&tmp)?;
    let mut ro = vec![];
    if let Some(h) = dirs::home_dir() {
        ro.extend(crate::jail::HOME_TOOLCHAINS.iter().map(|d| h.join(d)).filter(|d| d.exists()));
    }
    let cache_s = cache.to_string_lossy().into_owned();
    let env = vec![
        ("HOME".to_string(), tmp.to_string_lossy().into_owned()),
        ("XDG_CACHE_HOME".into(), cache_s.clone()),
        ("CARGO_HOME".into(), format!("{cache_s}/cargo")),
        ("npm_config_cache".into(), format!("{cache_s}/npm")),
    ];
    let jail = crate::jail::Jail { rw: vec![dir.to_path_buf(), cache.to_path_buf()], ro, tmp: tmp.clone(), env, binds: vec![] };
    let out = crate::procs::command(&app.env.jailed(Some(std::sync::Arc::new(jail))), program, Some(dir))
        .args(args)
        .stdin(crate::exec::Io::Null)
        .output()
        .await
        .with_context(|| format!("run {program}"))?;
    let _ = std::fs::remove_dir_all(&tmp);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(5).collect();
        bail!("{program} failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn glob_matches_release_assets() {
        assert!(super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_amd64.tar.gz"));
        assert!(!super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_arm64.tar.gz"));
        assert!(!super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_amd64.tar.gz.sig"));
        assert!(super::glob("tool", "tool"));
    }
}
