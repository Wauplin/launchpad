//! Startup-only Tailscale discovery. Never widen the listener to a LAN/public interface.
use launchpad_core::{config::ServerConfig, procs, shellenv::UserEnv};
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
            if https_port == 0
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
            .stdin(std::process::Stdio::null())
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

async fn discover(env: &UserEnv, port: u16, bind: IpAddr) -> (Vec<IpAddr>, Option<String>) {
    // GUI launches may not have Tailscale's install directory on PATH.
    let mut commands = vec![std::path::PathBuf::from("tailscale")];
    #[cfg(target_os = "macos")]
    commands.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
    #[cfg(target_os = "windows")]
    if let Some(root) = std::env::var_os("ProgramFiles") {
        commands.push(std::path::PathBuf::from(root).join("Tailscale/tailscale.exe"));
    }
    #[cfg(target_os = "linux")]
    commands.push("/usr/bin/tailscale".into());
    for executable in commands {
        if let Some(status) = command_json(env, &executable, &["status", "--json"]).await {
            let serve = command_json(env, &executable, &["serve", "status", "--json"])
                .await
                .unwrap_or(Value::Null);
            return (
                addresses(&status),
                https_origin(&status, &serve, port, bind),
            );
        }
    }
    (Vec::new(), None)
}

/// Prefer a configured private HTTPS Serve route, then an available direct listener.
/// Explicit origins always win (including HTTPS gateways and explicit local-only origins).
pub async fn prepare(config: &mut ServerConfig, port: u16, env: &UserEnv) -> Option<TcpListener> {
    if config.public_origin.is_some() || !config.auto_tailnet {
        return None;
    }
    let bind: IpAddr = config.bind.parse().ok()?;
    let (ips, https) = discover(env, port, bind).await;
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
