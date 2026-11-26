//! `lomway catalog`: a read-only snapshot of the final tool catalog.
//!
//! Each configured backend is probed exactly once (`initialize` +
//! `tools/list`, never retried, never `tools/call`). The result lists the
//! final `<prefix><tool>` names a client sees, split by exposure, with a
//! definition fingerprint per tool so a later run can be diffed against a
//! saved snapshot before a restart or deployment.
//!
//! Snapshots never contain backend URLs, credentials, descriptions, or
//! schemas: only ids, prefixes, exposure, availability, final names,
//! upstream names, and SHA-256 fingerprints.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::backend::probe::{ProbeReport, ProbedBackend, probe_backend};
use crate::backend::registry::BackendRegistry;
use crate::config::model::{BackendExposure, GatewayConfig};
use crate::namespace::collision::plan_tools;
use crate::namespace::public_tool_name;

/// Snapshot format identifier; bump on incompatible changes.
pub const SNAPSHOT_FORMAT: &str = "lomway.catalog.v1";

/// A complete catalog snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    /// Always [`SNAPSHOT_FORMAT`].
    pub format: String,
    /// Backends in configuration order.
    pub backends: Vec<BackendCatalog>,
}

/// Whether the backend answered its single probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendStatus {
    Available,
    Unavailable,
}

/// One backend's part of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendCatalog {
    pub id: String,
    pub prefix: String,
    pub exposure: BackendExposure,
    pub required: bool,
    pub status: BackendStatus,
    /// Sorted by final name; empty when unavailable.
    pub tools: Vec<CatalogTool>,
}

/// One tool as a client would address it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogTool {
    /// Final `<prefix><upstream>` name.
    pub name: String,
    /// Name reported by the backend.
    pub upstream: String,
    /// SHA-256 of the canonical definition (see `tool_fingerprint`).
    pub fingerprint: String,
}

impl CatalogSnapshot {
    /// Final names of direct tools (listed in `tools/list`).
    pub fn direct_tool_count(&self) -> usize {
        self.count(BackendExposure::Direct)
    }

    /// Final names of deferred tools (reachable via the meta-tools).
    pub fn deferred_tool_count(&self) -> usize {
        self.count(BackendExposure::Deferred)
    }

    fn count(&self, exposure: BackendExposure) -> usize {
        self.backends
            .iter()
            .filter(|backend| backend.exposure == exposure)
            .map(|backend| backend.tools.len())
            .sum()
    }

    /// Keep only the named backends; unknown ids are an error.
    pub fn retain_backends(&mut self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            ensure!(
                self.backends.iter().any(|backend| &backend.id == id),
                "unknown backend id '{id}' (see `lomway list-backends`)"
            );
        }
        self.backends.retain(|backend| ids.contains(&backend.id));
        Ok(())
    }

    /// Read a snapshot previously written by [`write_snapshot`].
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read catalog snapshot {}", path.display()))?;
        let snapshot: Self = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a Lomway catalog snapshot", path.display()))?;
        ensure!(
            snapshot.format == SNAPSHOT_FORMAT,
            "unsupported catalog snapshot format '{}' in {} (expected '{SNAPSHOT_FORMAT}')",
            snapshot.format,
            path.display()
        );
        Ok(snapshot)
    }
}

/// Probe every configured backend once and build the catalog.
///
/// Unlike `serve`, an unavailable backend (required or optional) does not
/// abort the command: it is reported as `unavailable` so the operator sees
/// the whole picture. A final-name collision among available backends is
/// still an error, exactly as at startup.
pub async fn collect(config: &GatewayConfig) -> Result<CatalogSnapshot> {
    let registry = BackendRegistry::from_config(config)?;
    let mut healthy = Vec::new();
    let mut backends = Vec::new();
    for descriptor in registry.iter() {
        let entry = config
            .backends
            .iter()
            .find(|entry| entry.id == descriptor.id())
            .context("registry backend missing from configuration")?;
        let (status, tools) = match probe_backend(descriptor).await {
            Ok(upstream) => {
                let mut tools: Vec<CatalogTool> = upstream
                    .iter()
                    .map(|tool| CatalogTool {
                        name: public_tool_name(descriptor.prefix(), &tool.name),
                        upstream: tool.name.clone(),
                        fingerprint: tool.fingerprint.clone(),
                    })
                    .collect();
                tools.sort_by(|left, right| left.name.cmp(&right.name));
                healthy.push(ProbedBackend {
                    descriptor: descriptor.clone(),
                    tools: upstream,
                });
                (BackendStatus::Available, tools)
            }
            Err(_) => (BackendStatus::Unavailable, Vec::new()),
        };
        backends.push(BackendCatalog {
            id: entry.id.clone(),
            prefix: entry.prefix.clone(),
            exposure: entry.exposure,
            required: entry.required,
            status,
            tools,
        });
    }
    plan_tools(&ProbeReport {
        healthy,
        unavailable: Vec::new(),
    })?;
    Ok(CatalogSnapshot {
        format: SNAPSHOT_FORMAT.to_string(),
        backends,
    })
}

