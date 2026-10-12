//! Backend transport recovery for long-lived gateway processes.
//!
//! The monitor never retries a client tool call. It only rebuilds an MCP
//! backend transport after the backend itself has disappeared/recovered or
//! after its MCP health has remained unhealthy across confirmation probes.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Result, anyhow};
use mcp_proxy::ProxyConfig;
use tokio::{sync::Mutex as AsyncMutex, time::Instant};
use tower::timeout::TimeoutLayer;
use tower_mcp::{client::HttpClientTransport, proxy::McpProxy};

const RECONNECT_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const RECONNECT_HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const RECONNECT_STABLE_PROBES: u8 = 2;
const RECONNECT_UNHEALTHY_PROBES: u8 = 2;
const MAX_DOWN_BACKOFF: Duration = Duration::from_secs(60);
const RECOVERY_TICK: Duration = Duration::from_secs(1);

use crate::config::model::MonitorConfig;

#[derive(Clone)]
pub(super) struct BackendReconnectSpec {
    name: String,
    url: String,
    port: u16,
    bearer_token: Option<String>,
    timeout_seconds: Option<u64>,
    deferred: bool,
}

#[derive(Clone)]
struct BackendReconnectState {
    observed_down: bool,
    stable_up_probes: u8,
    unhealthy_probes: u8,
    reconnect_pending: bool,
    ready: bool,
    next_probe: Instant,
    down_backoff: Duration,
    last_used: Option<Instant>,
    revision: u64,
}

impl BackendReconnectState {
    fn new(ready: bool) -> Self {
        Self {
            observed_down: !ready,
            stable_up_probes: 0,
            unhealthy_probes: 0,
            reconnect_pending: false,
            ready,
            next_probe: Instant::now(),
            down_backoff: RECOVERY_TICK,
            last_used: None,
            revision: 0,
        }
    }

    fn active(&self, deferred: bool, now: Instant, idle: Duration) -> bool {
        !deferred
            || self
                .last_used
                .is_some_and(|last| now.duration_since(last) < idle)
    }

    fn mark_down(&mut self, now: Instant) {
        self.ready = false;
        self.observed_down = true;
        self.stable_up_probes = 0;
        self.unhealthy_probes = 0;
        self.reconnect_pending = false;
        self.next_probe = now + self.down_backoff;
        self.down_backoff = (self.down_backoff * 2).min(MAX_DOWN_BACKOFF);
    }
}

/// Shared monitor snapshot. /readyz only reads this cache, never probes a backend.
#[derive(Clone)]
pub struct MonitorHandle {
    states: Arc<Mutex<HashMap<String, BackendReconnectState>>>,
    specs: Arc<Vec<BackendReconnectSpec>>,
    settings: MonitorConfig,
    proxy: McpProxy,
    reconnect_lock: Arc<AsyncMutex<()>>,
}

impl MonitorHandle {
    fn commit_observation(&self, name: &str, state: BackendReconnectState) {
        let mut states = self.states.lock().expect("monitor lock poisoned");
        if states
            .get(name)
            .is_some_and(|current| current.revision == state.revision)
        {
            states.insert(name.to_string(), state);
        }
    }
    pub fn ready(&self) -> bool {
        let states = self.states.lock().expect("monitor lock poisoned");
        self.specs
            .iter()
            .filter(|spec| !spec.deferred)
            .all(|spec| states.get(&spec.name).is_some_and(|state| state.ready))
    }

