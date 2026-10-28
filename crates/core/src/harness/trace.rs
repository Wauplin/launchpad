//! Native trace files are owned by adapters; never reconstruct them from normalized UI events.
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use crate::{App, model::Session, shellenv::UserEnv};

pub struct TraceFile {
    /// Relative path within the session's backup folder.
    pub name: String,
    pub data: Vec<u8>,
}

pub fn environment(app: &App, session: &Session) -> Result<UserEnv> {
    let host = crate::hosts::loc::host_of(&session.cwd);
    let overrides = crate::profiles::env(app, session.harness, &session.launch_spec, session.model.as_deref(), host)?;
    Ok(app.env.on(host).with(&overrides))
}

pub async fn root(app: &App, session: &Session, key: &str, default: &str) -> Result<PathBuf> {
    let env = environment(app, session)?;
    let host = env.host();
    let home = match host {
        Some(h) => app.hosts.home(h).context("trace host is disconnected")?,
        None => env.get("HOME").or_else(|| env.get("USERPROFILE")).map(str::to_string)
            .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned())).context("no home directory")?,
    };
    let value = if host.is_some() {
        // Read only the requested variable in the host's actual login/profile environment.
        crate::procs::output(&env, "printenv", &[key], None).await.ok()
    } else { env.get(key).map(str::to_string) };
    let path = value.filter(|v| !v.is_empty()).unwrap_or_else(|| Path::new(&home).join(default).to_string_lossy().into_owned());
    anyhow::ensure!(!path.contains("${"), "unresolved trace directory");
    Ok(PathBuf::from(crate::hosts::loc::join(host, &path)))
}

pub fn conversation(session: &Session) -> Result<&str> {
    let id = session.harness_session_id.as_deref().context("no native conversation yet")?;
    anyhow::ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')), "invalid native conversation ID");
    Ok(id)
}

pub async fn read(path: &Path, name: String) -> Result<TraceFile> {
    let mut data = crate::hostfs::read_limited(path, 256 * 1024 * 1024).await?;
    // A live JSONL writer may be halfway through its last record. Retry that tail next pass.
    if name.ends_with(".jsonl") && !data.ends_with(b"\n") {
        let end = data.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        if serde_json::from_slice::<serde_json::Value>(&data[end..]).is_err() { data.truncate(end); }
    }
    Ok(TraceFile { name, data })
}

pub async fn files(root: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    let mut pending = vec![(root.to_path_buf(), depth)];
    let mut files = vec![];
    while let Some((dir, depth)) = pending.pop() {
        let entries = match crate::hostfs::read_dir(&dir).await {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for e in entries {
            if e.kind == "dir" && depth > 0 { pending.push((dir.join(e.name), depth - 1)); }
            else if e.kind == "file" { files.push(dir.join(e.name)); }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct Temp(pub PathBuf);
    impl Temp {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!("launchpad-trace-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        pub fn write(&self, name: &str, data: &[u8]) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
    }

    #[tokio::test]
    async fn native_trace_preserves_bytes_and_retries_partial_jsonl_tail() {
        let tmp = Temp::new();
        let complete = b"{\"type\":\"assistant\",\"extra\":true}\n";
        let mut partial = complete.to_vec();
        partial.extend(b"{\"type\":");
        tmp.write("trace.jsonl", &partial);
        assert_eq!(read(&tmp.0.join("trace.jsonl"), "trace.jsonl".into()).await.unwrap().data, complete);
        partial.extend(b"\"user\"}");
        tmp.write("trace.jsonl", &partial);
        assert_eq!(read(&tmp.0.join("trace.jsonl"), "trace.jsonl".into()).await.unwrap().data, partial);
        tmp.write("export.json", b"{\"native\":true}");
        assert_eq!(read(&tmp.0.join("export.json"), "export.json".into()).await.unwrap().data, b"{\"native\":true}");
    }
}
