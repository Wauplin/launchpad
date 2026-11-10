//! Network capture of contained sessions, the same on the laptop jail, a Local VM and Burst.
//!
//! Every contained process runs under `launchpad netcap` (the supervisor, in the `launchpad` binary): a seccomp
//! filter hands each internet socket and connect to it, whatever HTTP library or language the tool uses. A TCP
//! connect to a remote address goes to the runner proxy's capture port (`proxy.rs`), which reaches the
//! destination: from the runner's network on a runner (its LAN and tailnet included), from Home's network on
//! Burst (`token_relay::tcp`). On a lease host on port 443, the proxy reads the TLS server name and adds the
//! token as for `HTTPS_PROXY` clients. The runner CA is in the sandbox's system certificate store.
//!
//! One policy, written here: loopback goes to its own address, UDP reaches port 53 (DNS) only, link-local and
//! cloud metadata addresses are refused except for DNS (`refused`), IPv4 and IPv6 are handled the same way, Unix
//! sockets reach paths but never the abstract namespace (`unix_target`: it belongs to the network namespace, which
//! the jail shares with the machine: X11, D-Bus), and Home relays Burst traffic to public addresses only. A machine that cannot run the
//! supervisor (Linux older than 5.9, no seccomp) runs no agents.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;

/// System CA bundles, in the usual Linux locations.
pub const SYSTEM_BUNDLES: &[&str] = &["/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt", "/etc/ssl/ca-bundle.pem", "/etc/ssl/cert.pem"];

pub const DNS_PORT: u16 = 53;

/// UDP may reach DNS only (QUIC clients fall back to TCP).
pub fn udp_allowed(port: u16) -> bool {
    port == DNS_PORT
}

/// Loopback (and the unspecified address, which means loopback) is reached directly: the Launchpad daemon, the
/// runner proxy, harness login URLs and the agent's own servers.
pub fn goes_direct(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    ip.is_loopback() || ip.is_unspecified()
}

/// Link-local addresses (cloud metadata services, instance credentials) and the metadata addresses outside that
/// range: never reached from a session, on any port but DNS (some clouds resolve names on 169.254.169.254). The
/// supervisor refuses them before the proxy, and the runner proxy again on the resolved address (`proxy::connect_out`,
/// names such as `metadata.google.internal`); Burst relays only public addresses (`public`).
pub fn refused(ip: IpAddr, port: u16) -> bool {
    if port == DNS_PORT {
        return false;
    }
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_link_local() || v4.octets() == [100, 100, 100, 200],
        IpAddr::V6(v6) => {
            let s = v6.segments();
            // NAT64 (64:ff9b::/96) carries an IPv4 address.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let o = v6.octets();
                return refused(IpAddr::V4(std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15])), port);
            }
            (s[0] & 0xffc0) == 0xfe80 || s == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
        }
    }
}

/// What a Unix socket address names.
#[derive(Debug, PartialEq)]
pub enum UnixTarget {
    /// No name (`bind` autobinds, `connect` fails).
    Unnamed,
    /// The abstract namespace (`sun_path[0] == 0`): refused.
    Abstract,
    /// A path in the file system (the bytes before the first NUL).
    Path(Vec<u8>),
}

/// The target of a `sockaddr_un` (None: another family or too short).
pub fn unix_target(addr: &[u8]) -> Option<UnixTarget> {
    if addr.len() < 2 || u16::from_ne_bytes([addr[0], addr[1]]) as i32 != libc::AF_UNIX {
        return None;
    }
    let path = &addr[2..];
    Some(match path.first() {
        None => UnixTarget::Unnamed,
        Some(0) => UnixTarget::Abstract,
        Some(_) => UnixTarget::Path(path.iter().take_while(|b| **b != 0).copied().collect()),
    })
}

/// Whether Home relays a Burst connection to this address: public unicast only, never Home's own loopback, its
/// private networks or cloud metadata.
pub fn public(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || s[0] == 0x2001 && s[1] == 0x0db8 || s[0] == 0x0064 && s[1] == 0xff9b)
        }
    }
}

/// The first line the supervisor sends on a captured connection.
pub fn capture_header(key: &str, dest: &SocketAddr) -> String {
    format!("LPCAP1 {key} {} {}\n", dest.ip().to_canonical(), dest.port())
}

