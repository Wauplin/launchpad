//! The daemon ↔ host helper protocol: one JSON object per line over the `ssh … host-helper bridge` pipe.
//!
//! Process output and input are byte streams with absolute offsets. The helper keeps unacknowledged output
//! and the daemon keeps unacknowledged input, so both resume exactly where they stopped after a reconnect.

use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes, base64-encoded on the wire.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct B64(pub Vec<u8>);

impl Serialize for B64 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for B64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(s).map(B64).map_err(serde::de::Error::custom)
    }
}

/// A process the daemon still follows, and the output it already has.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Known {
    pub key: String,
    pub out: u64,
    pub err: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToHelper {
    /// First message. Processes of another daemon boot are stopped: restarting the daemon ends its turns.
    Hello { boot: String, host: String, env: String, known: Vec<Known> },
    Req { id: u64, op: Op },
    /// Input bytes starting at absolute offset `off`; bytes the helper already wrote are skipped.
    Input { key: String, off: u64, data: B64 },
    /// End of input at offset `off` (after every earlier byte).
    CloseInput { key: String, off: u64 },
    /// The daemon has the output up to these offsets: the helper may drop it.
    Ack { key: String, out: u64, err: u64 },
    /// The daemon no longer needs an exited process.
    Release { key: String },
    CliResponse { id: u64, status: u16, body: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcState {
    pub key: String,
    /// Input bytes written to the process so far.
    pub input: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FromHelper {
    /// Answer to `Hello`: the known processes that still exist. The others are gone (host restarted).
    Hello { version: String, procs: Vec<ProcState>, info: HostInfo },
    Res { id: u64, ok: Option<serde_json::Value>, err: Option<ErrorInfo> },
    /// Output of stream `s` (0 stdout or PTY, 1 stderr) starting at offset `off`.
    Out { key: String, s: u8, off: u64, data: B64 },
    /// The process exited, after all of its output.
    Exit { key: String, code: Option<i32>, signal: Option<i32> },
    /// Input written up to this offset.
    InputAck { key: String, off: u64 },
    /// A `launchpad` CLI call from an agent on the host, for the daemon's API.
    Cli {
        id: u64,
        method: String,
        path: String,
        body: Option<String>,
        /// Accepted by the helper while the daemon was away; a failure is reported to the session.
        #[serde(default)]
        queued: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub message: String,
    /// `not_found`, `exists` or `other`, so callers can tell a missing file from a failure.
    pub kind: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct HostInfo {
    /// `uname -s` style name: `Linux`, `Darwin`.
    pub os: String,
    pub arch: String,
    pub home: String,
    /// The account the helper runs as (agents run as it too).
    #[serde(default)]
    pub user: String,
    pub shell: String,
    /// Absolute path of the helper binary (the `launchpad` CLI agents call on the host).
    pub cli: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A short command: the output comes back in the response.
    Exec { argv: Vec<String>, cwd: Option<String>, env: Vec<(String, String)>, stdin: Option<B64> },
    /// A long-running process (harness, PTY shell) owned by the helper. Output streams as `Out`.
    Spawn { key: String, argv: Vec<String>, cwd: Option<String>, env: Vec<(String, String)>, pty: Option<(u16, u16)>, stdin: bool },
    /// Sends a Unix signal to the process (its group with `group`).
    Signal { key: String, signal: i32, group: bool },
    Resize { key: String, rows: u16, cols: u16 },
    Which { programs: Vec<String> },
    Fs(FsOp),
    Ping,
    /// Reads the login-shell environment again (a harness was installed since).
    RefreshEnv,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "fs", rename_all = "snake_case")]
pub enum FsOp {
    Canonicalize { path: String },
    Metadata { path: String, follow: bool },
    Read { path: String, max: u64 },
    /// The last `len` bytes of a file.
    Tail { path: String, len: u64 },
    Write { path: String, data: B64, mode: Option<u32> },
    CreateDirAll { path: String, mode: Option<u32> },
    Remove { path: String, recursive: bool },
    Rename { from: String, to: String },
    ReadDir { path: String },
    Copy { from: String, to: String },
    /// Files of a folder that is not a Git repository (code viewer).
    Walk { root: String, #[serde(default)] directories: bool },
    /// Disk usage of each folder, in bytes.
    DiskUsage { paths: Vec<String> },
    /// Ensures the Launchpad skill is installed for agents on the host.
    InstallSkill,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    /// `file`, `dir`, `symlink` or `other`.
    pub kind: String,
    pub len: u64,
    pub modified_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecResult {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: B64,
    pub stderr: B64,
}

/// First line a bridge sends to the helper server, and its answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum BridgeHello {
    Bridge { version: String },
    Ok,
    /// The running server is another build: the bridge asks it to stop and starts its own.
    Mismatch { version: String },
    Shutdown,
}
