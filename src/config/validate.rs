//! Policy validation for the public configuration model.
//!
//! Validation is fail-closed: unsupported or unsafe values are hard errors,
//! never warnings, and nothing is silently normalized. The v1 safety posture
//! is identical to the pre-generalization deployment posture.
//!
//! One deliberate generalization: the public schema aggregates 0..N
//! backends, so an empty backend list is a valid configuration. Every entry
//! that is present is still fully validated.

use anyhow::{Context, Result, ensure};

use crate::config::model::GatewayConfig;

/// The only supported configuration schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// Default (and maximum) tool-call argument payload size in bytes: 1 MiB.
pub const MAX_ARGUMENT_SIZE_BYTES: usize = 1024 * 1024;
/// Maximum backend request/probe timeout in seconds.
pub const MAX_BACKEND_TIMEOUT_SECONDS: u64 = 300;
/// Namespace prefixes reserved for gateway control planes. Backends may
/// never claim them.
pub const RESERVED_PREFIXES: [&str; 3] = ["proxy_", "lomway_", "lmg_"];
/// Allowed prefix charset, documented as part of the namespace policy.
pub const ALLOWED_PREFIX_CHARSET: &str = "a-z 0-9 '_' '-' (must end with '_')";
/// Allowed backend id charset, documented as part of the namespace policy.
pub const ALLOWED_ID_CHARSET: &str = "a-z 0-9 '_' '-' (must start with a letter or digit)";

/// Validate a public configuration end to end: schema version, listener,
/// policy flags, and every backend entry (including id/prefix uniqueness).
///
/// The backend list may be empty, hold one entry, or hold many: the public
/// schema aggregates 0..N backends and imposes no population minimum. Every
/// entry that is present is fully validated.
pub fn validate(config: &GatewayConfig) -> Result<()> {
    validate_schema_version(config.schema_version)?;
    validate_server(&config.server.host, config.server.port)?;
    validate_policy_flags(config)?;
    validate_unique_ids(config)?;
    validate_unique_prefixes(config)?;
    for backend in &config.backends {
        validate_backend_id(&backend.id)?;
        validate_backend_prefix(&backend.prefix)?;
        validate_backend_url(&backend.url)
            .with_context(|| format!("backend '{}' has an invalid URL", backend.id))?;
        ensure!(
            (1..=MAX_BACKEND_TIMEOUT_SECONDS).contains(&backend.timeout_seconds),
            "backend '{}' timeout_seconds must be between 1 and {MAX_BACKEND_TIMEOUT_SECONDS}",
            backend.id
        );
    }
    Ok(())
}

/// Only schema version 1 exists today; newer or older files fail with the
/// exact value so users know what to change.
pub fn validate_schema_version(schema_version: u32) -> Result<()> {
    ensure!(
        schema_version == SCHEMA_VERSION,
        "unsupported schema_version {schema_version}: this release supports only {SCHEMA_VERSION}"
    );
    Ok(())
}

/// The listener is loopback-only in v1.
pub fn validate_server(host: &str, port: u16) -> Result<()> {
    ensure!(
        host == "127.0.0.1",
        "server.host must be exactly 127.0.0.1; non-loopback listeners require a future security profile"
    );
    ensure!(port > 0, "server.port must be a positive TCP port");
    Ok(())
}

/// Every policy escape hatch fails closed in v1.
pub fn validate_policy_flags(config: &GatewayConfig) -> Result<()> {
    ensure!(
        !config.policy.allow_non_loopback_listener,
        "policy.allow_non_loopback_listener is not permitted in this release"
    );
    ensure!(
        !config.policy.allow_non_loopback_backends,
        "policy.allow_non_loopback_backends is not permitted in this release"
    );
    ensure!(
        !config.policy.hot_reload,
        "policy.hot_reload is disabled in this release because gateway policy is validated only at startup"
    );
    ensure!(
        (1..=MAX_ARGUMENT_SIZE_BYTES).contains(&config.policy.max_argument_size_bytes),
        "policy.max_argument_size_bytes must be between 1 and {MAX_ARGUMENT_SIZE_BYTES} bytes"
    );
    Ok(())
}

fn validate_unique_ids(config: &GatewayConfig) -> Result<()> {
    for (index, backend) in config.backends.iter().enumerate() {
        ensure!(
            config.backends[index + 1..]
                .iter()
                .all(|other| other.id != backend.id),
            "duplicate backend id '{}': backend ids must be unique",
            backend.id
        );
    }
    Ok(())
}

