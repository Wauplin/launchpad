//! Updates of the companion: a check of Wauplin/launchpad's GitHub releases (at start, then every 6 hours, with
//! backoff; the latest release, or the `main-build` prerelease on the main channel), and, only on the owner's click,
//! a verified install: the `.deb` is downloaded into a private folder, its SHA-256 must equal both GitHub's `digest`
//! of the asset (`sha256:<hex>`, the release asset schema of the REST API) and the published `.sha256`, `dpkg-deb -f`
//! must name package `launchpad-desktop`, architecture `amd64` and the release's version, and PackageKit installs it
//! (`InstallFiles`; polkit asks for the password, the owner's confirmation), else `pkexec apt-get install`.
//! Nothing is installed without the click; a failure says why and changes nothing.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const REPO: &str = "Wauplin/launchpad";
pub const PACKAGE: &str = "launchpad-desktop";
pub const ARCH: &str = "amd64";
const API: &str = "https://api.github.com";

/// This build's version and channel (`build.rs`).
pub fn version() -> &'static str {
    env!("LAUNCHPAD_DESKTOP_BUILD_VERSION")
}

pub fn channel() -> &'static str {
    env!("LAUNCHPAD_DESKTOP_BUILD_CHANNEL")
}

/// Debian's version order (dpkg's `verrevcmp`) for an upstream version without epoch or revision: digits compare
/// as numbers, `~` sorts before everything (so `2026.11.23.0~main…` comes before `2026.11.23.0` and `.1`), letters
/// before other characters.
pub fn compare(a: &str, b: &str) -> Ordering {
    fn order(c: Option<u8>) -> i32 {
        match c {
            None => 0,
            Some(b'~') => -1,
            Some(c) if c.is_ascii_digit() => 0,
            Some(c) if c.is_ascii_alphabetic() => c as i32,
            Some(c) => c as i32 + 256,
        }
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        // The non-digit parts, character by character.
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let (ca, cb) = (order(a.get(i).copied().filter(|c| !c.is_ascii_digit())), order(b.get(j).copied().filter(|c| !c.is_ascii_digit())));
            if ca != cb {
                return ca.cmp(&cb);
            }
            if i < a.len() && !a[i].is_ascii_digit() {
                i += 1;
            }
            if j < b.len() && !b[j].is_ascii_digit() {
                j += 1;
            }
        }
        // The digit parts, as numbers.
        while i < a.len() && a[i] == b'0' {
            i += 1;
        }
        while j < b.len() && b[j] == b'0' {
            j += 1;
        }
        let (si, sj) = (i, j);
        while i < a.len() && a[i].is_ascii_digit() {
            i += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let (da, db) = (&a[si..i], &b[sj..j]);
        match da.len().cmp(&db.len()).then_with(|| da.cmp(db)) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    /// `sha256:<hex>` (GitHub computes it for every uploaded asset; null for older ones).
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// A newer package: its version, the `.deb` and its `.sha256`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: String,
    pub deb: Asset,
    pub sha256: Asset,
}

/// `launchpad-desktop_<version>_amd64.deb` → its version. GitHub stores an uploaded asset whose name has `~` with
/// `.` instead, so a main build's asset `…_2026.11.23.0.main.165830.g614bccb_amd64.deb` holds the package version
/// `2026.11.23.0~main.165830.g614bccb` (build.yml's format): that one is taken back.
pub fn deb_version(name: &str) -> Option<String> {
    let v = name.strip_prefix("launchpad-desktop_")?.strip_suffix("_amd64.deb")?;
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric() || ".~+".contains(c)) {
        return None;
    }
    let digits = |s: &str, n: Option<usize>| !s.is_empty() && n.is_none_or(|n| s.len() == n) && s.bytes().all(|b| b.is_ascii_digit());
    if let Some((day, build)) = v.split_once(".main.")
        && let [y, m, d, n] = day.split('.').collect::<Vec<_>>()[..]
        && digits(y, Some(4))
        && digits(m, Some(2))
        && digits(d, Some(2))
        && digits(n, None)
        && let Some((time, commit)) = build.split_once(".g")
        && digits(time, Some(6))
        && !commit.is_empty()
        && commit.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Some(format!("{day}~main.{build}"));
    }
    Some(v.to_string())
}

