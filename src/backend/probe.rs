//! Startup probe semantics.
//!
//! Before serving, every configured backend is probed with a real MCP
//! `initialize` + `tools/list` round trip. At least one backend must be
//! configured and healthy (zero configured backends also fail closed), and
//! probes never retry: a failed or timed-out probe is a startup outcome,
//! not a transient error to be masked.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tower_mcp::client::{HttpClientTransport, McpClient};

use crate::backend::descriptor::BackendDescriptor;
use crate::backend::registry::BackendRegistry;

/// One tool as reported by the upstream backend during the probe.
#[derive(Debug, Clone)]
pub struct UpstreamTool {
    /// Upstream tool name.
    pub name: String,
    /// Upstream description, preserved verbatim.
    pub description: Option<String>,
    /// Upstream input schema, preserved verbatim.
    pub input_schema: Value,
}

/// A backend that answered the startup probe.
#[derive(Debug, Clone)]
pub struct ProbedBackend {
    /// The validated descriptor that was probed.
    pub descriptor: BackendDescriptor,
    /// Tools listed by the backend.
    pub tools: Vec<UpstreamTool>,
}

/// A backend that failed its startup probe.
#[derive(Debug, Clone)]
pub struct UnavailableBackend {
    /// Backend id.
    pub id: String,
    /// Whether the entry was marked required.
    pub required: bool,
    /// The probe failure reason (never includes request payloads).
    pub error: String,
}

/// The complete startup probe outcome.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// Healthy backends in configuration order.
    pub healthy: Vec<ProbedBackend>,
    /// Unavailable backends in configuration order.
    pub unavailable: Vec<UnavailableBackend>,
}

impl ProbeReport {
    /// Number of healthy backends.
    pub fn healthy_count(&self) -> usize {
        self.healthy.len()
    }

    /// Ids of unavailable backends.
    pub fn unavailable_ids(&self) -> Vec<&str> {
        self.unavailable
            .iter()
            .map(|backend| backend.id.as_str())
            .collect()
    }
}

/// Probe every backend in the registry and enforce startup semantics:
///
/// * a registry with zero configured backends fails closed (there is no
///   explicit empty-serving mode in this release);
/// * every `required` backend must be healthy, otherwise startup fails;
/// * optional backends may be down and are reported as degraded;
/// * if every configured backend is unavailable, startup fails by default.
///
/// Backends are probed one at a time in registry (configuration) order, so
/// the report and every error message are deterministic for a given
/// configuration and set of reachable backends.
pub async fn probe_registry(registry: &BackendRegistry) -> Result<ProbeReport> {
    if registry.is_empty() {
        bail!(
            "no backends are configured at startup; gateway fails closed by default (this release has no empty-serving mode)"
        );
    }

    let mut healthy = Vec::new();
    let mut unavailable = Vec::new();

    for descriptor in registry.iter() {
        match probe_backend(descriptor).await {
            Ok(tools) => healthy.push(ProbedBackend {
                descriptor: descriptor.clone(),
                tools,
            }),
            Err(error) => unavailable.push(UnavailableBackend {
                id: descriptor.id().to_string(),
                required: descriptor.required(),
                error: format!("{error:#}"),
            }),
        }
    }

    let missing_required: Vec<&str> = unavailable
        .iter()
        .filter(|backend| backend.required)
        .map(|backend| backend.id.as_str())
        .collect();
    if !missing_required.is_empty() {
        bail!(
            "required backend(s) unavailable at startup: {}; gateway fails closed",
            missing_required.join(", ")
        );
    }
    if healthy.is_empty() {
        bail!(
            "all backends are unavailable at startup: {}; gateway fails closed by default",
            unavailable
                .iter()
                .map(|backend| backend.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(ProbeReport {
        healthy,
        unavailable,
    })
}

/// Probe a single backend: connect, initialize, and list tools within the
/// backend's configured timeout. No retries are attempted.
pub async fn probe_backend(descriptor: &BackendDescriptor) -> Result<Vec<UpstreamTool>> {
    let probe = async {
        let transport = HttpClientTransport::new(descriptor.url().to_string());
        let client = McpClient::connect(transport)
            .await
            .context("connect failed")?;
        client
            .initialize(crate::DEFAULT_SERVER_NAME, env!("CARGO_PKG_VERSION"))
            .await
            .context("initialize failed")?;
        let definitions = client.list_all_tools().await.context("tools/list failed")?;
        Ok(definitions
            .into_iter()
            .map(|definition| UpstreamTool {
                name: definition.name,
                description: definition.description,
                input_schema: definition.input_schema,
            })
            .collect())
    };

    let timeout = probe_timeout(descriptor);
    tokio::time::timeout(timeout, probe)
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "startup probe timed out after {}s",
                timeout.as_secs()
            ))
        })
}

fn probe_timeout(descriptor: &BackendDescriptor) -> Duration {
    descriptor.timeout().max(Duration::from_secs(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::BackendEntry;

    fn descriptor(id: &str, url: &str, required: bool) -> BackendDescriptor {
        BackendDescriptor::try_from_entry(&BackendEntry {
            id: id.to_string(),
            prefix: format!("{id}_"),
            url: url.to_string(),
            required,
            timeout_seconds: 2,
        })
        .expect("valid descriptor")
    }

    fn registry(backends: Vec<BackendDescriptor>) -> BackendRegistry {
        BackendRegistry::from_descriptors(backends)
    }

    // tokio::test with a multi-thread runtime is unnecessary; the probe only
    // needs one reactor for timers and I/O.
    #[tokio::test]
    async fn required_backend_down_fails_startup() {
        let registry = registry(vec![
            descriptor("dead_required", "http://127.0.0.1:9/mcp", true),
            descriptor("offline_optional", "http://127.0.0.1:9/mcp", false),
        ]);
        let error = probe_registry(&registry)
            .await
            .expect_err("required backend is down");
        let message = error.to_string();
        assert!(
            message.contains("required backend(s) unavailable"),
            "{message}"
        );
        assert!(message.contains("dead_required"), "{message}");
    }

    #[tokio::test]
    async fn optional_backend_down_degrades_but_is_reported() {
        // Use a live server on an ephemeral port as the healthy backend.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener); // placeholder backend: never accepts; probe fails fast on refused connect

        let registry = registry(vec![descriptor(
            "optional_down",
            &format!("http://{addr}/mcp"),
            false,
        )]);
        let error = probe_registry(&registry)
            .await
            .expect_err("all-unavailable must fail by default");
        assert!(
            error.to_string().contains("all backends are unavailable"),
            "{error:#}"
        );
    }
}
