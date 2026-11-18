//! A folio's small key and value store (`folio_data`): what typed blocks do not cover, such as a ranking in a sketch,
//! a form or a "read" flag. JSON values only, last write wins, with fixed limits: 64 KB per value, 1 MB and 2,000 keys
//! per folio. The owner's window writes it through a sketch's data bridge (only the sketch's own key prefix, checked
//! by the reader); agents read it at every access level and write it at propose access as their own writes
//! (`launchpad folio data get|set|list|rm`). A shared snapshot has no data API.
use anyhow::{Context, Result, ensure};
use launchpad_core::App;
use launchpad_core::kb::Author;
use launchpad_core::util::now_ms;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const VALUE_MAX: usize = 64 * 1024;
pub const TOTAL_MAX: usize = 1 << 20;
pub const KEYS_MAX: usize = 2000;
const KEY_MAX: usize = 200;

/// One entry of a folio's data.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioDataEntry")]
pub struct Entry {
    pub key: String,
    #[ts(type = "unknown")]
    pub value: Value,
    pub size: u32,
    pub author: Author,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// A key: 1 to 200 characters of letters, digits and `_ . : / -`.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= KEY_MAX && key.chars().all(|c| c.is_ascii_alphanumeric() || "_.:/-".contains(c))
}

/// Pure: whether a write of `size` bytes to `key` keeps the folio within its limits, given its current entries
/// (key, size). Returns why not.
pub fn over_limit(entries: &[(String, usize)], key: &str, size: usize) -> Option<String> {
    if size > VALUE_MAX {
        return Some(format!("the value is {} KB: at most 64 KB per key", size.div_ceil(1024)));
    }
    let others: usize = entries.iter().filter(|(k, _)| k != key).map(|(_, s)| s).sum();
    if others + size > TOTAL_MAX {
        return Some("the folio's data would pass 1 MB: remove keys first (launchpad folio data rm)".into());
    }
    let new = !entries.iter().any(|(k, _)| k == key);
    (new && entries.len() >= KEYS_MAX).then(|| "the folio already has 2,000 keys: remove keys first".into())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        key: r.get(0)?,
        value: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or(Value::Null),
        size: r.get(2)?,
        author: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_else(|_| Author::owner(None)),
        updated_at: r.get(4)?,
    })
}

/// The entries whose key starts with `prefix`, sorted by key.
pub async fn list(app: &App, folio: &str, prefix: &str) -> Result<Vec<Entry>> {
    let (f, p) = (folio.to_string(), prefix.to_string());
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT key, value, size, author, updated_at FROM folio_data WHERE folio_id = ?1 AND substr(key, 1, length(?2)) = ?2 ORDER BY key")?;
        let rows = st.query_map(params![f, p], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

pub async fn get(app: &App, folio: &str, key: &str) -> Result<Option<Entry>> {
    let (f, k) = (folio.to_string(), key.to_string());
    app.state_db.call(move |c| Ok(c.query_row("SELECT key, value, size, author, updated_at FROM folio_data WHERE folio_id = ?1 AND key = ?2", params![f, k], row).optional()?)).await
}

/// One writer at a time, so two writes never pass the limits together.
static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sets a key (last write wins), within the limits.
pub async fn set(app: &App, folio: &str, key: &str, value: Value, author: Author) -> Result<Entry> {
    ensure!(valid_key(key), "a data key is 1 to 200 letters, digits and _ . : / -");
    let text = serde_json::to_string(&value)?;
    let _one = WRITES.lock().await;
    let f = folio.to_string();
    let sizes: Vec<(String, usize)> = app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT key, size FROM folio_data WHERE folio_id = ?1")?;
        let rows = st.query_map([f], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    if let Some(why) = over_limit(&sizes, key, text.len()) {
        anyhow::bail!(why);
    }
    let (f, k, a, at, n) = (folio.to_string(), key.to_string(), serde_json::to_string(&author)?, now_ms(), text.len() as i64);
    app.state_db.call(move |c| {
        c.execute("INSERT INTO folio_data (folio_id, key, value, size, author, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (folio_id, key) DO UPDATE SET value = excluded.value, size = excluded.size, author = excluded.author, updated_at = excluded.updated_at", params![f, k, text, n, a, at])?;
        Ok(())
    }).await?;
    get(app, folio, key).await?.context("the value was not saved")
}

pub async fn remove(app: &App, folio: &str, key: &str) -> Result<bool> {
    let (f, k) = (folio.to_string(), key.to_string());
    Ok(app.state_db.call(move |c| Ok(c.execute("DELETE FROM folio_data WHERE folio_id = ?1 AND key = ?2", params![f, k])?)).await? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_limits() {
        assert!(valid_key("poll:ranking") && valid_key("inbox/read.1") && !valid_key("") && !valid_key("a b") && !valid_key(&"k".repeat(201)));
        assert!(over_limit(&[], "a", VALUE_MAX).is_none());
        assert!(over_limit(&[], "a", VALUE_MAX + 1).unwrap().contains("64 KB"));
        let full: Vec<(String, usize)> = (0..16).map(|i| (format!("k{i}"), VALUE_MAX)).collect();
        assert!(over_limit(&full, "k0", VALUE_MAX).is_none(), "replacing a value counts only the new size");
        assert!(over_limit(&full, "new", 10).unwrap().contains("1 MB"));
        let many: Vec<(String, usize)> = (0..KEYS_MAX).map(|i| (format!("k{i}"), 1)).collect();
        assert!(over_limit(&many, "k1", 1).is_none() && over_limit(&many, "other", 1).unwrap().contains("2,000"));
    }
}
