//! The network capture with a non-dumpable process (Chrome, crashpad): its Unix sockets work, UDP other than DNS
//! stays refused and its TCP connects still go through the capture port. Runs the real supervisor with a stand-in
//! for the runner proxy's capture port; skipped inside a session, where a capture already runs (seccomp allows
//! one listener).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;

#[test]
fn nondumpable_process_stays_captured() {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = proxy.local_addr().unwrap().port();
    let seen = std::thread::spawn(move || {
        let (mut s, _) = proxy.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        s.write_all(b"OK\n").unwrap();
        line
    });
    let exe = env!("CARGO_BIN_EXE_launchpad");
    let out = Command::new(exe).args(["netcap", "--port", &port.to_string(), "--key", "k", "--", exe, "netcap-probe", "--nondumpable", "192.0.2.1:9"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.code() == Some(126) && stderr.contains("seccomp user notification refused") {
        eprintln!("skipped: this process already runs under a capture");
        return;
    }
    let checks: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("probe output: {stderr}"));
    for c in &checks {
        assert_eq!(c["ok"], true, "{c}");
    }
    assert_eq!(checks.len(), 3);
    assert_eq!(seen.join().unwrap(), "LPCAP1 k 192.0.2.1 9\n");
}