    pub fn snapshot_for_proxy(proxy: McpProxy) -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
            specs: Arc::new(Vec::new()),
            settings: MonitorConfig::default(),
            proxy,
            reconnect_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    /// Explicit deferred use. Refresh a previously-idle transport before
    /// discovery/dispatch; never retry the *user's* tool invocation.
    pub async fn activate(&self, name: &str) -> Result<()> {
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.name == name && spec.deferred)
            .ok_or_else(|| anyhow!("unknown deferred backend"))?;
        let _guard = self.reconnect_lock.lock().await;
        let now = Instant::now();
        let idle = Duration::from_secs(self.settings.deferred_idle_minutes * 60);
        let refresh = {
            let mut states = self.states.lock().expect("monitor lock poisoned");
            let state = states
                .get_mut(name)
                .ok_or_else(|| anyhow!("missing backend monitor state"))?;
            let returning_from_idle = !state.active(true, now, idle);
            // An idle deferred connection may have stale MCP session state
            // even when a backend has returned on the same TCP port.
            // Rebuild it at the next explicit use, never on the idle timer.
            let refresh = returning_from_idle || !state.ready || state.reconnect_pending;
            state.last_used = Some(now);
            state.revision = state.revision.wrapping_add(1);
            refresh
        };
        if refresh && !port_reachable(spec.port).await {
            let mut states = self.states.lock().expect("monitor lock poisoned");
            states
                .get_mut(name)
                .expect("known backend")
                .mark_down(Instant::now());
            return Err(anyhow!(
                "deferred backend '{name}' is unreachable; tool call not attempted"
            ));
        }
        if refresh {
            if let Err(error) = replace_transport(&self.proxy, spec).await {
                let mut states = self.states.lock().expect("monitor lock poisoned");
                states
                    .get_mut(name)
                    .expect("known backend")
                    .mark_down(Instant::now());
                return Err(anyhow!(
                    "deferred backend '{name}' failed to establish an MCP transport: {error}"
                ));
            }
            let mut states = self.states.lock().expect("monitor lock poisoned");
            let state = states.get_mut(name).expect("known backend");
            let revision = state.revision.wrapping_add(1);
            *state = BackendReconnectState::new(true);
            state.revision = revision;
            state.last_used = Some(Instant::now());
            state.next_probe = Instant::now() + Duration::from_secs(self.settings.interval_seconds);
        }
        Ok(())
    }

    pub fn transport_failure(&self, name: &str) {
        let mut states = self.states.lock().expect("monitor lock poisoned");
        if let Some(state) = states.get_mut(name) {
            state.revision = state.revision.wrapping_add(1);
            state.ready = false;
            state.unhealthy_probes = state.unhealthy_probes.saturating_add(1);
            if state.unhealthy_probes == 1 {
                tracing::warn!(backend = %name, "backend MCP health is unhealthy; waiting for confirmation");
            }
            if state.unhealthy_probes >= RECONNECT_UNHEALTHY_PROBES {
                state.reconnect_pending = true;
            }
            state.next_probe = Instant::now();
        }
    }
}

fn loopback_port(url: &str) -> Option<u16> {
    crate::config::validate::loopback_mcp_port(url)
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
                deferred: backend.hide_tools.as_slice() == ["*"],
            })
        })
        .collect()
}

pub(super) fn spawn(
    proxy: McpProxy,
    specs: Vec<BackendReconnectSpec>,
    settings: MonitorConfig,
) -> MonitorHandle {
    let active = proxy.backend_namespaces();
    let states = specs
        .iter()
        .map(|spec| {
            (
                spec.name.clone(),
                BackendReconnectState::new(active.contains(&spec.name)),
            )
        })
        .collect();
    let handle = MonitorHandle {
        states: Arc::new(Mutex::new(states)),
        specs: Arc::new(specs),
        settings,
        proxy,
        reconnect_lock: Arc::new(AsyncMutex::new(())),
    };
    if !handle.specs.is_empty() {
        let cloned = handle.clone();
        tokio::spawn(async move {
            monitor(cloned).await;
        });
    }
    handle
}

