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
//!
//! Every cached backend manifest is bound to a SHA-256 fingerprint of the
//! settings that decide where the gateway connects for that backend. A
//! manifest recorded for a different endpoint is never served: the backend
//! is `unverified` until it is observed again. Only the hash is persisted;
//! endpoint values never reach the file name, the file body, or logs.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower_mcp::ToolDefinition;

/// Version 2 adds the per-backend endpoint fingerprint. Version 1 files have
/// no fingerprint and are ignored (every backend starts `unverified`).
const MANIFEST_SCHEMA_VERSION: u32 = 2;
const ENDPOINT_FINGERPRINT_DOMAIN: &[u8] = b"lomway-deferred-endpoint-v1";
/// Environment override for the gateway's per-user state directory.
pub const STATE_DIR_ENV_VAR: &str = "LOMWAY_STATE_DIR";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BackendManifest {
    /// SHA-256 of the endpoint settings this manifest was observed from.
    pub(super) endpoint_sha256: String,
    pub(super) seen_at: String,
    pub(super) tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    schema_version: u32,
    backends: BTreeMap<String, BackendManifest>,
}

/// SHA-256 (hex) of the settings that decide where a backend is reached.
/// The public configuration reaches every backend over Streamable HTTP, so
/// the transport and resolved URL fully determine the endpoint.
pub(super) fn endpoint_fingerprint(transport: &str, url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ENDPOINT_FINGERPRINT_DOMAIN);
    for (key, value) in [("transport", transport), ("url", url)] {
        hasher.update(b"\n");
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, Default)]
pub(super) struct ManifestStore {
    path: Option<Arc<PathBuf>>,
    /// Current endpoint fingerprint per backend id.
    fingerprints: Arc<BTreeMap<String, String>>,
    inner: Arc<Mutex<BTreeMap<String, BackendManifest>>>,
}

impl ManifestStore {
    /// In-memory only. Used when no state directory is configured.
    pub(super) fn in_memory(fingerprints: BTreeMap<String, String>) -> Self {
        Self {
            fingerprints: Arc::new(fingerprints),
            ..Self::default()
        }
    }

