use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{Result, ensure};
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use mcp_proxy::{Proxy, ProxyConfig};
use tower::timeout::TimeoutLayer;
use tower_mcp::{client::HttpClientTransport, proxy::McpProxy};

pub mod backend;
pub mod cli;
pub mod config;
pub mod gateway;
pub mod health;
pub mod namespace;

/// Server name the gateway reports for itself during MCP initialization.
pub const DEFAULT_SERVER_NAME: &str = "lomway";

const LOOPBACK_PREFIX: &str = "http://127.0.0.1:";
const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RECONNECT_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const RECONNECT_HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const RECONNECT_STABLE_PROBES: u8 = 2;
const RECONNECT_UNHEALTHY_PROBES: u8 = 2;

#[derive(Clone)]
struct BackendReconnectSpec {
    name: String,
    url: String,
    port: u16,
    bearer_token: Option<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Default)]
struct BackendReconnectState {
    observed_down: bool,
    stable_up_probes: u8,
    unhealthy_probes: u8,
    reconnect_pending: bool,
}

/// Load, resolve environment references, and enforce this gateway's stricter
/// production policy before any backend connection is attempted.
pub fn load_config(path: &Path) -> Result<ProxyConfig> {
    let mut config = ProxyConfig::load(path)?;
    let missing = config.check_env_vars();
    ensure!(
        missing.is_empty(),
        "configuration contains unresolved environment references: {}",
        missing.join(", ")
    );
    config.resolve_env_vars();
    validate_policy(&config)?;
    Ok(config)
}

/// Enforce the intentionally small v1 responsibility boundary.
///
/// Upstream supports many routing/middleware features. This gateway does not:
/// arbitrary retries, fan-out, schema rewriting, dynamic reload, remote
/// backends, or northbound auth belong outside this thin local boundary.
///
/// The gate delegates to the decomposed validators in
/// [`gateway::policy::validate_proxy_policy`], which reproduce the original
/// accept/reject decisions and error text exactly. In particular the legacy
/// deployment schema keeps requiring at least one backend, matching the
/// pinned upstream loader.
pub fn validate_policy(config: &ProxyConfig) -> Result<()> {
    gateway::policy::validate_proxy_policy(config)
}

fn loopback_port(url: &str) -> Option<u16> {
    let rest = url.strip_prefix(LOOPBACK_PREFIX)?;
    let (port, path) = rest.split_once('/')?;
    if path != "mcp" {
        return None;
    }
    port.parse::<u16>().ok().filter(|port| *port > 0)
}

fn reconnect_specs(config: &ProxyConfig) -> Vec<BackendReconnectSpec> {
    config
        .backends
        .iter()
        .filter_map(|backend| {
            let url = backend.url.as_ref()?;
            let port = loopback_port(url)?;
            Some(BackendReconnectSpec {
                name: backend.name.clone(),
                url: url.clone(),
                port,
                bearer_token: backend.bearer_token.clone(),
                timeout_seconds: backend.timeout.as_ref().map(|timeout| timeout.seconds),
            })
        })
        .collect()
}

/// Construct the upstream proxy and fail closed unless its client-visible
/// control-plane backend can be removed before serving.
pub async fn build_proxy(config: ProxyConfig) -> Result<Proxy> {
    validate_policy(&config)?;
    let specs = reconnect_specs(&config);
    let proxy = Proxy::from_config(config).await?;
    ensure!(
        proxy.mcp_proxy().remove_backend("proxy").await,
        "upstream control-plane MCP backend 'proxy' was not present; refusing to serve because admin-tool removal cannot be proven"
    );
    spawn_backend_reconnectors(proxy.mcp_proxy().clone(), specs);
    Ok(proxy)
}

fn spawn_backend_reconnectors(proxy: McpProxy, specs: Vec<BackendReconnectSpec>) {
    let active = proxy.backend_namespaces();
    let specs: Vec<_> = specs
        .into_iter()
        .filter(|spec| active.iter().any(|name| name == &spec.name))
        .collect();
    if specs.is_empty() {
        return;
    }
    tokio::spawn(async move {
        monitor_backend_restarts(proxy, specs).await;
    });
}