/// The newest package of a release (the main-build prerelease holds several) that has its `.sha256`, when it is
/// newer than `current`.
pub fn select(release: &Release, current: &str) -> Option<Candidate> {
    release
        .assets
        .iter()
        .filter_map(|a| {
            let version = deb_version(&a.name)?;
            let sha = release.assets.iter().find(|s| s.name == format!("{}.sha256", a.name))?;
            Some(Candidate { version, deb: a.clone(), sha256: sha.clone() })
        })
        .max_by(|x, y| compare(&x.version, &y.version))
        .filter(|c| compare(&c.version, current) == Ordering::Greater)
}

fn hex64(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

/// GitHub's `digest` of an asset (`sha256:<hex>`).
pub fn parse_digest(digest: Option<&str>) -> Option<String> {
    hex64(digest?.strip_prefix("sha256:")?)
}

/// A `sha256sum` line (`<hex>  <name>`, or `<hex> *<name>`); the name must be the package's when given.
pub fn parse_sha256_file(text: &str, name: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let mut parts = line.split_whitespace();
    let hex = hex64(parts.next()?)?;
    match parts.next().map(|n| n.trim_start_matches('*')) {
        Some(n) if n != name => None,
        _ => Some(hex),
    }
}

/// The `.sha256` of a candidate: it names the asset, or the file build-deb.sh wrote (with the version's `~`).
fn sha256_of(text: &str, c: &Candidate) -> Option<String> {
    parse_sha256_file(text, &c.deb.name).or_else(|| parse_sha256_file(text, &format!("{PACKAGE}_{}_{ARCH}.deb", c.version)))
}

/// The package fields `dpkg-deb -f <file> Package Architecture Version` printed.
pub fn check_fields(output: &str, version: &str) -> Result<(), String> {
    let field = |k: &str| output.lines().find_map(|l| l.strip_prefix(&format!("{k}: ")).map(str::trim));
    for (k, want) in [("Package", PACKAGE), ("Architecture", ARCH), ("Version", version)] {
        match field(k) {
            Some(v) if v == want => {}
            got => return Err(format!("the package's {k} is {:?}, expected {want}", got.unwrap_or("missing"))),
        }
    }
    Ok(())
}

/// The checks of a downloaded package, before anything is installed: both checksums present and equal to the file's.
pub fn verify_sums(actual: &str, digest: Option<&str>, sha_file: Option<&str>) -> Result<(), String> {
    let digest = digest.ok_or("GitHub gives no digest for this asset")?;
    let sha_file = sha_file.ok_or("the release has no usable .sha256 for this package")?;
    if digest != actual {
        return Err("the download does not match GitHub's digest".into());
    }
    if sha_file != actual {
        return Err("the download does not match the published .sha256".into());
    }
    Ok(())
}

/// Where an update moves along (folio page States, "Update states").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Available(Candidate),
    Downloading(String),
    Verifying(String),
    Installing(String),
    Failed { candidate: Candidate, reason: String },
    /// Installed: the companion restarts into it.
    Restart(String),
}

impl State {
    /// The menu line, and whether a click installs (or retries).
    pub fn menu(&self) -> (String, bool) {
        match self {
            State::Available(c) => (format!("Update to {}", c.version), true),
            State::Downloading(v) => (format!("Downloading {v}…"), false),
            State::Verifying(v) => (format!("Verifying {v}…"), false),
            State::Installing(v) => (format!("Installing {v}… (confirm in the password dialog)"), false),
            State::Failed { candidate, reason } => (format!("Update to {} failed: {} · Retry", candidate.version, crate::model::cut(reason, 60)), true),
            State::Restart(v) => (format!("Restarting into {v}…"), false),
        }
    }
}

/// The GitHub API base: always api.github.com, except a debug build's `LAUNCHPAD_DESKTOP_UPDATE_API` (a local fake
/// release server for tests).
pub fn api_base() -> String {
    if cfg!(debug_assertions)
        && let Ok(base) = std::env::var("LAUNCHPAD_DESKTOP_UPDATE_API")
        && !base.is_empty()
    {
        return base.trim_end_matches('/').to_string();
    }
    API.into()
}

/// The client for GitHub: HTTPS only (redirects too, to GitHub's download hosts), never Home's device header.
pub fn client() -> reqwest::Client {
    let local = cfg!(debug_assertions) && api_base() != API;
    reqwest::Client::builder()
        .user_agent(format!("launchpad-desktop/{}", version()))
        .https_only(!local)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("HTTP client")
}