pub fn parse_capture_header(line: &str) -> Option<(String, SocketAddr)> {
    let mut parts = line.trim_end().split(' ');
    (parts.next()? == "LPCAP1").then_some(())?;
    let key = parts.next()?.to_string();
    let ip: IpAddr = parts.next()?.parse().ok()?;
    let port: u16 = parts.next()?.parse().ok()?;
    Some((key, SocketAddr::new(ip, port)))
}

/// The proxy's answer once it reached the destination (`OK`) or not (`ERR <errno>`).
pub fn capture_answer(result: std::result::Result<(), i32>) -> String {
    match result {
        Ok(()) => "OK\n".into(),
        Err(e) => format!("ERR {e}\n"),
    }
}

/// 0, or the errno `connect` returns to the agent.
pub fn parse_capture_answer(line: &str) -> i32 {
    match line.trim() {
        "OK" => 0,
        other => other.strip_prefix("ERR ").and_then(|n| n.trim().parse().ok()).unwrap_or(libc::ECONNREFUSED),
    }
}

/// The errno of a failed connect, for the agent.
pub fn errno_of(e: &std::io::Error) -> i32 {
    if let Some(n) = e.raw_os_error() {
        return n;
    }
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused => libc::ECONNREFUSED,
        std::io::ErrorKind::TimedOut => libc::ETIMEDOUT,
        std::io::ErrorKind::PermissionDenied => libc::EACCES,
        std::io::ErrorKind::HostUnreachable => libc::EHOSTUNREACH,
        std::io::ErrorKind::NetworkUnreachable => libc::ENETUNREACH,
        std::io::ErrorKind::AddrNotAvailable => libc::EADDRNOTAVAIL,
        _ => libc::ECONNREFUSED,
    }
}

/// `errno <n>` in an error text from Home (`token_relay::tcp`).
pub fn errno_in(text: &str) -> i32 {
    text.split("errno ").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(libc::ECONNREFUSED)
}

/// How a contained process reaches the supervisor: `launchpad netcap --port <capture port> --key <proxy key> --`.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub exe: PathBuf,
    pub port: u16,
    pub key: String,
}

impl Capture {
    pub fn new(app: &App, key: &str) -> Result<Capture> {
        Ok(Capture { exe: std::env::current_exe().context("the path of the launchpad binary")?, port: app.proxy.capture_port().context("the runner proxy is not started")?, key: key.to_string() })
    }

    /// The command line before the program.
    pub fn prefix(&self) -> Vec<std::ffi::OsString> {
        vec![self.exe.clone().into(), "netcap".into(), "--port".into(), self.port.to_string().into(), "--key".into(), self.key.clone().into(), "--".into()]
    }
}

static AVAILABLE: tokio::sync::OnceCell<std::result::Result<(), String>> = tokio::sync::OnceCell::const_new();

/// Whether the supervisor works on this machine (checked once): agents run only under capture.
pub async fn check() -> Result<()> {
    let r = AVAILABLE
        .get_or_init(|| async {
            let exe = std::env::current_exe().map_err(|e| format!("{e}"))?;
            let out = tokio::time::timeout(std::time::Duration::from_secs(20), tokio::process::Command::new(&exe).args(["netcap", "--self-test"]).stdin(std::process::Stdio::null()).output())
                .await
                .map_err(|_| "the self-test did not finish".to_string())?
                .map_err(|e| format!("{e}"))?;
            if out.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let failed: Vec<String> = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout).unwrap_or_default().into_iter().filter(|c| c["ok"] != true).map(|c| format!("{}: {}", c["id"].as_str().unwrap_or("?"), c["detail"].as_str().unwrap_or(""))).collect();
            Err(if failed.is_empty() { stderr } else { failed.join("; ") })
        })
        .await
        .clone();
    r.map_err(|e| anyhow::anyhow!("agents cannot run on this machine: Launchpad's network capture does not work here ({e}). It needs Linux 5.9 or newer with seccomp user notification, on x86_64 or arm64."))
}