async fn monitor_backend_restarts(proxy: McpProxy, specs: Vec<BackendReconnectSpec>) {
    let mut states: HashMap<String, BackendReconnectState> = specs
        .iter()
        .map(|spec| (spec.name.clone(), BackendReconnectState::default()))
        .collect();

    loop {
        tokio::time::sleep(RECONNECT_POLL_INTERVAL).await;
        let health = tokio::time::timeout(RECONNECT_HEALTH_TIMEOUT, proxy.health_check())
            .await
            .ok();

        for spec in &specs {
            let state = states
                .get_mut(&spec.name)
                .expect("reconnect state exists for every backend spec");
            let reachable = backend_port_reachable(spec.port).await;

            if !reachable {
                if !state.observed_down {
                    tracing::warn!(backend = %spec.name, port = spec.port, "backend became unreachable; waiting for recovery");
                }
                state.observed_down = true;
                state.stable_up_probes = 0;
                state.unhealthy_probes = 0;
                state.reconnect_pending = false;
                continue;
            }

            if state.observed_down {
                state.stable_up_probes = state.stable_up_probes.saturating_add(1);
                if state.stable_up_probes >= RECONNECT_STABLE_PROBES {
                    state.reconnect_pending = true;
                }
            } else if !state.reconnect_pending {
                let backend_health = health.as_ref().and_then(|statuses| {
                    statuses.iter().find(|status| status.namespace == spec.name)
                });
                match backend_health {
                    Some(status) if status.healthy => state.unhealthy_probes = 0,
                    Some(_) => {
                        state.unhealthy_probes = state.unhealthy_probes.saturating_add(1);
                        if state.unhealthy_probes == 1 {
                            tracing::warn!(backend = %spec.name, "backend MCP health is unhealthy; waiting for confirmation");
                        }
                        if state.unhealthy_probes >= RECONNECT_UNHEALTHY_PROBES {
                            state.reconnect_pending = true;
                        }
                    }
                    None => {}
                }
            }

            if !state.reconnect_pending {
                continue;
            }

            match replace_backend_transport(&proxy, spec).await {
                Ok(()) => {
                    let reason = if state.observed_down {
                        "backend transport reconnected after restart"
                    } else {
                        "backend transport reconnected after MCP health failure"
                    };
                    tracing::info!(backend = %spec.name, "{reason}");
                    *state = BackendReconnectState::default();
                }
                Err(error) => {
                    tracing::warn!(backend = %spec.name, error = %error, "backend transport reconnect failed; will retry");
                }
            }
        }
    }
}

async fn backend_port_reachable(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            RECONNECT_PROBE_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn replace_backend_transport(proxy: &McpProxy, spec: &BackendReconnectSpec) -> Result<()> {
    proxy.remove_backend(&spec.name).await;

    let mut transport = HttpClientTransport::new(&spec.url);
    if let Some(token) = &spec.bearer_token {
        transport = transport.bearer_token(token);
    }

    let result = if let Some(seconds) = spec.timeout_seconds {
        proxy
            .add_backend_with_layer(
                &spec.name,
                transport,
                TimeoutLayer::new(Duration::from_secs(seconds)),
            )
            .await
    } else {
        proxy.add_backend(&spec.name, transport).await
    };

    result.map_err(|error| anyhow::anyhow!(error))
}

/// Build the only externally served HTTP surface.
///
/// The upstream router contains both MCP and `/admin/*`. Nesting it under
/// `/mcp` gives us a stable MCP endpoint, while explicit deny routes prevent
/// the upstream management plane from being reachable through that listener.
pub fn gateway_router(proxy: Proxy) -> Router {
    let readiness_proxy = proxy.mcp_proxy().clone();
    let expected_backends = readiness_proxy.backend_count();
    let (upstream, _sessions) = proxy.into_router();
    let upstream = upstream.layer(middleware::from_fn(block_upstream_admin));
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route(
            "/readyz",
            get(move || {
                let proxy = readiness_proxy.clone();
                async move { backend_readiness(proxy, expected_backends).await }
            }),
        )
        .nest("/mcp", upstream)
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

pub async fn serve(proxy: Proxy, host: &str, port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    tracing::info!(listen = %listener.local_addr()?, mcp_path = "/mcp", "gateway ready");
    axum::serve(listener, gateway_router(proxy))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received");
        })
        .await?;
    Ok(())
}