/// Write a snapshot as pretty JSON to a new file; never overwrites.
pub fn write_snapshot(snapshot: &CatalogSnapshot, path: &Path) -> Result<()> {
    let mut text = serde_json::to_string_pretty(snapshot)?;
    text.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "refusing to write catalog snapshot {} (it must not already exist)",
                path.display()
            )
        })?;
    std::io::Write::write_all(&mut file, text.as_bytes())
        .with_context(|| format!("failed to write catalog snapshot {}", path.display()))
}

/// Human-readable catalog summary.
pub fn render_text(snapshot: &CatalogSnapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<20} {:<20} {:<9} {:<12} TOOLS",
        "BACKEND", "PREFIX", "EXPOSURE", "STATUS"
    );
    for backend in &snapshot.backends {
        let tools = match backend.status {
            BackendStatus::Available => backend.tools.len().to_string(),
            BackendStatus::Unavailable => "-".to_string(),
        };
        let _ = writeln!(
            out,
            "{:<20} {:<20} {:<9} {:<12} {tools}",
            backend.id,
            backend.prefix,
            exposure_label(backend.exposure),
            status_label(backend.status),
        );
    }
    let _ = writeln!(
        out,
        "Direct tools (listed in tools/list): {}",
        snapshot.direct_tool_count()
    );
    let _ = writeln!(
        out,
        "Deferred tools (via lomway_search_tools / lomway_describe_tool / lomway_call_tool): {}",
        snapshot.deferred_tool_count()
    );
    let unavailable: Vec<String> = snapshot
        .backends
        .iter()
        .filter(|backend| backend.status == BackendStatus::Unavailable)
        .map(|backend| {
            if backend.required {
                format!("{} (required: `serve` would fail closed)", backend.id)
            } else {
                format!("{} (optional)", backend.id)
            }
        })
        .collect();
    if !unavailable.is_empty() {
        let _ = writeln!(out, "Unavailable: {}", unavailable.join(", "));
    }
    out
}

/// One final tool name per line, for allowlists such as `pat.ps1 -Tools`.
pub fn render_names(snapshot: &CatalogSnapshot) -> String {
    snapshot
        .backends
        .iter()
        .flat_map(|backend| backend.tools.iter())
        .map(|tool| format!("{}\n", tool.name))
        .collect()
}

/// Differences between a saved snapshot and the current catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogDiff {
    pub backends_added: Vec<String>,
    pub backends_removed: Vec<String>,
    /// `(id, old, new)`.
    pub exposure_changed: Vec<(String, BackendExposure, BackendExposure)>,
    /// Backends unavailable in either snapshot; their tools are not compared.
    pub not_compared: Vec<String>,
    pub tools_added: Vec<String>,
    pub tools_removed: Vec<String>,
    /// Same final name, different definition fingerprint.
    pub tools_changed: Vec<String>,
}

impl CatalogDiff {
    /// No difference that affects what a client sees or calls.
    pub fn is_empty(&self) -> bool {
        self.backends_added.is_empty()
            && self.backends_removed.is_empty()
            && self.exposure_changed.is_empty()
            && self.tools_added.is_empty()
            && self.tools_removed.is_empty()
            && self.tools_changed.is_empty()
    }
}

