//! REST server lifecycle (auto-signer spec section 5.5)

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use eleven_barrage_collector::pool::BrowserPool;
use tracing::info;

use crate::api;

pub async fn run_rest_server(addr: SocketAddr, pool: Arc<BrowserPool>) -> Result<()> {
    let app = api::router(pool);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind REST on {}", addr))?;
    info!(addr = %addr, "REST server listening");
    axum::serve(listener, app)
        .await
        .context("REST server crashed")?;
    Ok(())
}