async fn monitor(handle: MonitorHandle) {
    loop {
        tokio::time::sleep(RECOVERY_TICK).await;
        let now = Instant::now();
        for spec in handle.specs.iter() {
            let state = {
                let states = handle.states.lock().expect("monitor lock poisoned");
                states.get(&spec.name).cloned()
            };
            let Some(mut state) = state else {
                continue;
            };
            let idle = Duration::from_secs(handle.settings.deferred_idle_minutes * 60);
            if !state.active(spec.deferred, now, idle) || now < state.next_probe {
                continue;
            }
            let reachable = port_reachable(spec.port).await;

            if !reachable {
                if state.ready {
                    tracing::warn!(backend = %spec.name, port = spec.port, "backend became unreachable; waiting for recovery");
                }
                state.mark_down(now);
                handle.commit_observation(&spec.name, state);
                continue;
            }

            if state.observed_down {
                state.stable_up_probes = state.stable_up_probes.saturating_add(1);
                if state.stable_up_probes >= RECONNECT_STABLE_PROBES {
                    state.reconnect_pending = true;
                }
            } else if state.unhealthy_probes > 0 && !state.reconnect_pending {
                // Only inspect MCP after a real failed tool transport. The
                // pinned tower-mcp ping is unsupported on 2026-07-28, so a
                // negative result ALONE never invalidates a healthy backend.
                let health =
                    tokio::time::timeout(RECONNECT_HEALTH_TIMEOUT, handle.proxy.health_check())
                        .await;
                let healthy = matches!(
                    health,
                    Ok(ref statuses) if statuses.iter().any(|item| {
                        item.namespace == spec.name && item.healthy
                    })
                );
                if healthy {
                    state.unhealthy_probes = 0;
                } else {
                    state.unhealthy_probes = state.unhealthy_probes.saturating_add(1);
                    if state.unhealthy_probes >= RECONNECT_UNHEALTHY_PROBES {
                        state.reconnect_pending = true;
                    }
                }
            }

            if !state.reconnect_pending {
                state.next_probe = now
                    + if state.observed_down {
                        RECOVERY_TICK // Two stable observations before reconnecting.
                    } else {
                        Duration::from_secs(handle.settings.interval_seconds)
                    };
                handle.commit_observation(&spec.name, state);
                continue;
            }

            let _guard = handle.reconnect_lock.lock().await;
            match replace_transport(&handle.proxy, spec).await {
                Ok(()) => {
                    let reason = if state.observed_down {
                        "backend transport reconnected after restart"
                    } else {
                        "backend transport reconnected after MCP health failure"
                    };
                    tracing::info!(backend = %spec.name, "{reason}");
                    let last_used = state.last_used;
                    state = BackendReconnectState::new(true);
                    state.last_used = last_used;
                    state.next_probe =
                        Instant::now() + Duration::from_secs(handle.settings.interval_seconds);
                }
                Err(error) => {
                    tracing::warn!(backend = %spec.name, error = %error, "backend transport reconnect failed; will retry");
                    state.mark_down(Instant::now());
                }
            }
            handle.commit_observation(&spec.name, state);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_capped() {
        let mut state = BackendReconnectState::new(true);
        let now = Instant::now();
        let mut delays = Vec::new();
        for _ in 0..9 {
            state.mark_down(now);
            delays.push(state.next_probe.duration_since(now).as_secs());
        }
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn deferred_backend_is_not_monitored_until_used_or_after_idle() {
        let mut state = BackendReconnectState::new(true);
        let now = Instant::now();
        let idle = Duration::from_secs(600);
        assert!(!state.active(true, now, idle));
        state.last_used = Some(now);
        assert!(state.active(true, now, idle));
        assert!(!state.active(true, now + idle, idle));
        assert!(state.active(false, now + idle, idle));
    }

    #[test]
    fn down_up_requires_two_stable_probes() {
        let mut state = BackendReconnectState::new(true);
        state.mark_down(Instant::now());
        state.stable_up_probes += 1;
        assert!(state.stable_up_probes < RECONNECT_STABLE_PROBES);
        state.stable_up_probes += 1;
        assert_eq!(state.stable_up_probes, RECONNECT_STABLE_PROBES);
    }
}
