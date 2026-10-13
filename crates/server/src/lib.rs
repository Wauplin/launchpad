//! HTTP + WebSocket server of the daemon, on 127.0.0.1 only.

mod api;
mod auth;
mod code;
mod error;
mod items;
mod operations;
mod predrafts;
mod routes;
mod suggestions;
mod terminals;
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
    let auth = Arc::new(auth::Auth::new(&app.paths)?);
    let operations = operations::Operations::new(app.state_db.clone());
    let server = Server { app, auth };
    let router: Router = Router::new()
        .merge(routes::router())
        .merge(api::router())
        .merge(code::router())
        .merge(items::router())
        .merge(predrafts::router())
        .merge(suggestions::router())
        .merge(terminals::router())
        .merge(worktrees::router())
        .route("/ws", axum::routing::get(ws::handler))
        .route("/auth", axum::routing::get(auth::handshake))
        .fallback(ui::serve)
        .layer(axum::middleware::from_fn_with_state(operations, operations::guard))
        .layer(axum::middleware::from_fn_with_state(server.clone(), auth::guard))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(server);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("bind 127.0.0.1:{port} (is another daemon running?)"))?;
    tracing::info!("listening on http://127.0.0.1:{port}");
    axum::serve(listener, router).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
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
