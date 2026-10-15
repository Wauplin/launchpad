//! Startup-only Tailscale discovery. Never widen the listener to a LAN/public interface.
use launchpad_core::{config::ServerConfig, exec::Io, procs, shellenv::UserEnv};
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::net::TcpListener;

fn addresses(status: &Value) -> Vec<IpAddr> {
    if status["BackendState"] != "Running" {
        return Vec::new();
    }
    let mut ips: Vec<IpAddr> = status["TailscaleIPs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|ip| ip.as_str()?.parse().ok())
        .filter(|ip| match ip {
            IpAddr::V4(ip) => ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]),
            IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
        })
        .collect();
    ips.sort_by_key(|ip| !ip.is_ipv4());
    ips.dedup();
    ips
}

/// Adopt only this node's private HTTPS Serve route to this daemon at the origin root.
fn https_origin(status: &Value, serve: &Value, port: u16, bind: IpAddr) -> Option<String> {
    https_origin_at(status, serve, port, bind, None)
}

fn https_origin_at(
    status: &Value,
    serve: &Value,
    port: u16,
    bind: IpAddr,
    selected: Option<u16>,
) -> Option<String> {
    if status["BackendState"] != "Running" {
        return None;
    }
    let dns = status["Self"]["DNSName"].as_str()?.trim_end_matches('.');
    if dns.is_empty() {
        return None;
    }
    let mut targets = Vec::new();
    if bind == "127.0.0.1".parse::<IpAddr>().unwrap()
        || bind == "0.0.0.0".parse::<IpAddr>().unwrap()
    {
        targets.extend([
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            port.to_string(),
        ]);
    }
    if bind == "::1".parse::<IpAddr>().unwrap() || bind == "::".parse::<IpAddr>().unwrap() {
        targets.extend([
            format!("[::1]:{port}"),
            format!("localhost:{port}"),
            port.to_string(),
        ]);
    }
    let mut candidates = Vec::new();
    let configs = std::iter::once(serve).chain(
        serve["Foreground"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.values()),
    );
    for config in configs {
        for (authority, web) in config["Web"].as_object().into_iter().flat_map(|v| v.iter()) {
            let Some((host, https_port)) = authority.rsplit_once(':') else {
                continue;
            };
            let Ok(https_port) = https_port.parse::<u16>() else {
                continue;
            };
            if selected.is_some_and(|port| port != https_port)
                || https_port == 0
                || !host.eq_ignore_ascii_case(dns)
                || config["TCP"][https_port.to_string()]["HTTPS"] != true
                || config["AllowFunnel"][authority] == true
                || serve["AllowFunnel"][authority] == true
            {
                continue;
            }
            let Some(handlers) = web["Handlers"].as_object() else {
                continue;
            };
            let Some(proxy) = handlers.get("/").and_then(|h| h["Proxy"].as_str()) else {
                continue;
            };
            let proxy = proxy.strip_prefix("http://").unwrap_or(proxy);
            let proxy = proxy.strip_suffix('/').unwrap_or(proxy);
            if !targets.iter().any(|target| target == proxy) {
                continue;
            }
            // Other apps may share this host, but must not intercept Launchpad routes.
            let paths = [
                "/auth",
                "/api",
                "/ws",
                "/assets",
                "/icons",
                "/sw.js",
                "/manifest.webmanifest",
                "/now",
                "/review",
                "/terminals",
                "/inbox",
                "/predrafts",
                "/github",
                "/publish",
                "/agents",
                "/projects",
                "/settings",
                "/dev",
            ];
            if handlers.keys().any(|path| {
                path != "/"
                    && paths.iter().any(|root| {
                        let path = path.trim_end_matches('/');
                        path == *root
                            || path.starts_with(&format!("{root}/"))
                            || root.starts_with(&format!("{path}/"))
                    })
            }) {
                continue;
            }
            let origin = ServerConfig {
                public_origin: Some(format!("https://{authority}")),
                ..ServerConfig::default()
            };
            if let Ok(Some(origin)) = origin.origin() {
                candidates.push((https_port != 443, https_port, origin));
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next().map(|(_, _, origin)| origin)
}

async fn command_json(env: &UserEnv, executable: &std::path::Path, args: &[&str]) -> Option<Value> {
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        procs::command(env, &executable.to_string_lossy(), None)
            .args(args)
            .stdin(Io::Null)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn executable(env: &UserEnv) -> Option<std::path::PathBuf> {
    // GUI launches may not have Tailscale's install directory on PATH.
    let mut commands: Vec<_> = env.which("tailscale").into_iter().collect();
    #[cfg(target_os = "macos")]
    commands.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
    #[cfg(target_os = "windows")]
    if let Some(root) = std::env::var_os("ProgramFiles") {
        commands.push(std::path::PathBuf::from(root).join("Tailscale/tailscale.exe"));
    }
    #[cfg(target_os = "linux")]
    commands.push("/usr/bin/tailscale".into());
    commands.into_iter().find(|path| path.is_file())
}

async fn read_state(env: &UserEnv) -> Option<(Value, Value)> {
    let executable = executable(env)?;
    let status = command_json(env, &executable, &["status", "--json"])
        .await
        .unwrap_or(Value::Null);
    let serve = command_json(env, &executable, &["serve", "status", "--json"])
        .await
        .unwrap_or(Value::Null);
    Some((status, serve))
}

fn port_taken(serve: &Value, port: u16) -> bool {
    let key = port.to_string();
    std::iter::once(serve)
        .chain(
            serve["Foreground"]
                .as_object()
                .into_iter()
                .flat_map(|v| v.values()),
        )
        .any(|config| {
            config["TCP"].get(&key).is_some()
                || config["AllowFunnel"]
                    .as_object()
                    .into_iter()
                    .flat_map(|v| v.iter())
                    .any(|(authority, enabled)| {
                        *enabled == true
                            && authority
                                .rsplit_once(':')
                                .and_then(|(_, value)| value.parse::<u16>().ok())
                                == Some(port)
                    })
        })
}

fn setup_port(status: &Value, serve: &Value, port: u16, bind: IpAddr) -> Option<u16> {
    if let Some(origin) = https_origin(status, serve, port, bind) {
        return Some(
            origin
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse().ok())
                .unwrap_or(443),
        );
    }
    [443, 8443, 9443, 10443]
        .into_iter()
        .find(|port| !port_taken(serve, *port))
}

pub struct Setup {
    pub installed: bool,
    pub connected: bool,
    pub port: Option<u16>,
}

pub async fn setup(env: &UserEnv, port: u16, bind: IpAddr) -> Setup {
    let state = read_state(env).await;
    let connected = state
        .as_ref()
        .is_some_and(|(status, _)| status["BackendState"] == "Running");
    let suggested = state.as_ref().and_then(|(status, serve)| {
        if serve.is_null() {
            None
        } else {
            setup_port(status, serve, port, bind)
        }
    });
    Setup {
        installed: executable(env).is_some(),
        connected,
        port: suggested,
    }
}

pub struct RunResult {
    pub origin: Option<String>,
    pub output: String,
    pub approval_url: Option<String>,
}

fn approval_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| matches!(c, '(' | ')' | '\'' | '"' | ',' | '.')))
        .find(|url| {
            [
                "https://login.tailscale.com/",
                "https://tailscale.com/",
                "https://admin.tailscale.com/",
            ]
            .iter()
            .any(|prefix| url.starts_with(prefix))
        })
        .map(String::from)
}

