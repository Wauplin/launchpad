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

async fn discover(env: &UserEnv) -> Vec<IpAddr> {
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
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            procs::command(env, &executable.to_string_lossy(), None)
                .args(["status", "--json"])
                .stdin(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = result
            && output.status.success()
            && let Ok(status) = serde_json::from_slice(&output.stdout)
        {
            return addresses(&status);
        }
    }
    Vec::new()
}

/// Return an effective external origin only after its direct listener is available.
/// Explicit origins always win (including HTTPS gateways and explicit local-only origins).
pub async fn prepare(config: &mut ServerConfig, port: u16, env: &UserEnv) -> Option<TcpListener> {
    if config.public_origin.is_some() || !config.auto_tailnet {
        return None;
    }
    prepare_addresses(config, port, discover(env).await).await
}

async fn prepare_addresses(
    config: &mut ServerConfig,
    port: u16,
    ips: Vec<IpAddr>,
) -> Option<TcpListener> {
    let bind: IpAddr = config.bind.parse().ok()?;
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
