//! The SSH key of the vault, used through ssh-agent forwarding: the key never leaves Home.
//!
//! Home runs one `ssh-agent` holding the vault's key in its data folder. A runner gives each session a socket
//! (`contain.rs`) and sends every agent message to Home through its tunnel; Home answers only "list keys" and
//! "sign" requests of an active lease. Adding, removing or locking keys is refused.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::App;

const REQUEST_IDENTITIES: u8 = 11;
const SIGN_REQUEST: u8 = 13;
const FAILURE: [u8; 5] = [0, 0, 0, 1, 5];

/// Message types a session may send.
pub fn allowed(frame: &[u8]) -> bool {
    frame.len() >= 5 && u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize == frame.len() - 4 && matches!(frame[4], REQUEST_IDENTITIES | SIGN_REQUEST)
}

fn socket(app: &App) -> PathBuf {
    app.paths.data_dir.join("vault/ssh-agent.sock")
}

static AGENT: std::sync::LazyLock<tokio::sync::Mutex<Option<tokio::process::Child>>> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Starts Home's agent with the vault key (or stops it when there is no key). Called after vault changes.
pub async fn reload(app: &App) {
    if let Err(e) = start(app).await {
        tracing::warn!("ssh-agent: {e:#}");
    }
}

async fn start(app: &App) -> Result<()> {
    let mut agent = AGENT.lock().await;
    if let Some(mut child) = agent.take() {
        let _ = child.kill().await;
    }
    let sock = socket(app);
    let _ = std::fs::remove_file(&sock);
    let key = app
        .state_db
        .call(|c| {
            match c.query_row("SELECT secret FROM vault WHERE kind = 'ssh_key' ORDER BY updated_at DESC LIMIT 1", [], |r| r.get::<_, String>(0)) {
                Ok(k) => Ok(Some(k)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await?;
    let Some(key) = key else { return Ok(()) };
    let dir = sock.parent().context("socket folder")?.to_path_buf();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let child = tokio::process::Command::new("ssh-agent").arg("-D").arg("-a").arg(&sock).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn().context("start ssh-agent (install openssh-client)")?;
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let file = dir.join(format!("key-{}", uuid::Uuid::new_v4()));
    crate::paths::write_private(&file, &format!("{}\n", key.trim()))?;
    let added = tokio::process::Command::new("ssh-add").arg(&file).env("SSH_AUTH_SOCK", &sock).stdin(std::process::Stdio::null()).output().await;
    let _ = std::fs::remove_file(&file);
    let out = added.context("run ssh-add")?;
    anyhow::ensure!(out.status.success(), "ssh-add: {}", String::from_utf8_lossy(&out.stderr).trim());
    *agent = Some(child);
    Ok(())
}

/// Home: answers one agent message of a session.
pub async fn handle(app: &App, frame: &[u8]) -> Result<Vec<u8>> {
    if !allowed(frame) {
        return Ok(FAILURE.to_vec());
    }
    if AGENT.lock().await.is_none() {
        start(app).await?;
    }
    let mut stream = tokio::net::UnixStream::connect(socket(app)).await.context("Home has no SSH key")?;
    stream.write_all(frame).await?;
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    anyhow::ensure!(n <= 256 * 1024, "ssh-agent reply too large");
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body).await?;
    Ok([len.to_vec(), body].concat())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_listing_and_signing_are_forwarded() {
        assert!(super::allowed(&[0, 0, 0, 1, 11]));
        assert!(super::allowed(&[0, 0, 0, 3, 13, 1, 2]));
        // Add identity, remove all, lock.
        assert!(!super::allowed(&[0, 0, 0, 1, 17]));
        assert!(!super::allowed(&[0, 0, 0, 1, 19]));
        assert!(!super::allowed(&[0, 0, 0, 1, 22]));
        // A length that does not match the frame.
        assert!(!super::allowed(&[0, 0, 0, 9, 11]));
    }
}