/// Compare `old` against `new`.
///
/// Tools of a backend that is unavailable on either side are skipped (and
/// listed in `not_compared`) so a backend outage is never misreported as
/// removed tools.
pub fn diff(old: &CatalogSnapshot, new: &CatalogSnapshot) -> CatalogDiff {
    let old_backends: BTreeMap<&str, &BackendCatalog> = old
        .backends
        .iter()
        .map(|backend| (backend.id.as_str(), backend))
        .collect();
    let new_backends: BTreeMap<&str, &BackendCatalog> = new
        .backends
        .iter()
        .map(|backend| (backend.id.as_str(), backend))
        .collect();
    let mut result = CatalogDiff::default();
    let mut skipped: BTreeSet<&str> = BTreeSet::new();

    for (id, backend) in &new_backends {
        match old_backends.get(id) {
            None => result.backends_added.push((*id).to_string()),
            Some(previous) => {
                if previous.exposure != backend.exposure {
                    result.exposure_changed.push((
                        (*id).to_string(),
                        previous.exposure,
                        backend.exposure,
                    ));
                }
                if previous.status == BackendStatus::Unavailable
                    || backend.status == BackendStatus::Unavailable
                {
                    skipped.insert(id);
                }
            }
        }
    }
    for id in old_backends.keys() {
        if !new_backends.contains_key(id) {
            result.backends_removed.push((*id).to_string());
        }
    }

    let tools = |snapshot: &CatalogSnapshot| -> BTreeMap<String, String> {
        snapshot
            .backends
            .iter()
            .filter(|backend| !skipped.contains(backend.id.as_str()))
            .flat_map(|backend| backend.tools.iter())
            .map(|tool| (tool.name.clone(), tool.fingerprint.clone()))
            .collect()
    };
    let old_tools = tools(old);
    let new_tools = tools(new);
    for (name, fingerprint) in &new_tools {
        match old_tools.get(name) {
            None => result.tools_added.push(name.clone()),
            Some(previous) if previous != fingerprint => result.tools_changed.push(name.clone()),
            Some(_) => {}
        }
    }
    for name in old_tools.keys() {
        if !new_tools.contains_key(name) {
            result.tools_removed.push(name.clone());
        }
    }
    result.not_compared = skipped.into_iter().map(str::to_string).collect();
    result
}

/// Human-readable diff report.
pub fn render_diff(diff: &CatalogDiff, baseline: &Path) -> String {
    let mut out = format!("Catalog diff against {}\n", baseline.display());
    if diff.is_empty() {
        out.push_str("No changes to backends, exposure, tool names, or tool definitions.\n");
    }
    let mut section = |label: &str, items: &[String]| {
        if !items.is_empty() {
            let _ = writeln!(out, "{label} ({}):", items.len());
            for item in items {
                let _ = writeln!(out, "  {item}");
            }
        }
    };
    section("Backends added", &diff.backends_added);
    section("Backends removed", &diff.backends_removed);
    let exposure: Vec<String> = diff
        .exposure_changed
        .iter()
        .map(|(id, old, new)| format!("{id}: {} -> {}", exposure_label(*old), exposure_label(*new)))
        .collect();
    section("Exposure changed", &exposure);
    section("Tools added", &diff.tools_added);
    section("Tools removed", &diff.tools_removed);
    section(
        "Tools changed (definition fingerprint)",
        &diff.tools_changed,
    );
    section(
        "Not compared (backend unavailable in one snapshot)",
        &diff.not_compared,
    );
    out
}

fn exposure_label(exposure: BackendExposure) -> &'static str {
    match exposure {
        BackendExposure::Direct => "direct",
        BackendExposure::Deferred => "deferred",
    }
}

fn status_label(status: BackendStatus) -> &'static str {
    match status {
        BackendStatus::Available => "available",
        BackendStatus::Unavailable => "unavailable",
    }
}

