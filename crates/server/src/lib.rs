//! HTTP + WebSocket server; loopback plus discovered tailnet access, or a configured gateway.

mod api;
mod browser;
mod auth;
mod code;
mod error;
mod hosts;
mod items;
mod operations;
mod predrafts;
mod push;
mod review_state;
mod routes;
mod setup;
mod suggestions;
mod terminals;
mod tailnet;
mod worktrees;
mod ui;
mod ws;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use launchpad_core::App;

pub use error::ApiError;

#[derive(Clone)]
pub struct Server {
    pub app: App,
    pub auth: Arc<auth::Auth>,
}

pub async fn serve(app: App) -> Result<()> {
    let port = app.paths.port;
    let mut config = app.config.get().server;
    let bind = config.bind.parse::<std::net::IpAddr>()?;
    let listener = tokio::net::TcpListener::bind((bind, port))
        .await
        .with_context(|| format!("bind {bind}:{port} (is another daemon running?)"))?;
    tracing::info!("listening on {bind}:{port}");
    let tailnet = tailnet::prepare(&mut config, port, &app.env).await;
    let auth = Arc::new(auth::Auth::new(&app.paths, &config, app.state_db.clone()).await?);
    let server = Server { app, auth };
    let router = router(server);
    if let Some(tailnet) = tailnet {
        use std::future::IntoFuture;
        tokio::try_join!(
            axum::serve(listener, router.clone().into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).into_future(),
            axum::serve(tailnet, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).into_future(),
        )?;
    } else {
        axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).await?;
    }
    Ok(())
}

pub(crate) fn router(server: Server) -> Router {
    let operations = operations::Operations::new(server.app.state_db.clone());
    Router::new()
        .merge(routes::router())
        .merge(api::router())
        .merge(browser::router())
        .merge(code::router())
        .merge(items::router())
        .merge(predrafts::router())
        .merge(review_state::router())
        .merge(suggestions::router())
        .merge(terminals::router())
        .merge(worktrees::router())
        .merge(setup::router())
        .merge(hosts::router())
        .merge(push::router())
        .route("/ws", axum::routing::get(ws::handler))
        .merge(auth::router())
        .fallback(ui::serve)
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
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    tracing::info!("shutting down");
}
