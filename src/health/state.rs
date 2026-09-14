//! Liveness and readiness are distinct concerns.
//!
//! * Liveness (`/healthz`): the process is up. Always `200` while serving.
//! * Readiness (`/readyz`): per-backend health observed by the background
//!   monitor. Readiness is *earned by observation*: it is `503` until every
//!   required backend has been observed healthy and at least one backend is
//!   healthy. Optional backends that are down degrade readiness reporting
//!   but never block the gateway: tracking runs in background tasks, so
//!   gateway startup never waits on a health probe.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::backend::probe::ProbeReport;
use crate::backend::registry::BackendRegistry;

/// Observed health of one backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendHealth {
    /// Whether the latest observation succeeded.
    pub healthy: bool,
    /// Whether an outage of this backend must hold readiness down.
    pub required: bool,
    /// The failure reason of the latest failed observation (never includes
    /// request payloads, URLs, or secrets).
    pub error: Option<String>,
}

/// Immutable per-backend health snapshot served by `/readyz`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealthState {
    backends: BTreeMap<String, BackendHealth>,
}

impl HealthState {
    /// Readiness rule: every required backend is currently observed healthy
    /// and at least one backend is healthy overall. A required failure never
    /// reports ready; optional failures are reported as degraded.
    pub fn ready(&self) -> bool {
        if self.backends.is_empty() {
            return false;
        }
        let required_total = self.backends.values().filter(|h| h.required).count();
        let healthy_required = self
            .backends
            .values()
            .filter(|h| h.required && h.healthy)
            .count();
        let healthy_total = self.backends.values().filter(|h| h.healthy).count();
        healthy_required == required_total && healthy_total > 0
    }

    /// Ids of backends whose latest observation failed, in deterministic
    /// (sorted) order.
    pub fn degraded(&self) -> Vec<String> {
        self.backends
            .iter()
            .filter(|(_, health)| !health.healthy)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// HTTP status for `/readyz`.
    pub fn http_status(&self) -> StatusCode {
        if self.ready() {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        }
    }

    /// JSON body for `/readyz`. Contains only ids, booleans, and counters —
    /// never URLs, payloads, or secrets.
    pub fn json_body(&self) -> String {
        let required_total = self.backends.values().filter(|h| h.required).count();
        let healthy_required = self
            .backends
            .values()
            .filter(|h| h.required && h.healthy)
            .count();
        let healthy_total = self.backends.values().filter(|h| h.healthy).count();
        let per_backend: serde_json::Map<String, Value> = self
            .backends
            .iter()
            .map(|(id, health)| {
                (
                    id.clone(),
                    json!({
                        "healthy": health.healthy,
                        "required": health.required,
                        "error": health.error,
                    }),
                )
            })
            .collect();
        json!({
            "status": if self.ready() { "ready" } else { "not_ready" },
            "backends": {
                "total": self.backends.len(),
                "required": required_total,
                "healthy_required": healthy_required,
                "healthy_total": healthy_total,
            },
            "degraded": self.degraded(),
            "per_backend": Value::Object(per_backend),
        })
        .to_string()
    }
}

/// Shared per-backend health tracker.
///
/// Startup registers the configured backends (unobserved, therefore not
/// ready); the background monitor records one observation per backend per
/// poll. Reads are snapshots, so the serving path never blocks on health
/// checks and startup never waits on monitoring.
#[derive(Debug, Clone, Default)]
pub struct HealthTracker(Arc<RwLock<HealthState>>);

impl HealthTracker {
    /// A tracker that has observed nothing yet; readiness is not earned.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize from the startup probe used by the strict public-config
    /// builder: probed backends are observed healthy, the rest unhealthy.
    pub fn from_startup(registry: &BackendRegistry, report: &ProbeReport) -> Self {
        let tracker = Self::new();
        for backend in registry.iter() {
            tracker.register(backend.id(), backend.required());
        }
        for backend in &report.unavailable {
            tracker.observe(&backend.id, false, Some(backend.error.clone()));
        }
        for backend in &report.healthy {
            tracker.observe(backend.descriptor.id(), true, None);
        }
        tracker
    }

    /// Register a backend as observed-nothing-yet. Idempotent; a backend
    /// that was already tracked keeps its current state.
    pub fn register(&self, id: &str, required: bool) {
        self.0
            .write()
            .expect("health state lock")
            .backends
            .entry(id.to_string())
            .or_insert(BackendHealth {
                healthy: false,
                required,
                error: None,
            });
    }

    /// Record one observation. Unknown backends are inserted as optional.
    /// A healthy observation clears the stored error; an unhealthy one keeps
    /// the previous error when no new reason is given.
    pub fn observe(&self, id: &str, healthy: bool, error: Option<String>) {
        let mut state = self.0.write().expect("health state lock");
        let entry = state
            .backends
            .entry(id.to_string())
            .or_insert(BackendHealth {
                healthy: false,
                required: false,
                error: None,
            });
        entry.healthy = healthy;
        entry.error = if healthy {
            None
        } else {
            error.or_else(|| entry.error.clone())
        };
    }