/// Burst and Local VM runners: puts the runner CA in the system certificate store when Launchpad may write it
/// (the Burst image makes the bundle writable). Returns whether the store holds it.
pub fn system_ca(ca: &std::path::Path) -> bool {
    let Ok(pem) = std::fs::read_to_string(ca) else { return false };
    let body = |t: &str| t.lines().filter(|l| !l.starts_with("-----")).collect::<String>();
    let ours = body(&pem);
    let has = |p: &str| std::fs::read_to_string(p).is_ok_and(|t| body(&t).contains(&ours));
    if SYSTEM_BUNDLES.iter().any(|p| has(p)) {
        return true;
    }
    let target = SYSTEM_BUNDLES[0];
    use std::io::Write;
    let appended = std::fs::OpenOptions::new().append(true).open(target).and_then(|mut f| f.write_all(format!("\n{pem}").as_bytes()));
    appended.is_ok() && has(target)
}

// ------------------------------------------------------------------ network check

/// One result of a runner's network check.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct NetworkCheckItem {
    pub id: String,
    pub label: String,
    pub ok: bool,
    /// A failed required item means a tool may work elsewhere and not on this runner.
    pub required: bool,
    pub detail: String,
}

/// An owner-started check of a runner's sandbox network, run inside a real session sandbox.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct NetworkCheck {
    #[ts(type = "number")]
    pub at: i64,
    /// `jail`, `vm` or `burst`.
    pub method: String,
    pub ok: bool,
    pub items: Vec<NetworkCheckItem>,
}

fn label(id: &str) -> (&'static str, bool) {
    match id {
        "capture" => ("Network capture runs", true),
        "socket" => ("Internet sockets go through the supervisor", true),
        "udp_refused" => ("UDP other than DNS is refused", true),
        "udp_dns" => ("DNS over UDP is allowed", true),
        "io_uring" => ("io_uring is blocked", true),
        "unix_path" => ("Unix sockets at a path work", true),
        "unix_abstract" => ("Abstract Unix sockets (X11, D-Bus) are refused", true),
        "link_local" => ("Link-local addresses (cloud metadata) are refused", true),
        "dns" => ("Names resolve", true),
        "tcp_v4" => ("IPv4 connection from a client that ignores proxy settings", true),
        "tcp_v6" => ("IPv6 connection (depends on the network)", false),
        "captured" => ("The runner proxy received that connection", true),
        "ca_store" => ("Runner CA in the system certificate store", true),
        "nondumpable_unix" => ("Unix sockets work in a non-dumpable process (Chrome)", true),
        "nondumpable_udp" => ("UDP other than DNS is refused in a non-dumpable process", true),
        "nondumpable_tcp" => ("A non-dumpable process connects through the capture", true),
        _ => ("Other check", false),
    }
}

fn item(id: &str, ok: bool, detail: String) -> NetworkCheckItem {
    let (l, required) = label(id);
    NetworkCheckItem { id: id.into(), label: l.into(), ok, required, detail }
}

/// Runner: runs the probe (`launchpad netcap-probe`) in a sandbox built like a session's, with an empty lease.
pub async fn network_check(app: &App) -> Result<NetworkCheck> {
    crate::role::ensure_runs_agents(app.paths.role)?;
    let vm = crate::contain::outside_jail(app);
    let method = if crate::home_link::burst().is_some() {
        "burst"
    } else if vm {
        "vm"
    } else {
        "jail"
    };
    let mut items = vec![];
    let done = |items: Vec<NetworkCheckItem>| NetworkCheck { at: crate::util::now_ms(), method: method.into(), ok: items.iter().all(|i| i.ok || !i.required), items };
    if let Err(e) = check().await {
        items.push(item("capture", false, format!("{e:#}")));
        return Ok(done(items));
    }
    items.push(item("capture", true, "the supervisor starts on this machine".into()));
    app.proxy.ensure(app).await?;
    const SESSION: &str = "network-check";
    let lease = crate::vault::Lease { id: format!("network-check-{}", hex::encode(rand::random::<[u8; 6]>())), session_id: SESSION.into(), generation: 0, credentials: vec![], tools: vec![], login: None, git_name: None, git_email: None, relayed: crate::home_link::burst().is_some(), mcp: vec![], launchpad: None };
    let active = app.proxy.register(lease);
    let result = probe(app, &active.key, vm).await;
    let captured = active.captured.load(std::sync::atomic::Ordering::Relaxed);
    app.proxy.drop_session(SESSION);
    let (probe_items, ok) = result?;
    let connected = probe_items.iter().any(|i| (i.id == "tcp_v4" || i.id == "tcp_v6") && i.ok);
    items.extend(probe_items);
    if ok {
        items.push(item("captured", connected && captured > 0, format!("{captured} connection(s) through the capture port")));
    }
    let check = done(items);
    Ok(check)
}