fn validate_unique_prefixes(config: &GatewayConfig) -> Result<()> {
    for (index, backend) in config.backends.iter().enumerate() {
        ensure!(
            config.backends[index + 1..]
                .iter()
                .all(|other| other.prefix != backend.prefix),
            "duplicate backend prefix '{}': prefixes must be unique",
            backend.prefix
        );
    }
    Ok(())
}

/// Backend ids stay stable and readable: lowercase, digits, `_` and `-`,
/// starting with a letter or digit. Ambiguous spellings are rejected instead
/// of being silently normalized.
pub fn validate_backend_id(id: &str) -> Result<()> {
    ensure!(!id.is_empty(), "backend id must not be empty");
    ensure!(
        id.len() <= 64,
        "backend id must be at most 64 characters: {id:?}"
    );
    let mut chars = id.chars();
    ensure!(
        chars
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit()),
        "backend id {id:?} must start with a lowercase letter or digit ({ALLOWED_ID_CHARSET}); ambiguous ids are rejected, not normalized"
    );
    ensure!(
        id.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
        "backend id {id:?} may only contain lowercase letters, digits, '_' and '-' ({ALLOWED_ID_CHARSET}); ambiguous ids are rejected, not normalized"
    );
    Ok(())
}

/// Prefixes are explicit: non-empty, lowercase, and terminated by the `_`
/// namespace separator. Missing separators and reserved prefixes fail.
pub fn validate_backend_prefix(prefix: &str) -> Result<()> {
    ensure!(
        !prefix.is_empty(),
        "backend prefix must not be empty in this release"
    );
    ensure!(
        prefix.len() <= 64,
        "backend prefix must be at most 64 characters: {prefix:?}"
    );
    ensure!(
        prefix.ends_with('_'),
        "backend prefix {prefix:?} must end with the '_' namespace separator; ambiguous prefixes are rejected, not normalized"
    );
    let stem = &prefix[..prefix.len() - 1];
    ensure!(
        !stem.is_empty(),
        "backend prefix {prefix:?} must contain at least one character before '_'"
    );
    ensure!(
        stem.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
        "backend prefix {prefix:?} may only contain lowercase letters, digits, '_' and '-' ({ALLOWED_PREFIX_CHARSET}); ambiguous prefixes are rejected, not normalized"
    );
    ensure!(
        !RESERVED_PREFIXES.contains(&prefix),
        "backend prefix {prefix:?} is reserved for gateway control planes; reserved prefixes are rejected: {RESERVED_PREFIXES:?}"
    );
    Ok(())
}

/// Backend URLs must be exact loopback Streamable HTTP MCP endpoints:
/// `http://127.0.0.1:<port>/mcp`.
pub fn validate_backend_url(url: &str) -> Result<()> {
    ensure!(
        is_exact_loopback_mcp_url(url),
        "backend URL must be a loopback http://127.0.0.1:<port>/mcp endpoint, got {url:?}"
    );
    Ok(())
}