pub async fn check(
    env: &UserEnv,
    port: u16,
    bind: IpAddr,
    https_port: u16,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(https_port != 0, "Choose an HTTPS port from 1 to 65535");
    let (status, serve) = read_state(env)
        .await
        .ok_or_else(|| anyhow::anyhow!("Tailscale is not installed"))?;
    Ok(https_origin_at(
        &status,
        &serve,
        port,
        bind,
        Some(https_port),
    ))
}

/// Runs only a fixed private Serve root proxy, after checking the selected port.
/// Output is bounded; approval or permission requests are returned to the owner.
pub async fn run(
    env: &UserEnv,
    port: u16,
    bind: IpAddr,
    https_port: u16,
) -> anyhow::Result<RunResult> {
    use tokio::io::AsyncReadExt;
    anyhow::ensure!(https_port != 0, "Choose an HTTPS port from 1 to 65535");
    anyhow::ensure!(
        bind == "127.0.0.1".parse::<IpAddr>()? || bind == "0.0.0.0".parse::<IpAddr>()?,
        "Launchpad must listen on 127.0.0.1 for Tailscale Serve"
    );
    let executable =
        executable(env).ok_or_else(|| anyhow::anyhow!("Tailscale is not installed"))?;
    let (status, serve) = read_state(env)
        .await
        .ok_or_else(|| anyhow::anyhow!("Tailscale is not installed"))?;
    anyhow::ensure!(
        status["BackendState"] == "Running",
        "Sign in to Tailscale on this machine, then try again"
    );
    anyhow::ensure!(
        !serve.is_null(),
        "Cannot inspect existing Tailscale Serve routes; no command was run"
    );
    if let Some(origin) = https_origin_at(&status, &serve, port, bind, Some(https_port)) {
        return Ok(RunResult {
            origin: Some(origin),
            output: "HTTPS is already ready. Pairing URL updated.".into(),
            approval_url: None,
        });
    }
    anyhow::ensure!(
        !port_taken(&serve, https_port),
        "HTTPS port {https_port} is already reserved by another route or Funnel. Choose an unused port"
    );
    let mut child = procs::command(env, &executable.to_string_lossy(), None)
        .args([
            "serve",
            "--bg",
            &format!("--https={https_port}"),
            "--set-path=/",
            &format!("http://127.0.0.1:{port}"),
        ])
        .stdin(Io::Null)
        .stdout(Io::Piped)
        .stderr(Io::Piped)
        .spawn().await?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout
            .take(65536)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let err = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr
            .take(65536)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let timed_out = match tokio::time::timeout(Duration::from_secs(12), child.wait()).await {
        Ok(status) => {
            status?;
            false
        }
        Err(_) => {
            child.kill().await?;
            let _ = child.wait().await;
            true
        }
    };
    let (out, err) = tokio::join!(out, err);
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out??),
        String::from_utf8_lossy(&err??)
    );
    let approval_url = approval_url(&output);
    let origin = check(env, port, bind, https_port).await?;
    let mut output = output.trim().to_string();
    if origin.is_some() {
        output.push_str("\nHTTPS is ready. Pairing URL updated.");
    } else if timed_out {
        output.push_str("\nThe command timed out. Complete any Tailscale approval, then check HTTPS before running the command again.");
    } else {
        output.push_str("\nHTTPS is not ready. Complete any approval or resolve the error above, then run the command again.");
    }
    Ok(RunResult {
        origin,
        output,
        approval_url,
    })
}

