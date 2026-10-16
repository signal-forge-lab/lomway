//! Cached deferred-backend tool manifests.
//!
//! Search must never start or reconnect a deferred backend, so deferred tool
//! definitions are served from the last manifest the gateway observed: at
//! startup (from the already-connected proxy catalog) or after an explicit
//! `lomway_describe_tool`/`lomway_call_tool` activation. The manifest is kept
//! in memory and, when a state directory is configured, persisted as JSON so
//! a restarted gateway can still answer searches for backends that are idle.
//!
//! The file lives in the per-user state directory, never in the repository
//! or the deployment's `runtime/` directory.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower_mcp::ToolDefinition;

const MANIFEST_SCHEMA_VERSION: u32 = 1;
/// Environment override for the gateway's per-user state directory.
pub const STATE_DIR_ENV_VAR: &str = "LOMWAY_STATE_DIR";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BackendManifest {
    pub(super) seen_at: String,
    pub(super) tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    schema_version: u32,
    backends: BTreeMap<String, BackendManifest>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ManifestStore {
    path: Option<Arc<PathBuf>>,
    inner: Arc<Mutex<BTreeMap<String, BackendManifest>>>,
}

impl ManifestStore {
    /// In-memory only. Used when no state directory is configured.
    pub(super) fn in_memory() -> Self {
        Self::default()
    }

    /// Load (or start) the manifest file at `path`. A missing or unreadable
    /// file starts empty: every deferred backend is then `unverified`.
    pub(super) fn open(path: PathBuf) -> Self {
        let backends = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ManifestFile>(&bytes) {
                Ok(file) if file.schema_version == MANIFEST_SCHEMA_VERSION => file.backends,
                Ok(_) | Err(_) => {
                    tracing::warn!("ignoring unreadable deferred tool manifest cache");
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Self {
            path: Some(Arc::new(path)),
            inner: Arc::new(Mutex::new(backends)),
        }
    }

    pub(super) fn get(&self, backend: &str) -> Option<BackendManifest> {
        self.inner
            .lock()
            .expect("manifest lock poisoned")
            .get(backend)
            .cloned()
    }

    /// Record the tools a backend exposed just now and persist best-effort.
    pub(super) fn observe(&self, backend: &str, tools: Vec<ToolDefinition>) {
        if tools.is_empty() {
            return;
        }
        let snapshot = {
            let mut inner = self.inner.lock().expect("manifest lock poisoned");
            inner.insert(
                backend.to_string(),
                BackendManifest {
                    seen_at: rfc3339_utc(SystemTime::now()),
                    tools,
                },
            );
            inner.clone()
        };
        if let Some(path) = &self.path
            && let Err(error) = persist(path, snapshot)
        {
            tracing::warn!(%error, "could not persist deferred tool manifest cache");
        }
    }
}

fn persist(path: &Path, backends: BTreeMap<String, BackendManifest>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(&ManifestFile {
        schema_version: MANIFEST_SCHEMA_VERSION,
        backends,
    })
    .map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, body)?;
    std::fs::rename(&temporary, path)
}

/// Default per-user state directory: `LOMWAY_STATE_DIR`, otherwise the
/// platform's local application-data/state location.
pub fn default_state_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(STATE_DIR_ENV_VAR).filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join("Lomway").join("state"));
    }
    if cfg!(target_os = "macos") {
        return std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Lomway")
                .join("state")
        });
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .map(|dir| dir.join("lomway"))
}

/// One manifest file per gateway identity (listener port plus deferred
/// backend ids/prefixes), so side-by-side gateways never share a cache.
pub(super) fn manifest_path(state_dir: &Path, port: u16, deferred: &[(String, String)]) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(port.to_string().as_bytes());
    for (id, prefix) in deferred {
        hasher.update(b"|");
        hasher.update(id.as_bytes());
        hasher.update(b":");
        hasher.update(prefix.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    state_dir.join(format!("deferred-manifest-{}.json", &digest[..16]))
}

/// RFC 3339 UTC timestamp without an extra date/time dependency.
pub(super) fn rfc3339_utc(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let day_of_year = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn tool(name: &str) -> ToolDefinition {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "description": "cached",
            "inputSchema": {"type": "object"}
        }))
        .expect("tool definition")
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339_utc(UNIX_EPOCH + Duration::from_secs(1_791_936_000)),
            "2026-10-14T00:00:00Z"
        );
        assert_eq!(
            rfc3339_utc(UNIX_EPOCH + Duration::from_secs(951_782_400 + 3_723)),
            "2000-02-29T01:02:03Z"
        );
    }

    #[test]
    fn manifest_round_trips_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = manifest_path(dir.path(), 17777, &[("browser".into(), "browser_".into())]);
        assert!(path.starts_with(dir.path()));
        let store = ManifestStore::open(path.clone());
        assert!(store.get("browser").is_none());
        store.observe("browser", vec![tool("browser_navigate")]);
        let reopened = ManifestStore::open(path);
        let manifest = reopened.get("browser").expect("persisted manifest");
        assert_eq!(manifest.tools[0].name, "browser_navigate");
        assert!(manifest.seen_at.ends_with('Z'));
    }

    #[test]
    fn corrupt_cache_starts_unverified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deferred-manifest-test.json");
        std::fs::write(&path, b"not json").expect("write");
        assert!(ManifestStore::open(path).get("browser").is_none());
    }

    #[test]
    fn manifest_identity_differs_per_gateway() {
        let dir = Path::new("state");
        let deferred = [("browser".to_string(), "browser_".to_string())];
        assert_ne!(
            manifest_path(dir, 17777, &deferred),
            manifest_path(dir, 17778, &deferred)
        );
    }
}
