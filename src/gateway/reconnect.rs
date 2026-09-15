//! Backend transport recovery for long-lived gateway processes.
//!
//! The monitor never retries a client tool call. It only rebuilds an MCP
//! backend transport after the backend itself has disappeared/recovered or
//! after its MCP health has remained unhealthy across confirmation probes.

use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use mcp_proxy::ProxyConfig;
use tower::timeout::TimeoutLayer;
use tower_mcp::{client::HttpClientTransport, proxy::McpProxy};

const LOOPBACK_PREFIX: &str = "http://127.0.0.1:";
const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RECONNECT_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const RECONNECT_HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const RECONNECT_STABLE_PROBES: u8 = 2;
const RECONNECT_UNHEALTHY_PROBES: u8 = 2;

#[derive(Clone)]
pub(super) struct BackendReconnectSpec {
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

fn loopback_port(url: &str) -> Option<u16> {
    let rest = url.strip_prefix(LOOPBACK_PREFIX)?;
    let (port, path) = rest.split_once('/')?;
    if path != "mcp" {
        return None;
    }
    port.parse::<u16>().ok().filter(|port| *port > 0)
}

pub(super) fn specs(config: &ProxyConfig) -> Vec<BackendReconnectSpec> {
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

pub(super) fn spawn(proxy: McpProxy, specs: Vec<BackendReconnectSpec>) {
    let active = proxy.backend_namespaces();
    if specs.is_empty() {
        return;
    }
    tokio::spawn(async move {
        monitor(proxy, specs, active).await;
    });
}

async fn monitor(proxy: McpProxy, specs: Vec<BackendReconnectSpec>, active: Vec<String>) {
    let mut states: HashMap<String, BackendReconnectState> = specs
        .iter()
        .map(|spec| {
            let state = BackendReconnectState {
                observed_down: !active.iter().any(|name| name == &spec.name),
                ..BackendReconnectState::default()
            };
            (spec.name.clone(), state)
        })
        .collect();

    loop {
        tokio::time::sleep(RECONNECT_POLL_INTERVAL).await;
        let health = tokio::time::timeout(RECONNECT_HEALTH_TIMEOUT, proxy.health_check())
            .await
            .ok();

        for spec in &specs {
            let Some(state) = states.get_mut(&spec.name) else {
                tracing::error!(backend = %spec.name, "backend reconnect state is missing; skipping this monitor cycle");
                continue;
            };
            let reachable = port_reachable(spec.port).await;

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

            match replace_transport(&proxy, spec).await {
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

async fn port_reachable(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            RECONNECT_PROBE_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn replace_transport(proxy: &McpProxy, spec: &BackendReconnectSpec) -> Result<()> {
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
