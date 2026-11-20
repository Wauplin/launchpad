//! Installs registry tools on a runner, in its own folder (`<data_dir>/tools/<id>-<hash>/bin`), once per install
//! recipe. Package-manager installs run in a jail that can write only that folder and the workspace cache.
//!
//! GitHub releases: Home resolves the newest release with its own GitHub login and caches it (`releases`); the
//! lease carries it, so a runner downloads from github.com without calling GitHub's API (whose unauthenticated
//! limit is per address, shared by every Burst job). Without it, or when it fails, the runner reads the newest tag
//! from github.com's `releases/latest` redirect, then tries the API (except on Burst).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::App;
use crate::util::now_ms;
use crate::vault::{InstallKind, Tool};

/// A GitHub release of a `github_release` tool, resolved by Home: its tag and the names of its assets (the runner
/// picks the one of its architecture).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<String>,
}

/// How long Home reuses a resolved release, and waits after a failed lookup before the next one.
const RELEASE_TTL_MS: i64 = 60 * 60 * 1000;
const RELEASE_RETRY_MS: i64 = 5 * 60 * 1000;

struct Known {
    release: Option<Release>,
    checked: i64,
}

static RELEASES: LazyLock<Mutex<HashMap<String, Known>>> = LazyLock::new(Default::default);

/// Home: the releases of the `github_release` tools of a lease, by tool ID. A failed lookup keeps the last
/// known release of the repository; a tool without one is left out (the runner resolves it itself).
pub async fn releases(app: &App, tools: &[Tool]) -> BTreeMap<String, Release> {
    let mut out = BTreeMap::new();
    for t in tools {
        if let Some(i) = t.install.as_ref().filter(|i| i.kind == InstallKind::GithubRelease)
            && let Some(r) = home_release(app, &i.source).await
        {
            out.insert(t.id.clone(), r);
        }
    }
    out
}

async fn home_release(app: &App, repo: &str) -> Option<Release> {
    let now = now_ms();
    let last = {
        let known = RELEASES.lock().unwrap();
        match known.get(repo) {
            Some(k) if now - k.checked < if k.release.is_some() { RELEASE_TTL_MS } else { RELEASE_RETRY_MS } => return k.release.clone(),
            Some(k) => k.release.clone(),
            None => None,
        }
    };
    let fresh = match tokio::time::timeout(Duration::from_secs(15), app.github.get(&format!("/repos/{repo}/releases/latest"))).await {
        Ok(Ok(v)) => parse_release(&v).with_context(|| format!("GitHub's latest release of {repo} has no tag")),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("GitHub did not answer in time")),
    };
    remember(repo, fresh, last, now)
}

/// Records a lookup: a failed one keeps the last known release.
fn remember(repo: &str, fresh: Result<Release>, last: Option<Release>, now: i64) -> Option<Release> {
    let release = match fresh {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("tool release of {repo}: {e:#}; runners get the last known one");
            last
        }
    };
    RELEASES.lock().unwrap().insert(repo.to_string(), Known { release: release.clone(), checked: now });
    release
}

fn parse_release(v: &serde_json::Value) -> Option<Release> {
    let tag = v["tag_name"].as_str().filter(|t| valid_tag(t))?.to_string();
    let assets = v["assets"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str().map(str::to_string)).collect();
    Some(Release { tag, assets })
}

fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.len() <= 200 && t != "." && t != ".." && !t.chars().any(|c| c.is_whitespace() || c.is_control() || c == '/')
}

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

static LOCKS: LazyLock<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

