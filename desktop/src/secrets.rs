//! Secrets in the Secret Service keyring (GNOME Keyring), through oo7: the Home device secret and the Hugging Face
//! refresh token (or a pasted token). Nothing is ever written to a plain file; without a keyring the companion
//! refuses to link and says why (decision D-KEYRING).

use std::collections::HashMap;

pub const DEVICE: &str = "device";
pub const HF_REFRESH: &str = "hf-refresh";
pub const HF_TOKEN: &str = "hf-token";

const APP: &str = "dev.launchpad.Desktop";

pub const NO_KEYRING: &str = "No keyring is available. Launchpad keeps its device secret only in the Secret Service keyring: install and unlock GNOME Keyring (package gnome-keyring), then try again.";

fn attributes(kind: &str, origin: &str) -> HashMap<String, String> {
    HashMap::from([("application".to_string(), APP.to_string()), ("kind".to_string(), kind.to_string()), ("home".to_string(), origin.to_string())])
}

async fn keyring() -> anyhow::Result<oo7::Keyring> {
    let keyring = oo7::Keyring::new().await.map_err(|e| {
        tracing::warn!("keyring: {e}");
        anyhow::anyhow!(NO_KEYRING)
    })?;
    // A locked keyring asks the system to unlock it (its own prompt); refused means nothing is stored.
    if keyring.is_locked().await.unwrap_or(false) {
        keyring.unlock().await.map_err(|_| anyhow::anyhow!("The keyring is locked. Unlock it to connect Launchpad."))?;
    }
    Ok(keyring)
}

/// Checks that a keyring answers (before linking).
pub async fn available() -> anyhow::Result<()> {
    keyring().await.map(|_| ())
}

pub async fn store(kind: &str, origin: &str, secret: &str) -> anyhow::Result<()> {
    let label = match kind {
        DEVICE => format!("Launchpad companion · device for {origin}"),
        _ => format!("Launchpad companion · Hugging Face for {origin}"),
    };
    keyring().await?.create_item(&label, &attributes(kind, origin), secret.as_bytes().to_vec(), true).await?;
    Ok(())
}

pub async fn load(kind: &str, origin: &str) -> anyhow::Result<Option<String>> {
    let items = keyring().await?.search_items(&attributes(kind, origin)).await?;
    let Some(item) = items.first() else { return Ok(None) };
    Ok(Some(String::from_utf8(item.secret().await?.as_bytes().to_vec())?))
}

pub async fn delete(kind: &str, origin: &str) -> anyhow::Result<()> {
    keyring().await?.delete(&attributes(kind, origin)).await?;
    Ok(())
}

/// Every secret of the companion for this Home (Sign out).
pub async fn delete_all(origin: &str) -> anyhow::Result<()> {
    for kind in [DEVICE, HF_REFRESH, HF_TOKEN] {
        delete(kind, origin).await?;
    }
    Ok(())
}
