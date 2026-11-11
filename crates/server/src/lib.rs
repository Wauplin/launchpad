//! HTTP + WebSocket server; loopback plus discovered tailnet access, or a configured gateway.

mod activity;
mod burst;
mod browser;
mod auth;
mod exchange;
mod git_http;
mod plain_http;
mod home_client;
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
    // A planned stop (update, restart) takes a final backup first.
    if app.paths.role == launchpad_core::role::Role::Home
        && let Err(e) = launchpad_core::home_backup::run(&app, false).await
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
        .merge(browser::router())
        .merge(terminals::router())
        .merge(worktrees::router())
        .merge(runners::router())
        .merge(burst::router())
        .merge(git_http::router())
        .merge(plain_http::router())
        .route("/ws", axum::routing::get(ws::handler))
        .merge(auth::router())
        .fallback(ui::serve)
        .layer(axum::middleware::from_fn_with_state(server.clone(), runners::relay))
        .layer(axum::middleware::from_fn_with_state(server.clone(), home_client::forward_writes))
        .layer(axum::middleware::from_fn_with_state(operations, operations::guard))
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
