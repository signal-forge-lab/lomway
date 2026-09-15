//! The only externally served HTTP surface.
//!
//! Routes:
//!
//! * `GET /healthz` — liveness (always `200` while the process serves);
//! * `GET /readyz` — live backend readiness;
//! * `/mcp/*` — the upstream MCP endpoint (client tools only).
//!
//! Everything else is `404`. The upstream `/admin/*` management plane is
//! unreachable both at the top level and through the `/mcp` mount.

use std::time::Duration;

use anyhow::Result;
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use mcp_proxy::Proxy;
use tower_mcp::proxy::McpProxy;

use crate::config::model::NorthboundOAuthConfig;

/// Build the externally served router.
///
/// Readiness is evaluated against the live backend transports rather than a
/// startup snapshot, so backend restarts correctly drive `/readyz` from 200
/// to 503 and back to 200 without restarting Lomway.
pub fn gateway_router(proxy: Proxy) -> Router {
    gateway_router_with_oauth(proxy, None)
}

pub(crate) fn gateway_router_with_oauth(
    proxy: Proxy,
    oauth: Option<NorthboundOAuthConfig>,
) -> Router {
    let readiness_proxy = proxy.mcp_proxy().clone();
    let expected_backends = readiness_proxy.backend_count();
    let (upstream, _sessions) = proxy.into_router();
    let upstream = upstream.layer(middleware::from_fn(block_upstream_admin));
    let (upstream, metadata) = match oauth {
        Some(oauth) => crate::gateway::auth::protect_mcp_router(upstream, oauth),
        None => (upstream, Router::new()),
    };
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route(
            "/readyz",
            get(move || {
                let proxy = readiness_proxy.clone();
                async move { backend_readiness(proxy, expected_backends).await }
            }),
        )
        .merge(metadata)
        .nest("/mcp", upstream)
        .fallback(|| async { StatusCode::NOT_FOUND })
}

async fn backend_readiness(proxy: McpProxy, expected_backends: usize) -> StatusCode {
    match tokio::time::timeout(Duration::from_secs(2), proxy.health_check()).await {
        Ok(statuses)
            if statuses.len() == expected_backends
                && statuses.iter().all(|status| status.healthy) =>
        {
            StatusCode::OK
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn block_upstream_admin(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path == "/admin" || path.starts_with("/admin/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

/// Serve an already-built router on the configured listener until Ctrl-C.
pub(crate) async fn serve_router(router: Router, host: &str, port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    tracing::info!(listen = %listener.local_addr()?, mcp_path = "/mcp", "gateway ready");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received");
        })
        .await?;
    Ok(())
}

/// Serve a proxy using Lomway's canonical external router.
pub async fn serve(proxy: Proxy, host: &str, port: u16) -> Result<()> {
    serve_router(gateway_router(proxy), host, port).await
}
