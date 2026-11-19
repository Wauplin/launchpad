//! HTTP + WebSocket server; loopback plus discovered tailnet access, or a configured gateway.

mod activity;
mod burst;
mod auth;
mod exchange;
mod git_http;
mod plain_http;
mod home_client;
mod maintenance;
mod operations;
mod retention;
mod routes;
mod runners;
mod runner_trust;
mod terminals;
mod tailnet;
mod worktrees;
mod ui;
mod wire;
mod ws;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use launchpad_core::App;

pub use launchpad_http::ApiError;
use launchpad_http::{Ctx, agent_auth, error};

#[derive(Clone)]
pub struct Server {
    pub app: App,
    pub auth: Arc<auth::Auth>,
}

pub use home_client::connect;
pub use ui::folio_viewer_files;

/// Home: an oracle's `tools.launchpad` read, in process: a GET of this daemon's router with the oracle's workspace as
/// its principal (`launchpad_http::OracleReader`, checked by `auth::guard` with `agent_auth::authorize_oracle`), no
/// token and nothing over the network. The status and the body (at most 16 MiB).
pub async fn oracle_read(app: &App, ws: &str, path: &str) -> Result<(u16, String)> {
    use tower::ServiceExt;
    anyhow::ensure!(path.starts_with("/api/"), "not a path of Home's API");
    let router = ROUTER.get().context("Home is starting")?;
    let mut req = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri(path)
        .header(axum::http::header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .body(axum::body::Body::empty())?;
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    req.extensions_mut().insert(launchpad_http::OracleReader(ws.to_string()));
    let res = router.clone().oneshot(req).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 16 << 20).await.context("the answer is larger than 16 MiB")?;
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// The daemon's own router, for API requests that arrive through a runner tunnel.
pub(crate) static ROUTER: std::sync::OnceLock<Router> = std::sync::OnceLock::new();

/// Serves the daemon's API: this crate's routes and `api` (`launchpad_api::router`).
pub async fn serve(app: App, api: Router<Ctx>) -> Result<()> {
    let port = app.paths.port;
    let mut config = app.config.get().server;
    // A Burst runner listens on every interface: Home reaches it through the HF Jobs proxy, and its router only
    // accepts Home's connection there (other Hosts are refused).
    let bind = if launchpad_core::home_link::burst().is_some() { std::net::IpAddr::from([0, 0, 0, 0]) } else { config.bind.parse::<std::net::IpAddr>()? };
    let listener = tokio::net::TcpListener::bind((bind, port))
        .await
        .with_context(|| format!("bind {bind}:{port} (is another daemon running?)"))?;
    tracing::info!("listening on {bind}:{port}");
    let tailnet = tailnet::prepare(&mut config, port, &app.env).await;
    let auth = Arc::new(auth::Auth::new(&app.paths, &config, app.state_db.clone()).await?);
    let server = Server { app: app.clone(), auth };
    let server_for_burst = server.clone();
    let router = router(server, api);
    let _ = ROUTER.set(router.clone());
    if app.paths.role == launchpad_core::role::Role::Runner {
        home_client::spawn(app.clone(), router.clone());
    } else {
        // Exit routes: the states of their exits, kept current for Home's own requests and the runners.
        launchpad_core::egress::spawn(&app);
    }
    burst::spawn(server_for_burst.clone());
    retention::spawn(&app);
    if let Some(tailnet) = tailnet {
        use std::future::IntoFuture;
        tokio::try_join!(
            axum::serve(listener, router.clone().into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).into_future(),
            axum::serve(tailnet, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).into_future(),
        )?;
    } else {
        axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).await?;
    }
    // A stop takes a final backup first, best effort (a planned update does not rely on it: its checkpoint was taken
    // and checked before the replacement, and `state.db` is frozen since, so nothing newer exists to save).
    if app.paths.role == launchpad_core::role::Role::Home
        && !launchpad_core::maintenance::frozen()
        && let Err(e) = launchpad_core::home_backup::run(&app, launchpad_core::home_backup::When::Changed).await
    {
        tracing::warn!("final home backup: {e:#}");
    }
    Ok(())
}

pub(crate) fn router(server: Server, api: Router<Ctx>) -> Router {
    let operations = operations::Operations::new(server.app.state_db.clone());
    let ctx = Ctx { app: server.app.clone(), auth: server.auth.clone() };
    Router::new()
        .merge(routes::router())
        .merge(api.with_state(ctx))
        .merge(terminals::router())
        .merge(worktrees::router())
        .merge(runners::router())
        .merge(burst::router())
        .merge(git_http::router())
        .merge(plain_http::router())
        .route("/ws", axum::routing::get(ws::handler))
        .route("/api/events", axum::routing::get(ws::events))
        .merge(auth::router())
        .fallback(ui::serve)
        .layer(axum::middleware::from_fn_with_state(server.clone(), runners::relay))
        .layer(axum::middleware::from_fn_with_state(server.clone(), home_client::forward_writes))
        .layer(axum::middleware::from_fn_with_state(operations, operations::guard))
        // Home's planned update: writes are refused (and running ones counted) before anything else sees them.
        .layer(axum::middleware::from_fn(maintenance::gate))
        .layer(axum::middleware::from_fn_with_state(server.clone(), auth::guard))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(server)
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c => {},
        _ = term.recv() => {},
        _ = launchpad_core::self_update::shutdown_requested() => {},
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    tracing::info!("shutting down");
}