/// Prefer a configured private HTTPS Serve route, then an available direct listener.
/// Explicit origins always win (including HTTPS gateways and explicit local-only origins).
pub async fn prepare(config: &mut ServerConfig, port: u16, env: &UserEnv) -> Option<TcpListener> {
    if config.public_origin.is_some() || !config.auto_tailnet {
        return None;
    }
    let bind: IpAddr = config.bind.parse().ok()?;
    let (status, serve) = read_state(env).await?;
    let ips = addresses(&status);
    let https = https_origin(&status, &serve, port, bind);
    if let Some(origin) = https {
        tracing::info!("using Tailscale Serve origin {origin}");
        config.public_origin = Some(origin);
        return None;
    }
    for ip in ips {
        let address = SocketAddr::new(ip, port);
        if bind == ip || (bind.is_unspecified() && bind.is_ipv4() == ip.is_ipv4()) {
            config.public_origin = Some(format!("http://{address}"));
            return None;
        }
        if !bind.is_loopback() {
            continue;
        }
        match TcpListener::bind(address).await {
            Ok(listener) => {
                tracing::info!("listening on tailnet {address}");
                config.public_origin = Some(format!("http://{address}"));
                return Some(listener);
            }
            Err(error) => tracing::warn!("cannot listen on tailnet {address}: {error}"),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approval_links_only_open_tailscale_https_hosts() {
        assert_eq!(
            approval_url("Approve: https://login.tailscale.com/a/abc"),
            Some("https://login.tailscale.com/a/abc".into())
        );
        assert!(approval_url("https://login.tailscale.com.evil.example/a").is_none());
        assert!(approval_url("http://login.tailscale.com/a").is_none());
        assert!(approval_url("https://other.example/a").is_none());
    }

    #[test]
    fn setup_port_keeps_existing_routes_and_public_funnel_ports() {
        let bind = "127.0.0.1".parse().unwrap();
        let mut serve = json!({"TCP":{"443":{"HTTPS":true}},
            "Foreground":{"other":{"TCP":{"8443":{"HTTPS":true}}}},
            "AllowFunnel":{"host.example.ts.net:9443":true}});
        assert_eq!(setup_port(&Value::Null, &serve, 7317, bind), Some(10443));
        serve["TCP"]["10443"] = json!({"TCPForward":"127.0.0.1:8765"});
        assert_eq!(setup_port(&Value::Null, &serve, 7317, bind), None);
        assert_eq!(setup_port(&Value::Null, &json!({}), 7317, bind), Some(443));
    }

    #[test]
    fn https_serve_must_match_this_node_daemon_and_private_root_route() {
        let status = json!({"BackendState":"Running", "Self":{"DNSName":"host.example.ts.net."}});
        let serve = json!({"TCP":{"443":{"HTTPS":true},"8443":{"HTTPS":true}},
            "Web":{"host.example.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7318"},
                "/other-app":{"Proxy":"http://127.0.0.1:8765"}}}}});
        let bind = "127.0.0.1".parse().unwrap();
        let origin = Some("https://host.example.ts.net".into());
        assert_eq!(https_origin(&status, &serve, 7318, bind), origin);
        assert_eq!(https_origin(&status, &serve, 7317, bind), None);
        assert_eq!(
            https_origin(&status, &serve, 7318, "192.168.1.2".parse().unwrap()),
            None
        );
        assert_eq!(
            https_origin(&json!({"BackendState":"Stopped"}), &serve, 7318, bind),
            None
        );
        for proxy in ["http://localhost:7318/", "127.0.0.1:7318", "7318"] {
            let mut changed = serve.clone();
            changed["Web"]["host.example.ts.net:443"]["Handlers"]["/"]["Proxy"] = json!(proxy);
            assert_eq!(https_origin(&status, &changed, 7318, bind), origin);
        }
        for proxy in [
            "http://127.0.0.1:7317",
            "http://127.0.0.1:7318/subpath",
            "https://127.0.0.1:7318",
            "http://remote:7318",
        ] {
            let mut changed = serve.clone();
            changed["Web"]["host.example.ts.net:443"]["Handlers"]["/"]["Proxy"] = json!(proxy);
            assert_eq!(https_origin(&status, &changed, 7318, bind), None);
        }
        let mut changed = serve.clone();
        changed["AllowFunnel"]["host.example.ts.net:443"] = json!(true);
        assert_eq!(https_origin(&status, &changed, 7318, bind), None);
        changed = serve.clone();
        changed["TCP"]["443"]["HTTPS"] = json!(false);
        assert_eq!(https_origin(&status, &changed, 7318, bind), None);
        changed = serve.clone();
        changed["Web"]["host.example.ts.net:443"]["Handlers"]["/api/"] =
            json!({"Proxy":"http://127.0.0.1:8765"});
        assert_eq!(https_origin(&status, &changed, 7318, bind), None);
        changed = serve.clone();
        let web = changed["Web"].as_object_mut().unwrap();
        let route = web.remove("host.example.ts.net:443").unwrap();
        web.insert("host.example.ts.net:8443".into(), route.clone());
        assert_eq!(
            https_origin(&status, &changed, 7318, bind),
            Some("https://host.example.ts.net:8443".into())
        );
        changed["Web"]
            .as_object_mut()
            .unwrap()
            .insert("host.example.ts.net:443".into(), route);
        assert_eq!(https_origin(&status, &changed, 7318, bind), origin);
        assert_eq!(
            https_origin(
                &status,
                &json!({"Foreground":{"session":serve}}),
                7318,
                bind
            ),
            origin
        );
    }

    #[test]
    fn only_running_local_tailnet_addresses_are_used_ipv4_first() {
        let status = json!({"BackendState":"Running", "TailscaleIPs":[
            "fd7a:115c:a1e0::1", "192.168.1.2", "100.64.1.2", "127.0.0.1",
            "100.128.0.1", "bad", "::1"], "Peer":{"other":{"TailscaleIPs":["100.64.9.9"]}}});
        assert_eq!(
            addresses(&status),
            vec![
                "100.64.1.2".parse::<IpAddr>().unwrap(),
                "fd7a:115c:a1e0::1".parse().unwrap()
            ]
        );
        assert!(
            addresses(&json!({"BackendState":"Stopped","TailscaleIPs":["100.64.1.2"]})).is_empty()
        );
        assert!(addresses(&json!({})).is_empty());
    }
}
