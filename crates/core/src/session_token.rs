//! Session CLI credentials use a different signing domain from Git and never expose the daemon token.
use anyhow::Result;
use hmac::{KeyInit, Mac};

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

#[cfg(test)]
mod tests {
    use super::*;
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