    /// Load (or start) the manifest file at `path`. A missing or unreadable
    /// file starts empty: every deferred backend is then `unverified`. A
    /// cached backend whose endpoint fingerprint no longer matches the
    /// current configuration is dropped and starts `unverified` as well.
    pub(super) fn open(path: PathBuf, fingerprints: BTreeMap<String, String>) -> Self {
        let mut backends = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ManifestFile>(&bytes) {
                Ok(file) if file.schema_version == MANIFEST_SCHEMA_VERSION => file.backends,
                Ok(_) | Err(_) => {
                    tracing::warn!("ignoring unreadable deferred tool manifest cache");
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        backends.retain(|backend, manifest| {
            let current = fingerprints.get(backend) == Some(&manifest.endpoint_sha256);
            if !current {
                tracing::info!(
                    backend = %backend,
                    "ignoring cached deferred tool manifest: backend endpoint changed or was removed"
                );
            }
            current
        });
        Self {
            path: Some(Arc::new(path)),
            fingerprints: Arc::new(fingerprints),
            inner: Arc::new(Mutex::new(backends)),
        }
    }

    /// The cached manifest for `backend`, only when it was observed from
    /// the endpoint the gateway is configured to use now.
    pub(super) fn get(&self, backend: &str) -> Option<BackendManifest> {
        let expected = self.fingerprints.get(backend)?;
        self.inner
            .lock()
            .expect("manifest lock poisoned")
            .get(backend)
            .filter(|manifest| &manifest.endpoint_sha256 == expected)
            .cloned()
    }

    /// Record the tools a backend exposed just now and persist best-effort.
    pub(super) fn observe(&self, backend: &str, tools: Vec<ToolDefinition>) {
        if tools.is_empty() {
            return;
        }
        let Some(endpoint_sha256) = self.fingerprints.get(backend).cloned() else {
            return;
        };
        let snapshot = {
            let mut inner = self.inner.lock().expect("manifest lock poisoned");
            inner.insert(
                backend.to_string(),
                BackendManifest {
                    endpoint_sha256,
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

    fn fingerprints(url: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("browser".to_string(), endpoint_fingerprint("http", url))])
    }

    #[test]
    fn manifest_round_trips_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = manifest_path(dir.path(), 17777, &[("browser".into(), "browser_".into())]);
        assert!(path.starts_with(dir.path()));
        let url = "http://127.0.0.1:18002/mcp";
        let store = ManifestStore::open(path.clone(), fingerprints(url));
        assert!(store.get("browser").is_none());
        store.observe("browser", vec![tool("browser_navigate")]);
        let reopened = ManifestStore::open(path, fingerprints(url));
        let manifest = reopened.get("browser").expect("persisted manifest");
        assert_eq!(manifest.tools[0].name, "browser_navigate");
        assert!(manifest.seen_at.ends_with('Z'));
    }

    #[test]
    fn corrupt_cache_starts_unverified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deferred-manifest-test.json");
        std::fs::write(&path, b"not json").expect("write");
        assert!(
            ManifestStore::open(path, fingerprints("http://127.0.0.1:1/mcp"))
                .get("browser")
                .is_none()
        );
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

    #[test]
    fn same_endpoint_settings_reuse_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deferred-manifest-test.json");
        let url = "http://127.0.0.1:18002/mcp";
        ManifestStore::open(path.clone(), fingerprints(url))
            .observe("browser", vec![tool("browser_navigate")]);
        let reopened = ManifestStore::open(path, fingerprints(url));
        assert_eq!(
            reopened.get("browser").expect("reused").tools[0].name,
            "browser_navigate"
        );
    }

    #[test]
    fn changed_endpoint_settings_invalidate_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deferred-manifest-test.json");
        let old = "http://127.0.0.1:18002/mcp";
        let new = "http://127.0.0.1:18009/mcp/other/";
        ManifestStore::open(path.clone(), fingerprints(old))
            .observe("browser", vec![tool("browser_navigate")]);
        // Same id, prefix and listener port, different endpoint: unverified.
        let changed = ManifestStore::open(path.clone(), fingerprints(new));
        assert!(changed.get("browser").is_none());
        // The stale entry is not carried forward when the cache is rewritten.
        changed.observe("browser", vec![tool("browser_click")]);
        let body = std::fs::read_to_string(&path).expect("cache file");
        assert!(!body.contains("browser_navigate"));
        // Only the hash is persisted, never the endpoint value itself.
        assert!(!body.contains("127.0.0.1"));
        assert!(!body.contains("18009"));
        assert!(body.contains(&endpoint_fingerprint("http", new)));
        assert!(!path.to_string_lossy().contains("18009"));
        // An in-memory store applies the same binding.
        let memory = ManifestStore::in_memory(fingerprints(new));
        memory.observe("browser", vec![tool("browser_click")]);
        assert!(memory.get("browser").is_some());
    }

    #[test]
    fn version_one_cache_without_fingerprints_is_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deferred-manifest-test.json");
        std::fs::write(
            &path,
            br#"{"schema_version":1,"backends":{"browser":{"seen_at":"2026-10-16T00:00:00Z","tools":[]}}}"#,
        )
        .expect("write");
        assert!(
            ManifestStore::open(path, fingerprints("http://127.0.0.1:1/mcp"))
                .get("browser")
                .is_none()
        );
    }

    #[test]
    fn endpoint_fingerprint_is_sha256_hex_and_input_sensitive() {
        let a = endpoint_fingerprint("http", "http://127.0.0.1:1/mcp");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(a, endpoint_fingerprint("http", "http://127.0.0.1:1/mcp"));
        assert_ne!(a, endpoint_fingerprint("http", "http://127.0.0.1:2/mcp"));
        assert_ne!(a, endpoint_fingerprint("stdio", "http://127.0.0.1:1/mcp"));
    }
}