/// Installs a tool (if needed) and returns its `bin` folder. `release` is Home's release of a `github_release` tool.
pub async fn ensure(app: &App, tool: &Tool, release: Option<&Release>, cache: &Path) -> Result<PathBuf> {
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
        InstallKind::GithubRelease => github_release(&install.source, &expand(&install.asset), release, &dir, &tool.id).await,
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

/// Downloads the release asset matching `pattern`: Home's release first (no API call), else the newest tag of
/// github.com's redirect, else the API (never on Burst). Each lookup and Home's download is tried twice.
async fn github_release(repo: &str, pattern: &str, home: Option<&Release>, dir: &Path, id: &str) -> Result<()> {
    let mut errors = vec![];
    if let Some(r) = home {
        match from_home(repo, pattern, r) {
            Ok(url) => match twice(|| download(url.as_str(), dir, id)).await {
                Ok(()) => return Ok(()),
                Err(e) => errors.push(format!("release {} from Home: {e:#}", r.tag)),
            },
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    match twice(|| latest_tag(repo)).await {
        Ok(tag) => {
            let mut last = None;
            for url in from_tag(repo, pattern, &tag, home)? {
                match download(url.as_str(), dir, id).await {
                    Ok(()) => return Ok(()),
                    Err(e) => last = Some(e),
                }
            }
            if let Some(e) = last {
                errors.push(format!("release {tag}: {e:#}"));
            }
        }
        Err(e) => errors.push(format!("newest release: {e:#}")),
    }
    // A Burst job shares its address with other HF Jobs: GitHub's API would count their requests too.
    if crate::home_link::burst().is_none() {
        match twice(|| github_asset(repo, pattern)).await {
            Ok(url) => match download(&url, dir, id).await {
                Ok(()) => return Ok(()),
                Err(e) => errors.push(format!("{url}: {e:#}")),
            },
            Err(e) => errors.push(format!("GitHub's API: {e:#}")),
        }
    }
    bail!("{}", errors.join("; "))
}

/// The download address of Home's release for this runner's asset pattern.
fn from_home(repo: &str, pattern: &str, r: &Release) -> Result<reqwest::Url> {
    let asset = r.assets.iter().find(|a| glob(pattern, a)).with_context(|| format!("release {} from Home has no asset matching {pattern}", r.tag))?;
    release_url(repo, &r.tag, asset)
}

/// The download addresses of a tag read from github.com's redirect, its asset names guessed from the pattern;
/// none when it is Home's release, already tried.
fn from_tag(repo: &str, pattern: &str, tag: &str, home: Option<&Release>) -> Result<Vec<reqwest::Url>> {
    if home.is_some_and(|r| r.tag == tag) {
        return Ok(vec![]);
    }
    asset_names(pattern, tag).iter().map(|name| release_url(repo, tag, name)).collect()
}

async fn twice<T, F: Future<Output = Result<T>>>(f: impl Fn() -> F) -> Result<T> {
    match f().await {
        Ok(v) => Ok(v),
        Err(e) if e.downcast_ref::<RateLimited>().is_some() => Err(e),
        Err(_) => {
            tokio::time::sleep(Duration::from_secs(2)).await;
            f().await
        }
    }
}

/// `https://github.com/<repo>/releases/download/<tag>/<asset>`: served by github.com, outside the API's limit.
fn release_url(repo: &str, tag: &str, asset: &str) -> Result<reqwest::Url> {
    anyhow::ensure!(valid_tag(tag) && valid_tag(asset), "invalid release {tag:?} or asset {asset:?}");
    let (owner, name) = repo.split_once('/').context("the source is not <owner>/<repo>")?;
    let mut url = reqwest::Url::parse("https://github.com/")?;
    url.path_segments_mut().map_err(|()| anyhow::anyhow!("invalid URL"))?.extend([owner, name, "releases", "download", tag, asset]);
    Ok(url)
}

/// The newest tag of `repo`, from the redirect of `https://github.com/<repo>/releases/latest` (not the API).
async fn latest_tag(repo: &str) -> Result<String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).user_agent("launchpad").redirect(reqwest::redirect::Policy::none()).build()?;
    let url = format!("https://github.com/{repo}/releases/latest");
    let res = client.get(&url).send().await?;
    if let Some(m) = rate_limit(res.status(), res.headers(), now_ms() / 1000) {
        return Err(anyhow::Error::new(RateLimited).context(format!("{m} for {url}")));
    }
    let location = res.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).with_context(|| format!("{url} answered {} without a redirect", res.status()))?;
    tag_from_location(location).with_context(|| format!("{url} redirects to {location}, not to a release"))
}