/// The newest package of the channel when it is newer than this build (None: up to date).
pub async fn check(http: &reqwest::Client, channel: &str) -> anyhow::Result<Option<Candidate>> {
    let path = if channel == "main" { "releases/tags/main-build" } else { "releases/latest" };
    let response = http.get(format!("{}/repos/{REPO}/{path}", api_base())).header("Accept", "application/vnd.github+json").send().await?;
    anyhow::ensure!(response.status().is_success(), "GitHub answered {}", response.status());
    let release: Release = response.json().await?;
    Ok(select(&release, version()))
}

/// A private folder for the download (0700) under the user's cache, where PackageKit (root) can read it.
fn private_dir() -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).ok_or_else(|| anyhow::anyhow!("no cache folder"))?;
    let dir = base.join(crate::settings::DIR).join(format!("update-{}", crate::oauth::random(8)));
    std::fs::create_dir_all(dir.parent().expect("parent"))?;
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// Where release assets come from: this repository's release downloads (GitHub then redirects to its asset host),
/// except a debug build's fake release server.
fn asset_url_ok(url: &str) -> bool {
    url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) || (cfg!(debug_assertions) && api_base() != API)
}

/// Downloads and verifies a package; answers its path in a private folder. Every check must pass; on any failure
/// the folder is removed.
pub async fn download(http: &reqwest::Client, c: &Candidate, on_verify: impl FnOnce()) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(asset_url_ok(&c.deb.browser_download_url) && asset_url_ok(&c.sha256.browser_download_url), "the release names a download outside {REPO}'s releases");
    let dir = private_dir()?;
    let result = fetch_and_verify(http, c, &dir, on_verify).await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

async fn fetch_and_verify(http: &reqwest::Client, c: &Candidate, dir: &Path, on_verify: impl FnOnce()) -> anyhow::Result<PathBuf> {
    use futures_util::StreamExt;
    use std::io::Write;
    let path = dir.join(&c.deb.name);
    let response = http.get(&c.deb.browser_download_url).send().await?;
    anyhow::ensure!(response.status().is_success(), "the download answered {}", response.status());
    let mut file = std::fs::File::create(&path)?;
    let mut hasher = Sha256::new();
    let mut body = response.bytes_stream();
    let mut size = 0usize;
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        size += chunk.len();
        anyhow::ensure!(size <= 200 * 1024 * 1024, "the package is larger than 200 MB");
        hasher.update(&chunk);
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    on_verify();
    let actual = hex::encode(hasher.finalize());
    let sha_text = http.get(&c.sha256.browser_download_url).send().await?.error_for_status()?.text().await?;
    let result = verify_sums(&actual, parse_digest(c.deb.digest.as_deref()).as_deref(), sha256_of(&sha_text, c).as_deref())
        .and_then(|()| fields(&path).and_then(|out| check_fields(&out, &c.version)));
    if let Err(reason) = result {
        anyhow::bail!(reason);
    }
    Ok(path)
}