/// Fail with the diff summary when `fail_on_change` is set and the diff is
/// not empty.
pub fn enforce_no_change(diff: &CatalogDiff, fail_on_change: bool) -> Result<()> {
    if fail_on_change && !diff.is_empty() {
        bail!(
            "catalog changed: {} tool(s) added, {} removed, {} changed, {} backend(s) added, {} removed, {} exposure change(s)",
            diff.tools_added.len(),
            diff.tools_removed.len(),
            diff.tools_changed.len(),
            diff.backends_added.len(),
            diff.backends_removed.len(),
            diff.exposure_changed.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, fingerprint: &str) -> CatalogTool {
        CatalogTool {
            name: format!("a_{name}"),
            upstream: name.to_string(),
            fingerprint: fingerprint.to_string(),
        }
    }

    fn backend(id: &str, status: BackendStatus, tools: Vec<CatalogTool>) -> BackendCatalog {
        BackendCatalog {
            id: id.to_string(),
            prefix: format!("{id}_"),
            exposure: BackendExposure::Direct,
            required: false,
            status,
            tools,
        }
    }

    fn snapshot(backends: Vec<BackendCatalog>) -> CatalogSnapshot {
        CatalogSnapshot {
            format: SNAPSHOT_FORMAT.to_string(),
            backends,
        }
    }

    #[test]
    fn identical_snapshots_have_an_empty_diff() {
        let old = snapshot(vec![backend(
            "a",
            BackendStatus::Available,
            vec![tool("x", "1")],
        )]);
        let result = diff(&old, &old.clone());
        assert!(result.is_empty(), "{result:?}");
        assert!(enforce_no_change(&result, true).is_ok());
    }

    #[test]
    fn added_removed_and_changed_tools_are_reported() {
        let old = snapshot(vec![backend(
            "a",
            BackendStatus::Available,
            vec![tool("keep", "1"), tool("gone", "2"), tool("edit", "3")],
        )]);
        let new = snapshot(vec![backend(
            "a",
            BackendStatus::Available,
            vec![tool("keep", "1"), tool("edit", "4"), tool("fresh", "5")],
        )]);
        let result = diff(&old, &new);
        assert_eq!(result.tools_added, ["a_fresh"]);
        assert_eq!(result.tools_removed, ["a_gone"]);
        assert_eq!(result.tools_changed, ["a_edit"]);
        let error = enforce_no_change(&result, true).expect_err("changed");
        assert!(error.to_string().contains("1 tool(s) added"), "{error}");
        assert!(enforce_no_change(&result, false).is_ok());
    }

    #[test]
    fn an_unavailable_backend_is_not_reported_as_removed_tools() {
        let old = snapshot(vec![backend(
            "a",
            BackendStatus::Available,
            vec![tool("x", "1")],
        )]);
        let new = snapshot(vec![backend("a", BackendStatus::Unavailable, Vec::new())]);
        let result = diff(&old, &new);
        assert!(result.tools_removed.is_empty(), "{result:?}");
        assert_eq!(result.not_compared, ["a"]);
        assert!(result.is_empty(), "an outage alone is not a catalog change");
    }

    #[test]
    fn backend_and_exposure_changes_are_reported() {
        let old = snapshot(vec![
            backend("a", BackendStatus::Available, Vec::new()),
            backend("b", BackendStatus::Available, Vec::new()),
        ]);
        let mut moved = backend("a", BackendStatus::Available, Vec::new());
        moved.exposure = BackendExposure::Deferred;
        let new = snapshot(vec![
            moved,
            backend("c", BackendStatus::Available, Vec::new()),
        ]);
        let result = diff(&old, &new);
        assert_eq!(result.backends_added, ["c"]);
        assert_eq!(result.backends_removed, ["b"]);
        assert_eq!(
            result.exposure_changed,
            [(
                "a".to_string(),
                BackendExposure::Direct,
                BackendExposure::Deferred
            )]
        );
        let report = render_diff(&result, Path::new("old.json"));
        assert!(report.contains("a: direct -> deferred"), "{report}");
    }

    #[test]
    fn text_and_names_rendering() {
        let mut deferred = backend("d", BackendStatus::Available, vec![tool("y", "2")]);
        deferred.exposure = BackendExposure::Deferred;
        let mut down = backend("r", BackendStatus::Unavailable, Vec::new());
        down.required = true;
        let current = snapshot(vec![
            backend("a", BackendStatus::Available, vec![tool("x", "1")]),
            deferred,
            down,
        ]);
        let text = render_text(&current);
        assert!(
            text.contains("Direct tools (listed in tools/list): 1"),
            "{text}"
        );
        assert!(text.contains("Deferred tools"), "{text}");
        assert!(
            text.contains("r (required: `serve` would fail closed)"),
            "{text}"
        );
        assert_eq!(render_names(&current), "a_x\na_y\n");
    }

    #[test]
    fn backend_filter_rejects_unknown_ids() {
        let mut current = snapshot(vec![
            backend("a", BackendStatus::Available, Vec::new()),
            backend("b", BackendStatus::Available, Vec::new()),
        ]);
        assert!(current.retain_backends(&["zzz".to_string()]).is_err());
        current
            .retain_backends(&["b".to_string()])
            .expect("known id");
        assert_eq!(current.backends.len(), 1);
        assert_eq!(current.backends[0].id, "b");
    }

    #[test]
    fn snapshots_round_trip_and_never_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("catalog.json");
        let current = snapshot(vec![backend(
            "a",
            BackendStatus::Available,
            vec![tool("x", "1")],
        )]);
        write_snapshot(&current, &path).expect("first write");
        assert_eq!(CatalogSnapshot::load(&path).expect("load"), current);
        let error = write_snapshot(&current, &path).expect_err("must not overwrite");
        assert!(
            error.to_string().contains("must not already exist"),
            "{error}"
        );

        let foreign = dir.path().join("foreign.json");
        std::fs::write(&foreign, r#"{"format":"other","backends":[]}"#).expect("write");
        assert!(CatalogSnapshot::load(&foreign).is_err());
    }
}
