//! The only externally served HTTP surface.
//!
//! Routes:
//!
//! * `GET /healthz` — liveness (always `200` while the process serves);
//! * `GET /readyz` — cached backend readiness;
//! * `/mcp/*` — the upstream MCP endpoint (client tools only).
//!
//! Everything else is `404`. The upstream `/admin/*` management plane is
//! unreachable both at the top level and through the `/mcp` mount.

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

use crate::config::model::NorthboundOAuthConfig;
use crate::gateway::reconnect::MonitorHandle;

/// Build the externally served router.
///
/// For legacy external callers without a monitor, readiness uses a snapshot.
/// Construct a serving gateway via Gateway::build for monitored readiness.
pub fn gateway_router(proxy: Proxy) -> Router {
    let monitor = proxy
        .application_extension::<MonitorHandle>()
        .map(|monitor| (*monitor).clone())
        .unwrap_or_else(|| MonitorHandle::snapshot_for_proxy(proxy.mcp_proxy().clone()));
    gateway_router_with_monitor_and_oauth(proxy, None, monitor)
}

pub fn gateway_router_with_monitor(proxy: Proxy, monitor: MonitorHandle) -> Router {
    gateway_router_with_monitor_and_oauth(proxy, None, monitor)
}

pub(crate) fn gateway_router_with_monitor_and_oauth(
    proxy: Proxy,
    oauth: Option<NorthboundOAuthConfig>,
    monitor: MonitorHandle,
) -> Router {
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
                let monitor = monitor.clone();
                async move {
                    if monitor.ready() {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }),
        )
        .merge(metadata)
        .nest("/mcp", upstream)
        .fallback(|| async { StatusCode::NOT_FOUND })
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