fn fields(path: &Path) -> Result<String, String> {
    let out = std::process::Command::new("dpkg-deb").arg("-f").arg(path).args(["Package", "Architecture", "Version"]).output().map_err(|e| format!("dpkg-deb: {e}"))?;
    if !out.status.success() {
        return Err(format!("dpkg-deb refused the file: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Installs a verified package: PackageKit's `InstallFiles` (transaction flags none, so polkit asks for the
/// administrator password of an unsigned local file), else `pkexec apt-get install`. The folder is removed after.
pub async fn install(path: &Path) -> anyhow::Result<()> {
    let result = match packagekit(path).await {
        Err(e) if e.to_string().contains("ServiceUnknown") || e.to_string().contains("not provided") => {
            tracing::info!("PackageKit is not available ({e}): trying pkexec");
            pkexec(path).await
        }
        other => other,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
}

async fn packagekit(path: &Path) -> anyhow::Result<()> {
    use ashpd::zbus;
    use futures_util::StreamExt;
    let conn = zbus::Connection::system().await?;
    let pk = zbus::Proxy::new(&conn, "org.freedesktop.PackageKit", "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit").await?;
    let tx: zbus::OwnedObjectPath = pk.call("CreateTransaction", &()).await?;
    let t = zbus::Proxy::new(&conn, "org.freedesktop.PackageKit", tx.as_str().to_string(), "org.freedesktop.PackageKit.Transaction").await?;
    // The owner is at the screen: PackageKit may ask (polkit's password dialog).
    let _: () = t.call("SetHints", &(vec!["interactive=true".to_string()],)).await?;
    let mut finished = t.receive_signal("Finished").await?;
    let mut errors = t.receive_signal("ErrorCode").await?;
    let file = path.to_str().ok_or_else(|| anyhow::anyhow!("the package path is not UTF-8"))?.to_string();
    // Transaction flags 0 (PK_TRANSACTION_FLAG_ENUM_NONE): the file is unsigned, its checks were made above.
    let _: () = t.call("InstallFiles", &(0u64, vec![file])).await?;
    let mut error: Option<String> = None;
    loop {
        tokio::select! {
            m = errors.next() => if let Some(m) = m && let Ok((code, details)) = m.body().deserialize::<(u32, String)>() {
                error = Some(format!("{details} (PackageKit error {code})"));
            },
            m = finished.next() => {
                let Some(m) = m else { anyhow::bail!("PackageKit ended the transaction") };
                let (exit, _runtime): (u32, u32) = m.body().deserialize()?;
                // PK_EXIT_ENUM_SUCCESS is 1.
                if exit == 1 {
                    return Ok(());
                }
                anyhow::bail!(error.unwrap_or_else(|| format!("PackageKit did not install it (exit {exit})")));
            }
        }
    }
}

async fn pkexec(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(Path::new("/usr/bin/pkexec").exists(), "neither PackageKit nor pkexec is available: install the package with: sudo apt install {}", path.display());
    let status = tokio::process::Command::new("/usr/bin/pkexec").args(["/usr/bin/apt-get", "install", "-y"]).arg(path).status().await?;
    anyhow::ensure!(status.success(), "apt-get did not install it ({status})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debian_version_order() {
        use Ordering::*;
        for (a, b, o) in [
            ("2026.11.23.2", "2026.11.23.10", Less),
            ("2026.11.23.1", "2026.11.23.1", Equal),
            ("2026.11.23.0~main.101500.gabc1234", "2026.11.23.1", Less),
            ("2026.11.23.0~main.101500.gabc1234", "2026.11.23.0", Less),
            ("2026.11.24.0~main.000100.g1234567", "2026.11.23.3", Greater),
            ("2026.11.23.0~main.101500.gabc1234", "2026.11.23.0~main.093000.gfff0000", Greater),
            ("0.1.0", "2026.11.23.1", Less),
            ("1.0a", "1.0", Greater),
            ("1.0~rc1", "1.0", Less),
        ] {
            assert_eq!(compare(a, b), o, "{a} vs {b}");
        }
    }

    fn asset(name: &str) -> Asset {
        Asset { name: name.into(), browser_download_url: format!("https://github.com/x/{name}"), digest: Some(format!("sha256:{}", "a".repeat(64))) }
    }

    #[test]
    fn picks_the_newest_package_with_its_checksum() {
        let release = Release {
            tag_name: "main-build".into(),
            assets: vec![
                asset("launchpad-linux-x86_64-abc.tar.gz"),
                asset("launchpad-desktop_2026.11.20.0~main.1.gaaa_amd64.deb"),
                asset("launchpad-desktop_2026.11.20.0~main.1.gaaa_amd64.deb.sha256"),
                asset("launchpad-desktop_2026.11.22.0~main.1.gbbb_amd64.deb"),
                asset("launchpad-desktop_2026.11.22.0~main.1.gbbb_amd64.deb.sha256"),
                // Newest, but without its .sha256: never chosen.
                asset("launchpad-desktop_2026.11.23.0~main.1.gccc_amd64.deb"),
            ],
        };
        assert_eq!(select(&release, "2026.11.21.1").unwrap().version, "2026.11.22.0~main.1.gbbb");
        assert_eq!(select(&release, "2026.11.22.1"), None);
        assert_eq!(deb_version("launchpad-desktop_1.2_arm64.deb"), None);
        assert_eq!(deb_version("launchpad-desktop_1;rm_amd64.deb"), None);
        assert_eq!(deb_version("launchpad-desktop_2026.11.23.1_amd64.deb").as_deref(), Some("2026.11.23.1"));
        assert_eq!(deb_version("launchpad-desktop_1.main.2_amd64.deb").as_deref(), Some("1.main.2"));
    }

    #[test]
    fn main_build_asset_names_from_github() {
        // The real main-build assets: GitHub turned the version's `~` into `.`, the .sha256 kept build-deb.sh's name.
        let name = "launchpad-desktop_2026.11.23.0.main.165830.g614bccb_amd64.deb";
        let release = Release {
            tag_name: "main-build".into(),
            assets: vec![
                asset(name),
                asset(&format!("{name}.sha256")),
                asset("launchpad-desktop_2026.11.22.0.main.235900.gaaaaaaa_amd64.deb"),
                asset("launchpad-desktop_2026.11.22.0.main.235900.gaaaaaaa_amd64.deb.sha256"),
            ],
        };
        let c = select(&release, "2026.11.22.0~main.235900.gaaaaaaa").unwrap();
        assert_eq!(c.version, "2026.11.23.0~main.165830.g614bccb");
        assert_eq!(c.deb.name, name);
        // The same build is not newer than itself, and that day's release is newer than it.
        assert_eq!(select(&release, "2026.11.23.0~main.165830.g614bccb"), None);
        assert_eq!(select(&release, "2026.11.23.1"), None);
        let hex = "ab".repeat(32);
        assert_eq!(sha256_of(&format!("{hex}  launchpad-desktop_2026.11.23.0~main.165830.g614bccb_amd64.deb\n"), &c), Some(hex.clone()));
        assert_eq!(sha256_of(&format!("{hex}  {name}\n"), &c), Some(hex.clone()));
        assert_eq!(sha256_of(&format!("{hex}  launchpad-desktop_2026.11.22.0~main.235900.gaaaaaaa_amd64.deb\n"), &c), None);
        assert!(check_fields("Package: launchpad-desktop\nArchitecture: amd64\nVersion: 2026.11.23.0~main.165830.g614bccb\n", &c.version).is_ok());
        assert!(asset_url_ok("https://github.com/Wauplin/launchpad/releases/download/main-build/launchpad-desktop_1_amd64.deb"));
        assert!(cfg!(debug_assertions) && std::env::var_os("LAUNCHPAD_DESKTOP_UPDATE_API").is_some() || !asset_url_ok("https://evil.example/launchpad-desktop_1_amd64.deb"));
    }

    #[test]
    fn checksums_and_fields() {
        let hex = "ab".repeat(32);
        assert_eq!(parse_digest(Some(&format!("sha256:{}", hex.to_uppercase()))), Some(hex.clone()));
        assert_eq!(parse_digest(Some("sha512:00")), None);
        assert_eq!(parse_digest(None), None);
        assert_eq!(parse_sha256_file(&format!("{hex}  launchpad-desktop_1_amd64.deb\n"), "launchpad-desktop_1_amd64.deb"), Some(hex.clone()));
        assert_eq!(parse_sha256_file(&format!("{hex} *launchpad-desktop_1_amd64.deb"), "launchpad-desktop_1_amd64.deb"), Some(hex.clone()));
        assert_eq!(parse_sha256_file(&format!("{hex}  other.deb"), "launchpad-desktop_1_amd64.deb"), None);
        assert_eq!(parse_sha256_file("nonsense", "x"), None);
        assert!(verify_sums(&hex, Some(&hex), Some(&hex)).is_ok());
        assert!(verify_sums(&hex, None, Some(&hex)).is_err());
        assert!(verify_sums(&hex, Some(&hex), None).is_err());
        assert!(verify_sums(&hex, Some(&"cd".repeat(32)), Some(&hex)).is_err());
        assert!(verify_sums(&hex, Some(&hex), Some(&"cd".repeat(32))).is_err());
        let out = "Package: launchpad-desktop\nArchitecture: amd64\nVersion: 2026.11.23.1\n";
        assert!(check_fields(out, "2026.11.23.1").is_ok());
        assert!(check_fields(out, "2026.11.23.2").is_err());
        assert!(check_fields(&out.replace("amd64", "arm64"), "2026.11.23.1").is_err());
        assert!(check_fields("Package: evil\n", "2026.11.23.1").is_err());
    }
}