/// The tag of a release page address (`…/releases/tag/<tag>`).
fn tag_from_location(location: &str) -> Option<String> {
    let url = reqwest::Url::parse(location).or_else(|_| reqwest::Url::parse("https://github.com/")?.join(location)).ok()?;
    let segments: Vec<&str> = url.path_segments()?.collect();
    let at = segments.windows(2).position(|w| w == ["releases", "tag"])?;
    let raw = segments.get(at + 2)?;
    let tag = percent_decode(raw)?;
    valid_tag(&tag).then_some(tag)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Asset names a pattern gives for a tag, without the release's asset list: `*` is the version, with or without
/// the tag's leading `v` (`gh_*_linux_amd64.tar.gz` and `v2.83.0` give `gh_2.83.0_linux_amd64.tar.gz` first).
fn asset_names(pattern: &str, tag: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for version in [tag.strip_prefix('v').unwrap_or(tag), tag] {
        let name = pattern.replace('*', version);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// GitHub refused a request for its rate limit: trying again at once does not help.
#[derive(Debug)]
struct RateLimited;

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rate limited")
    }
}

impl std::error::Error for RateLimited {}

/// "GitHub rate limit (…)" when GitHub refused a request for its rate limit: a 429, or a 403 with
/// `x-ratelimit-remaining: 0` or `retry-after`.
fn rate_limit(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap, now_s: i64) -> Option<String> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let retry = header("retry-after").and_then(|v| v.parse::<i64>().ok());
    let spent = header("x-ratelimit-remaining") == Some("0");
    let limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS || (status == reqwest::StatusCode::FORBIDDEN && (spent || retry.is_some()));
    if !limited {
        return None;
    }
    let wait = retry.or_else(|| header("x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok()).filter(|_| spent).map(|r| r - now_s)).filter(|w| *w > 0);
    Some(match wait {
        Some(w) => format!("GitHub rate limit (HTTP {}, retry in {} min)", status.as_u16(), (w + 59) / 60),
        None => format!("GitHub rate limit (HTTP {})", status.as_u16()),
    })
}

/// A GET that fails with "GitHub rate limit" instead of a bare 403 or 429.
async fn get_ok(client: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    let res = client.get(url).send().await?;
    if let Some(m) = rate_limit(res.status(), res.headers(), now_ms() / 1000) {
        return Err(anyhow::Error::new(RateLimited).context(format!("{m} for {url}")));
    }
    Ok(res.error_for_status()?)
}

/// The download URL of the newest release asset matching `pattern`, from GitHub's API (unauthenticated).
async fn github_asset(repo: &str, pattern: &str) -> Result<String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).user_agent("launchpad").build()?;
    let v: serde_json::Value = get_ok(&client, &format!("https://api.github.com/repos/{repo}/releases/latest")).await?.json().await?;
    v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| glob(pattern, n)))
        .and_then(|a| a["browser_download_url"].as_str().map(str::to_string))
        .with_context(|| format!("no asset of {repo} matches {pattern}"))
}

async fn download(url: &str, dir: &Path, id: &str) -> Result<()> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(300)).user_agent("launchpad").build()?;
    let bytes = get_ok(&client, url).await?.bytes().await?;
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
    // On a Local VM or Burst the machine is the boundary: no bubblewrap there.
    let jail = crate::jail::Jail { rw: vec![dir.to_path_buf(), cache.to_path_buf()], ro, tmp: tmp.clone(), env, bwrap: !crate::contain::outside_jail(app), capture: None, ca_bundle: None };
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

/// A tool's settings (`Tool::vars`) apply only to its own commands: for each program of `bin`, a script in `out`
/// (put on PATH instead of `bin`) sets them and runs the program. The session's environment never holds them.
pub fn wrappers(tool: &Tool, bin: &Path, out: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(out)?;
    for e in std::fs::read_dir(bin)?.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        // Written aside, then renamed: a running session may be reading the previous script.
        let tmp = out.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, wrapper_script(tool, &e.path()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, out.join(name))?;
    }
    Ok(())
}