/// Shared strict URL rule for both the public model and the legacy adapter.
pub(crate) fn is_exact_loopback_mcp_url(url: &str) -> bool {
    const LOOPBACK_PREFIX: &str = "http://127.0.0.1:";
    let Some(rest) = url.strip_prefix(LOOPBACK_PREFIX) else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };
    path == "mcp" && port.parse::<u16>().is_ok_and(|port| port > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::BackendEntry;

    fn entry(id: &str, prefix: &str, url: &str) -> BackendEntry {
        BackendEntry {
            id: id.to_string(),
            prefix: prefix.to_string(),
            url: url.to_string(),
            required: true,
            timeout_seconds: 30,
        }
    }

    fn config(backends: Vec<BackendEntry>) -> GatewayConfig {
        GatewayConfig {
            schema_version: SCHEMA_VERSION,
            server: Default::default(),
            observability: Default::default(),
            policy: Default::default(),
            backends,
        }
    }

    #[test]
    fn accepts_valid_public_config() {
        validate(&config(vec![
            entry("fs", "fs_", "http://127.0.0.1:8001/mcp"),
            entry("browser", "browser_", "http://127.0.0.1:8002/mcp"),
        ]))
        .expect("valid config");
    }

    #[test]
    fn rejects_unsupported_schema_versions() {
        let error = validate_schema_version(2).expect_err("future version");
        assert!(error.to_string().contains("unsupported schema_version 2"));
        let error = validate_schema_version(0).expect_err("zero version");
        assert!(error.to_string().contains("unsupported schema_version 0"));
    }

    #[test]
    fn rejects_non_loopback_listener_and_backends() {
        let error = validate_server("0.0.0.0", 17777).expect_err("wildcard host");
        assert!(error.to_string().contains("127.0.0.1"));

        let error = validate_backend_url("https://example.invalid/mcp").expect_err("remote URL");
        assert!(error.to_string().contains("loopback"));

        let error =
            validate_backend_url("http://127.0.0.1:8001/nested/mcp").expect_err("nested path");
        assert!(error.to_string().contains("loopback"));

        let error = validate_backend_url("http://127.0.0.1:not-a-port/mcp").expect_err("bad port");
        assert!(error.to_string().contains("loopback"));
    }

    #[test]
    fn rejects_escaped_policy_flags() {
        let mut raw = config(vec![entry("a", "a_", "http://127.0.0.1:8001/mcp")]);
        raw.policy.hot_reload = true;
        assert!(validate(&raw).is_err());
        raw.policy.hot_reload = false;
        raw.policy.allow_non_loopback_backends = true;
        assert!(validate(&raw).is_err());
        raw.policy.allow_non_loopback_backends = false;
        raw.policy.max_argument_size_bytes = MAX_ARGUMENT_SIZE_BYTES + 1;
        assert!(validate(&raw).is_err());
        raw.policy.max_argument_size_bytes = 0;
        assert!(validate(&raw).is_err());
    }

    #[test]
    fn accepts_zero_one_and_many_backends() {
        // Zero backends is a valid generalized configuration: the gateway
        // aggregates 0..N backends and no population minimum is implied.
        validate(&config(vec![])).expect("zero backends are valid");

        validate(&config(vec![entry(
            "fs",
            "fs_",
            "http://127.0.0.1:8001/mcp",
        )]))
        .expect("one backend is valid");

        validate(&config(vec![
            entry("fs", "fs_", "http://127.0.0.1:8001/mcp"),
            entry("browser", "browser_", "http://127.0.0.1:8002/mcp"),
            entry("calc", "calc_", "http://127.0.0.1:8003/mcp"),
        ]))
        .expect("many backends are valid");
    }

    #[test]
    fn unrestricted_population_still_validates_every_present_entry() {
        // Population is unrestricted, but every entry that *is* present must
        // still pass every per-backend validator.
        let error = validate(&config(vec![entry(
            "Bad",
            "bad_",
            "http://127.0.0.1:8001/mcp",
        )]))
        .expect_err("ambiguous id still rejected");
        assert!(error.to_string().contains("not normalized"));
    }

    #[test]
    fn rejects_duplicate_ids_and_prefixes() {
        let error = validate(&config(vec![
            entry("a", "a_", "http://127.0.0.1:8001/mcp"),
            entry("a", "b_", "http://127.0.0.1:8002/mcp"),
        ]))
        .expect_err("duplicate id");
        assert!(error.to_string().contains("duplicate backend id"));

        let error = validate(&config(vec![
            entry("a", "dup_", "http://127.0.0.1:8001/mcp"),
            entry("b", "dup_", "http://127.0.0.1:8002/mcp"),
        ]))
        .expect_err("duplicate prefix");
        assert!(error.to_string().contains("duplicate backend prefix"));
    }

    #[test]
    fn rejects_ambiguous_ids_and_prefixes_without_normalization() {
        let error = validate_backend_id("MyBackend").expect_err("uppercase id");
        assert!(error.to_string().contains("not normalized"));

        let error = validate_backend_prefix("fs").expect_err("missing separator");
        assert!(error.to_string().contains("'_' namespace separator"));

        let error = validate_backend_prefix("FS_").expect_err("uppercase prefix");
        assert!(error.to_string().contains("not normalized"));

        let error = validate_backend_prefix("proxy_").expect_err("reserved prefix");
        assert!(error.to_string().contains("reserved"));

        let error = validate_backend_prefix("lomway_").expect_err("reserved lomway prefix");
        assert!(error.to_string().contains("reserved"));

        // Preserve the pre-rename reserved prefix as a compatibility guard.
        let error = validate_backend_prefix("lmg_").expect_err("reserved legacy lmg prefix");
        assert!(error.to_string().contains("reserved"));

        let error = validate_backend_prefix("_").expect_err("empty stem");
        assert!(error.to_string().contains("at least one character"));
    }
}
