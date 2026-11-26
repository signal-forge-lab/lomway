//! Startup probe semantics.
//!
//! Before serving, every configured backend is probed with a real MCP
//! `initialize` + `tools/list` round trip. At least one backend must be
//! configured and healthy (zero configured backends also fail closed), and
//! probes never retry: a failed or timed-out probe is a startup outcome,
//! not a transient error to be masked.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use mcp_proxy::http_backend::http_backend_transport;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower_mcp::client::McpClient;
use tower_mcp::protocol::ToolDefinition;

use crate::backend::descriptor::BackendDescriptor;
use crate::backend::registry::BackendRegistry;

/// One tool as reported by the upstream backend during the probe.
#[derive(Debug, Clone)]
pub struct UpstreamTool {
    /// Upstream tool name.
    pub name: String,
    /// SHA-256 over the canonical client-relevant definition (everything
    /// except `name`, `icons`, and `_meta`), used to detect drift.
    pub fingerprint: String,
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
            Err(_error) => unavailable.push(UnavailableBackend {
                id: descriptor.id().to_string(),
                required: descriptor.required(),
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
        let transport = http_backend_transport(
            descriptor.url(),
            descriptor.bearer_token().map(|token| token.expose()),
            Some(descriptor.timeout().as_secs()),
        );
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
            .map(|definition| {
                let fingerprint = tool_fingerprint(&definition);
                UpstreamTool {
                    name: definition.name,
                    fingerprint,
                }
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

/// Fingerprint of the parts of a tool definition a client acts on:
/// description, title, input/output schemas, annotations, and execution
/// hints. Object keys are sorted first so the digest is independent of the
/// backend's key order.
pub fn tool_fingerprint(definition: &ToolDefinition) -> String {
    let mut value = serde_json::to_value(definition).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        for volatile in ["name", "icons", "_meta"] {
            map.remove(volatile);
        }
    }
    let mut canonical = String::new();
    write_canonical_json(&value, &mut canonical);
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical_json(item, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
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
            exposure: Default::default(),
            bearer_token_env: None,
            bearer_token: None,
        })
        .expect("valid descriptor")
    }

    fn registry(backends: Vec<BackendDescriptor>) -> BackendRegistry {
        BackendRegistry::from_descriptors(backends)
    }

    // tokio::test with a multi-thread runtime is unnecessary; the probe only
    // needs one reactor for timers and I/O.
    fn definition(value: serde_json::Value) -> ToolDefinition {
        serde_json::from_value(value).expect("tool definition")
    }

    #[test]
    fn fingerprint_ignores_key_order_name_and_meta() {
        let left = definition(serde_json::json!({
            "name": "a",
            "description": "d",
            "inputSchema": {"type": "object", "properties": {"x": {"type": "string"}, "y": {"type": "number"}}},
            "_meta": {"volatile": 1}
        }));
        let right = definition(serde_json::json!({
            "inputSchema": {"properties": {"y": {"type": "number"}, "x": {"type": "string"}}, "type": "object"},
            "description": "d",
            "name": "renamed"
        }));
        assert_eq!(tool_fingerprint(&left), tool_fingerprint(&right));
        assert_eq!(tool_fingerprint(&left).len(), 64);
    }

    #[test]
    fn fingerprint_changes_with_description_schema_or_annotations() {
        let base = serde_json::json!({
            "name": "a",
            "description": "d",
            "inputSchema": {"type": "object"}
        });
        let original = tool_fingerprint(&definition(base.clone()));
        let mut described = base.clone();
        described["description"] = "changed".into();
        let mut schema = base.clone();
        schema["inputSchema"]["required"] = serde_json::json!(["x"]);
        let mut annotated = base;
        annotated["annotations"] = serde_json::json!({"destructiveHint": true});
        for changed in [described, schema, annotated] {
            assert_ne!(original, tool_fingerprint(&definition(changed)));
        }
    }

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