fn wrapper_script(tool: &Tool, program: &Path) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut text = format!("#!/bin/sh\n# Settings of the tool {} (Launchpad).\n", tool.id);
    for (k, v) in &tool.vars {
        text.push_str(&format!("export {k}={}\n", quote(v)));
    }
    text.push_str(&format!("exec {} \"$@\"\n", quote(&program.to_string_lossy())));
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn wrappers_set_quoted_settings_and_run_the_program() {
        let mut t = crate::vault::templates().into_iter().find(|t| t.id == "gh").unwrap();
        t.vars.insert("GH_PAGER".into(), "it's $HOME".into());
        t.vars.insert("GH_PROMPT_DISABLED".into(), "1".into());
        let s = super::wrapper_script(&t, std::path::Path::new("/data/tools/gh-1/bin/gh"));
        assert_eq!(s, "#!/bin/sh\n# Settings of the tool gh (Launchpad).\nexport GH_PAGER='it'\\''s $HOME'\nexport GH_PROMPT_DISABLED='1'\nexec '/data/tools/gh-1/bin/gh' \"$@\"\n");
    }

    fn release(tag: &str) -> super::Release {
        super::Release { tag: tag.into(), assets: vec![format!("gh_{}_linux_amd64.tar.gz", tag.trim_start_matches('v')), format!("gh_{}_linux_arm64.tar.gz", tag.trim_start_matches('v'))] }
    }

    #[test]
    fn home_keeps_the_last_known_release_when_a_lookup_fails() {
        let repo = "test-owner/remember";
        assert_eq!(super::remember(repo, Ok(release("v2.83.0")), None, 1), Some(release("v2.83.0")));
        // GitHub fails: the runner still gets the release Home knew.
        assert_eq!(super::remember(repo, Err(anyhow::anyhow!("403")), Some(release("v2.83.0")), 2), Some(release("v2.83.0")));
        assert_eq!(super::remember(repo, Err(anyhow::anyhow!("403")), None, 3), None);
    }

    #[test]
    fn runners_download_from_github_com_not_the_api() {
        let url = super::from_home("cli/cli", "gh_*_linux_amd64.tar.gz", &release("v2.83.0")).unwrap();
        assert_eq!(url.as_str(), "https://github.com/cli/cli/releases/download/v2.83.0/gh_2.83.0_linux_amd64.tar.gz");
        assert!(super::from_home("cli/cli", "gh_*_linux_riscv.tar.gz", &release("v2.83.0")).is_err());
    }

    #[test]
    fn without_home_the_redirect_tag_gives_the_downloads() {
        // Home's release is missing (or failed): the tag of github.com's redirect, asset names from the pattern.
        let tag = super::tag_from_location("https://github.com/cli/cli/releases/tag/v2.84.0").unwrap();
        assert_eq!(tag, "v2.84.0");
        let urls: Vec<String> = super::from_tag("cli/cli", "gh_*_linux_amd64.tar.gz", &tag, None).unwrap().iter().map(|u| u.to_string()).collect();
        assert_eq!(urls, ["https://github.com/cli/cli/releases/download/v2.84.0/gh_2.84.0_linux_amd64.tar.gz", "https://github.com/cli/cli/releases/download/v2.84.0/gh_v2.84.0_linux_amd64.tar.gz"]);
        // Home's release failed to download and the redirect names the same tag: nothing new to try.
        assert!(super::from_tag("cli/cli", "gh_*_linux_amd64.tar.gz", "v2.84.0", Some(&release("v2.84.0"))).unwrap().is_empty());
        assert_eq!(super::tag_from_location("/co42/slackline/releases/tag/v0.4.1").as_deref(), Some("v0.4.1"));
        // A tag that would leave its path segment is refused.
        assert_eq!(super::tag_from_location("/co42/slackline/releases/tag/a%2F..").as_deref(), None);
        assert_eq!(super::tag_from_location("https://github.com/cli/cli/releases"), None);
        assert_eq!(super::asset_names("tool", "v1"), ["tool"]);
    }

    #[test]
    fn rate_limits_say_so() {
        use reqwest::StatusCode;
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        h.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1600"));
        assert_eq!(super::rate_limit(StatusCode::FORBIDDEN, &h, 1000).as_deref(), Some("GitHub rate limit (HTTP 403, retry in 10 min)"));
        assert_eq!(super::rate_limit(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), 0).as_deref(), Some("GitHub rate limit (HTTP 429)"));
        let mut r = HeaderMap::new();
        r.insert("retry-after", HeaderValue::from_static("30"));
        assert_eq!(super::rate_limit(StatusCode::FORBIDDEN, &r, 0).as_deref(), Some("GitHub rate limit (HTTP 403, retry in 1 min)"));
        // A 403 for another reason stays a 403.
        assert_eq!(super::rate_limit(StatusCode::FORBIDDEN, &HeaderMap::new(), 0), None);
        let mut ok = HeaderMap::new();
        ok.insert("x-ratelimit-remaining", HeaderValue::from_static("12"));
        assert_eq!(super::rate_limit(StatusCode::FORBIDDEN, &ok, 0), None);
    }

    #[test]
    fn home_parses_releases() {
        let v = serde_json::json!({"tag_name": "v0.4.1", "assets": [{"name": "slackline-0.4.1-x86_64-unknown-linux-gnu.tar.gz"}]});
        assert_eq!(super::parse_release(&v), Some(super::Release { tag: "v0.4.1".into(), assets: vec!["slackline-0.4.1-x86_64-unknown-linux-gnu.tar.gz".into()] }));
        assert_eq!(super::parse_release(&serde_json::json!({"tag_name": "../x"})), None);
    }

    #[test]
    fn glob_matches_release_assets() {
        assert!(super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_amd64.tar.gz"));
        assert!(!super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_arm64.tar.gz"));
        assert!(!super::glob("gh_*_linux_amd64.tar.gz", "gh_2.83.0_linux_amd64.tar.gz.sig"));
        assert!(super::glob("tool", "tool"));
    }
}
