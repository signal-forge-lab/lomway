//! The only externally served HTTP surface.
//!
//! Routes:
//!
//! * `GET /healthz` — liveness (always `200` while the process serves);
//! * `GET /readyz` — readiness with a JSON startup snapshot;
//! * `/mcp/*` — the upstream MCP endpoint (client tools only).
//!
//! Everything else is `404`. The upstream `/admin/*` management plane is
//! unreachable both at the top level and through the `/mcp` mount.

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use mcp_proxy::Proxy;

use crate::health::HealthState;

/// Build the gateway router from an upstream proxy and its startup health
/// state.
pub fn gateway_router(proxy: Proxy, health: HealthState) -> Router {
    let (upstream, _sessions) = proxy.into_router();
    let upstream = upstream.layer(middleware::from_fn(block_upstream_admin));
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/readyz", get(move || readyz(health.clone())))
        .nest("/mcp", upstream)
        .fallback(|| async { StatusCode::NOT_FOUND })
}

async fn readyz(health: HealthState) -> Response {
    let status = health.http_status();
    let body = health.json_body();
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

async fn block_upstream_admin(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path == "/admin" || path.starts_with("/admin/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}
