//! Session CLI credentials use a different signing domain from Git and never expose the daemon token. An oracle
//! evaluation with Launchpad read access gets an oracle token instead (`oracle_token`): random, in Home's memory only,
//! for one workspace, revoked when the evaluation ends.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::Result;
use hmac::{KeyInit, Mac};
use sha2::Digest;

pub fn token(secret: &str, id: &str, generation: u32) -> Result<String> {
    let scope = format!("lp-session.{id}.{generation}");
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    mac.update(scope.as_bytes());
    Ok(format!("{scope}.{}", hex::encode(mac.finalize().into_bytes())))
}

pub fn verify(secret: &str, value: &str) -> Option<(String, u32)> {
    let mut parts = value.split('.');
    if parts.next()? != "lp-session" { return None; }
    let id = parts.next()?;
    let generation = parts.next()?.parse().ok()?;
    let signature = hex::decode(parts.next()?).ok()?;
    if id.is_empty() || parts.next().is_some() { return None; }
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(format!("lp-session.{id}.{generation}").as_bytes());
    mac.verify_slice(&signature).ok()?;
    Some((id.into(), generation))
}

/// The prefix of an oracle token.
pub const ORACLE_PREFIX: &str = "lp-oracle.";

/// Live oracle tokens by SHA-256: workspace and expiry (Unix ms).
type OracleTokens = HashMap<[u8; 32], (String, i64)>;
static ORACLE_TOKENS: LazyLock<Mutex<OracleTokens>> = LazyLock::new(Default::default);

fn oracle_key(value: &str) -> [u8; 32] {
    sha2::Sha256::digest(value.as_bytes()).into()
}

/// An oracle evaluation's read token for one workspace (`access.launchpad = "read"`), valid until it is dropped or
/// expires. Home's auth accepts it like a read-level session CLI token of that workspace, for GET requests only
/// (`agent_auth::authorize_oracle`).
pub struct OracleToken {
    pub value: String,
    key: [u8; 32],
}

impl Drop for OracleToken {
    fn drop(&mut self) {
        ORACLE_TOKENS.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.key);
    }
}

/// Mints a token for `workspace`, valid `ttl_ms` at most.
pub fn oracle_token(workspace: &str, ttl_ms: i64) -> OracleToken {
    let value = format!("{ORACLE_PREFIX}{}", hex::encode(rand::random::<[u8; 32]>()));
    let key = oracle_key(&value);
    let mut tokens = ORACLE_TOKENS.lock().unwrap_or_else(|p| p.into_inner());
    let now = crate::util::now_ms();
    tokens.retain(|_, (_, until)| *until > now);
    tokens.insert(key, (workspace.to_string(), now + ttl_ms));
    OracleToken { value, key }
}

/// The workspace of a live oracle token.
pub fn oracle_workspace(value: &str) -> Option<String> {
    if !value.starts_with(ORACLE_PREFIX) {
        return None;
    }
    let tokens = ORACLE_TOKENS.lock().unwrap_or_else(|p| p.into_inner());
    tokens.get(&oracle_key(value)).filter(|(_, until)| *until > crate::util::now_ms()).map(|(ws, _)| ws.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oracle_tokens_name_their_workspace_until_dropped() {
        let t = oracle_token("w1", 60_000);
        assert_eq!(oracle_workspace(&t.value).as_deref(), Some("w1"));
        assert_eq!(oracle_workspace(&format!("{}0", t.value)), None);
        let expired = oracle_token("w2", -1);
        assert_eq!(oracle_workspace(&expired.value), None);
        let value = t.value.clone();
        drop(t);
        assert_eq!(oracle_workspace(&value), None);
    }
    #[test]
    fn binds_session_and_generation_and_rejects_other_signatures() {
        let t = token("secret", "session", 3).unwrap();
        assert_eq!(verify("secret", &t), Some(("session".into(), 3)));
        assert!(verify("other", &t).is_none());
        assert!(verify("secret", &t.replace(".3.", ".4.")).is_none());
        assert!(verify("secret", &t.replace(".session.", ".other.")).is_none());
        assert!(verify("secret", &(t + ".extra")).is_none());
    }
}