async fn probe(app: &App, key: &str, vm: bool) -> Result<(Vec<NetworkCheckItem>, bool)> {
    let (ca, bundle) = app.proxy.ca_files(app)?;
    if vm {
        system_ca(&ca);
    }
    let dir = app.paths.data_dir.join("network-check");
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp)?;
    let exe = std::env::current_exe()?;
    let p = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let jail = crate::jail::Jail {
        rw: vec![dir.clone()],
        ro: vec![ca.clone(), bundle.clone(), exe.clone()],
        tmp,
        env: vec![("HOME".into(), p(&dir)), ("TMPDIR".into(), "/tmp".into())],
        bwrap: !vm,
        capture: Some(Capture::new(app, key)?),
        ca_bundle: Some(bundle),
    };
    if jail.bwrap {
        crate::jail::check().await?;
    }
    let env = app.env.jailed(Some(std::sync::Arc::new(jail)));
    let mut cmd = crate::procs::command(&env, &p(&exe), Some(&dir));
    cmd.args(["netcap-probe", "--ca", &p(&ca)]).stdin(crate::exec::Io::Null).kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output()).await.context("the network check did not finish within two minutes")??;
    let raw: Vec<serde_json::Value> = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Ok((vec![item("socket", false, format!("the probe did not run: {}", String::from_utf8_lossy(&out.stderr).trim()))], false)),
    };
    let items = raw.into_iter().map(|c| item(c["id"].as_str().unwrap_or("?"), c["ok"] == true, c["detail"].as_str().unwrap_or_default().to_string())).collect();
    Ok((items, true))
}

/// Home: the last network check of each runner (until Home restarts).
static LAST: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, NetworkCheck>>> = std::sync::LazyLock::new(Default::default);

pub fn remember(runner_id: &str, check: &NetworkCheck) {
    LAST.lock().unwrap().insert(runner_id.to_string(), check.clone());
}

pub fn last(runner_id: &str) -> Option<NetworkCheck> {
    LAST.lock().unwrap().get(runner_id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_local_and_metadata_are_refused_except_dns() {
        for (ip, port) in [("169.254.169.254", 80), ("169.254.0.1", 443), ("::ffff:169.254.169.254", 80), ("fe80::1", 22), ("febf::1", 80), ("fd00:ec2::254", 80), ("64:ff9b::a9fe:a9fe", 80), ("100.100.100.200", 80)] {
            assert!(refused(ip.parse().unwrap(), port), "{ip}:{port}");
        }
        // DNS stays (GCE resolves on 169.254.169.254); the LAN, the tailnet and the internet stay reachable.
        for (ip, port) in [("169.254.169.254", 53), ("192.168.1.10", 80), ("10.0.0.1", 443), ("100.101.102.103", 22), ("100.100.100.100", 53), ("fd7a:115c:a1e0::1", 80), ("fd00:ec2::253", 80), ("140.82.112.3", 443), ("2606:4700::1", 443), ("127.0.0.1", 80)] {
            assert!(!refused(ip.parse().unwrap(), port), "{ip}:{port}");
        }
    }

    #[test]
    fn abstract_unix_addresses_are_told_apart() {
        let addr = |path: &[u8]| [&(libc::AF_UNIX as u16).to_ne_bytes()[..], path].concat();
        assert_eq!(unix_target(&addr(b"\0/tmp/.X11-unix/X0")), Some(UnixTarget::Abstract));
        assert_eq!(unix_target(&addr(b"/tmp/sccache.sock\0junk")), Some(UnixTarget::Path(b"/tmp/sccache.sock".to_vec())));
        assert_eq!(unix_target(&addr(b"")), Some(UnixTarget::Unnamed));
        assert_eq!(unix_target(&[&(libc::AF_INET as u16).to_ne_bytes()[..], &[0; 14]].concat()), None);
        assert_eq!(unix_target(&[1]), None);
    }
}