    /// Current immutable snapshot.
    pub fn snapshot(&self) -> HealthState {
        self.0.read().expect("health state lock").clone()
    }

    /// Whether the current snapshot is ready.
    pub fn ready(&self) -> bool {
        self.snapshot().ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::descriptor::BackendDescriptor;
    use crate::backend::probe::{ProbedBackend, UnavailableBackend, UpstreamTool};
    use crate::config::model::BackendEntry;

    fn descriptor(id: &str, required: bool) -> BackendDescriptor {
        BackendDescriptor::try_from_entry(&BackendEntry {
            id: id.to_string(),
            prefix: format!("{id}_"),
            url: "http://127.0.0.1:19101/mcp".to_string(),
            required,
            timeout_seconds: 5,
        })
        .expect("valid descriptor")
    }

    fn tool(name: &str) -> UpstreamTool {
        UpstreamTool {
            name: name.to_string(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn liveness_and_readiness_are_distinct_concepts() {
        // Liveness has no state: the route exists and always answers 200
        // while the process serves. Readiness must be earned by observation,
        // so an unobserved tracker is never ready.
        let state = HealthTracker::new().snapshot();
        assert!(!state.ready());
        assert_eq!(state.http_status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(state.json_body().contains("not_ready"));
    }

    #[test]
    fn optional_degradation_is_reported_but_ready() {
        let registry = crate::backend::registry::BackendRegistry::from_descriptors(vec![
            descriptor("healthy", true),
            descriptor("optional", false),
        ]);
        let report = ProbeReport {
            healthy: vec![ProbedBackend {
                descriptor: descriptor("healthy", true),
                tools: vec![tool("status")],
            }],
            unavailable: vec![UnavailableBackend {
                id: "optional".to_string(),
                required: false,
                error: "connection refused".to_string(),
            }],
        };
        let state = HealthTracker::from_startup(&registry, &report).snapshot();
        assert!(state.ready(), "optional outage degrades but stays ready");
        assert_eq!(state.degraded(), ["optional".to_string()]);
        assert_eq!(state.http_status(), StatusCode::OK);
        let body = state.json_body();
        assert!(body.contains("\"degraded\":[\"optional\"]"), "{body}");
        let parsed: Value = serde_json::from_str(&body).expect("readiness JSON");
        assert_eq!(parsed["per_backend"]["optional"]["healthy"], false);
        assert_eq!(parsed["per_backend"]["optional"]["required"], false);
    }

    #[test]
    fn required_failure_never_reports_ready() {
        let registry =
            crate::backend::registry::BackendRegistry::from_descriptors(vec![descriptor(
                "needed", true,
            )]);
        let report = ProbeReport {
            healthy: vec![],
            unavailable: vec![UnavailableBackend {
                id: "needed".to_string(),
                required: true,
                error: "connection refused".to_string(),
            }],
        };
        let state = HealthTracker::from_startup(&registry, &report).snapshot();
        assert!(!state.ready(), "required failure must never be ready");
        assert_eq!(state.http_status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn observations_track_each_backend_independently() {
        let tracker = HealthTracker::new();
        tracker.register("required_a", true);
        tracker.register("optional_b", false);
        assert!(!tracker.ready(), "nothing observed yet");

        tracker.observe("required_a", true, None);
        assert!(
            tracker.ready(),
            "an unobserved optional backend must not block readiness once every required backend is healthy"
        );

        tracker.observe("optional_b", false, Some("port unreachable".to_string()));
        assert!(
            tracker.ready(),
            "a degraded optional backend must not block readiness"
        );

        tracker.observe("optional_b", true, None);
        let snapshot = tracker.snapshot();
        assert!(snapshot.ready());
        assert!(snapshot.degraded().is_empty());
        assert_eq!(
            snapshot.backends.get("required_a").map(|h| h.healthy),
            Some(true)
        );

        tracker.observe("required_a", false, Some("port unreachable".to_string()));
        assert!(
            !tracker.ready(),
            "a required failure must flip readiness back down"
        );
        let snapshot = tracker.snapshot();
        assert_eq!(
            snapshot.backends.get("required_a").map(|h| h.error.clone()),
            Some(Some("port unreachable".to_string()))
        );
        assert_eq!(
            snapshot.backends.get("optional_b").map(|h| h.healthy),
            Some(true),
            "per-backend state is independent"
        );
    }

    #[test]
    fn all_backends_unobserved_or_unhealthy_is_never_ready() {
        let tracker = HealthTracker::new();
        tracker.register("a", false);
        tracker.register("b", false);
        assert!(!tracker.ready(), "nothing healthy yet");

        tracker.observe("a", false, Some("port unreachable".to_string()));
        tracker.observe("b", false, Some("port unreachable".to_string()));
        let state = tracker.snapshot();
        assert!(
            !state.ready(),
            "all backends unavailable must never report ready"
        );
        let body = state.json_body();
        assert!(body.contains("\"healthy_total\":0"), "{body}");
        assert!(body.contains("\"degraded\":[\"a\",\"b\"]"), "{body}");
    }
}
