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

pub use crate::config::constants::{
    MAX_ARGUMENT_SIZE_BYTES, MAX_BACKEND_TIMEOUT_SECONDS, SCHEMA_VERSION,
};
use crate::config::model::{BackendEntry, GatewayConfig, NorthboundOAuthConfig};

/// Namespace prefixes reserved for gateway control planes. Backends may
/// never claim them.
pub const RESERVED_PREFIXES: [&str; 3] = ["proxy_", "lomway_", "lmg_"];
/// Allowed prefix charset, documented as part of the namespace policy.
pub const ALLOWED_PREFIX_CHARSET: &str = "a-z 0-9 '_' '-' (must end with '_')";
/// Allowed backend id charset, documented as part of the namespace policy.
pub const ALLOWED_ID_CHARSET: &str = "a-z 0-9 '_' '-' (must start with a letter or digit)";
/// Every `bearer_token_env` name starts with this prefix, so a configuration
/// can never point a backend at an unrelated secret already in the
/// environment (for example a cloud API key).
pub const BACKEND_TOKEN_ENV_PREFIX: &str = "LOMWAY_BACKEND_";

/// Validate a public configuration end to end: schema version, listener,
/// policy flags, and every backend entry (including id/prefix uniqueness).
///
/// The backend list may be empty, hold one entry, or hold many: the public
/// schema aggregates 0..N backends and imposes no population minimum. Every
/// entry that is present is fully validated.
pub fn validate(config: &GatewayConfig) -> Result<()> {
    validate_schema_version(config.schema_version)?;
    ensure!(
        (1..=300).contains(&config.monitor.interval_seconds),
        "monitor.interval_seconds must be between 1 and 300"
    );
    ensure!(
        (1..=1440).contains(&config.monitor.deferred_idle_minutes),
        "monitor.deferred_idle_minutes must be between 1 and 1440"
    );
    validate_server(&config.server.host, config.server.port)?;
    if let Some(oauth) = &config.server.oauth {
        validate_northbound_oauth(oauth)?;
    }
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
        validate_backend_credentials(backend)?;
    }
    Ok(())
}

/// A credential is either referenced and resolved, or absent. A reference
/// that was never resolved fails closed instead of connecting without it,
/// and a resolved credential without a reference cannot exist.
fn validate_backend_credentials(backend: &BackendEntry) -> Result<()> {
    match (&backend.bearer_token_env, &backend.bearer_token) {
        (Some(name), Some(_)) => validate_bearer_token_env_name(&backend.id, name),
        (Some(name), None) => {
            validate_bearer_token_env_name(&backend.id, name)?;
            anyhow::bail!(
                "backend '{}' requires a bearer token from environment variable {name}, but none was resolved; the gateway fails closed instead of connecting without credentials",
                backend.id
            )
        }
        (None, Some(_)) => anyhow::bail!(
            "backend '{}' carries a bearer token without bearer_token_env; credentials can only come from a named environment variable",
            backend.id
        ),
        (None, None) => Ok(()),
    }
}

/// `bearer_token_env` holds a variable *name*: `LOMWAY_BACKEND_` followed by
/// uppercase letters, digits, and `_`, at most 128 characters. The rejected
/// value is never echoed, because a mistaken configuration may hold the
/// token itself.
pub(crate) fn validate_bearer_token_env_name(id: &str, name: &str) -> Result<()> {
    let suffix = name.strip_prefix(BACKEND_TOKEN_ENV_PREFIX).unwrap_or("");
    ensure!(
        name.len() <= 128
            && !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "backend '{id}' bearer_token_env must be the NAME of an environment variable matching {BACKEND_TOKEN_ENV_PREFIX}[A-Z0-9_]+ (at most 128 characters); the configured value is not shown. Never put the token itself in the configuration file"
    );
    Ok(())
}

fn validate_northbound_oauth(oauth: &NorthboundOAuthConfig) -> Result<()> {
    let resource: axum::http::Uri = oauth
        .resource_url
        .parse()
        .context("server.oauth.resource_url is not a valid URI")?;
    ensure!(
        resource.scheme_str() == Some("https")
            && resource.authority().is_some()
            && resource.path() == "/mcp"
            && resource.query().is_none(),
        "server.oauth.resource_url must be an exact https://<host>/mcp URL"
    );

    const LOOPBACK_PREFIX: &str = "http://127.0.0.1:";
    let Some(rest) = oauth.introspection_url.strip_prefix(LOOPBACK_PREFIX) else {
        anyhow::bail!(
            "server.oauth.introspection_url must be loopback http://127.0.0.1:<port>/oauth/introspect"
        );
    };
    let Some((port, path)) = rest.split_once('/') else {
        anyhow::bail!(
            "server.oauth.introspection_url must be loopback http://127.0.0.1:<port>/oauth/introspect"
        );
    };
    ensure!(
        port.parse::<u16>().is_ok_and(|port| port > 0) && path == "oauth/introspect",
        "server.oauth.introspection_url must be loopback http://127.0.0.1:<port>/oauth/introspect"
    );

    ensure!(
        !oauth.required_scope.is_empty()
            && oauth.required_scope.bytes().all(|byte| {
                byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)
            }),
        "server.oauth.required_scope must be one valid OAuth scope token"
    );
    Ok(())
}

/// Only schema version 1 exists today; newer or older files fail with the
/// exact value so users know what to change.
fn validate_schema_version(schema_version: u32) -> Result<()> {
    ensure!(
        schema_version == SCHEMA_VERSION,
        "unsupported schema_version {schema_version}: this release supports only {SCHEMA_VERSION}"
    );
    Ok(())
}

/// The listener is loopback-only in v1.
fn validate_server(host: &str, port: u16) -> Result<()> {
    ensure!(
        host == "127.0.0.1",
        "server.host must be exactly 127.0.0.1; non-loopback listeners require a future security profile"
    );
    ensure!(port > 0, "server.port must be a positive TCP port");
    Ok(())
}

/// Every policy escape hatch fails closed in v1.
fn validate_policy_flags(config: &GatewayConfig) -> Result<()> {
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
        for other in &config.backends[index + 1..] {
            ensure!(
                other.prefix != backend.prefix,
                "duplicate backend prefix '{}': prefixes must be unique",
                backend.prefix
            );
            ensure!(
                !other.prefix.starts_with(&backend.prefix)
                    && !backend.prefix.starts_with(&other.prefix),
                "overlapping backend prefixes {:?} and {:?}: one prefix must not contain another because tool ownership would be ambiguous",
                backend.prefix,
                other.prefix
            );
        }
    }
    Ok(())
}

/// Backend ids stay stable and readable: lowercase, digits, `_` and `-`,
/// starting with a letter or digit. Ambiguous spellings are rejected instead
/// of being silently normalized.
pub(crate) fn validate_backend_id(id: &str) -> Result<()> {
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
pub(crate) fn validate_backend_prefix(prefix: &str) -> Result<()> {
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

/// Backend URLs must be loopback Streamable HTTP MCP endpoints rooted at
/// `/mcp`, for example `http://127.0.0.1:<port>/mcp` or a backend-defined
/// scoped endpoint such as `http://127.0.0.1:<port>/mcp/<scope>/`.
pub(crate) fn validate_backend_url(url: &str) -> Result<()> {
    ensure!(
        is_loopback_mcp_url(url),
        "backend URL must be a loopback http://127.0.0.1:<port>/mcp or /mcp/... endpoint, got {url:?}"
    );
    Ok(())
}

/// Shared strict URL rule for both the public model and the legacy adapter.
pub(crate) fn is_loopback_mcp_url(url: &str) -> bool {
    loopback_mcp_port(url).is_some()
}

/// Return the loopback MCP port when the URL satisfies the backend endpoint
/// policy. Paths may be exactly `/mcp` or scoped beneath `/mcp/`.
pub(crate) fn loopback_mcp_port(url: &str) -> Option<u16> {
    let uri: axum::http::Uri = url.parse().ok()?;
    let authority = uri.authority()?;
    let port = authority.port_u16()?;
    let path = uri.path();
    (uri.scheme_str() == Some("http")
        && authority.host() == "127.0.0.1"
        && port > 0
        && uri.query().is_none()
        && (path == "/mcp" || path.starts_with("/mcp/")))
    .then_some(port)
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
            exposure: Default::default(),
            bearer_token_env: None,
            bearer_token: None,
        }
    }

    fn config(backends: Vec<BackendEntry>) -> GatewayConfig {
        GatewayConfig {
            schema_version: SCHEMA_VERSION,
            server: Default::default(),
            observability: Default::default(),
            policy: Default::default(),
            monitor: Default::default(),
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

        validate_backend_url("http://127.0.0.1:8001/mcp/personal/").expect("scoped MCP endpoint");

        let error =
            validate_backend_url("http://127.0.0.1:8001/mcpish").expect_err("lookalike path");
        assert!(error.to_string().contains("loopback"));

        let error = validate_backend_url("http://127.0.0.1:8001/mcp?bank=personal")
            .expect_err("query-bearing endpoint");
        assert!(error.to_string().contains("loopback"));

        let error = validate_backend_url("http://127.0.0.1:not-a-port/mcp").expect_err("bad port");
        assert!(error.to_string().contains("loopback"));
    }

    #[test]
    fn validates_northbound_oauth_urls_and_scope() {
        let valid = NorthboundOAuthConfig {
            resource_url: "https://example.test/mcp".to_string(),
            introspection_url: "http://127.0.0.1:7677/oauth/introspect".to_string(),
            required_scope: "devspace".to_string(),
        };
        validate_northbound_oauth(&valid).expect("valid oauth config");

        let mut invalid = valid.clone();
        invalid.resource_url = "http://example.test/mcp".to_string();
        assert!(validate_northbound_oauth(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.resource_url = "https://example.test/other".to_string();
        assert!(validate_northbound_oauth(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.introspection_url = "http://192.168.1.2:7677/oauth/introspect".to_string();
        assert!(validate_northbound_oauth(&invalid).is_err());

        let mut invalid = valid;
        invalid.required_scope = "two scopes".to_string();
        assert!(validate_northbound_oauth(&invalid).is_err());
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
    fn rejects_overlapping_prefixes_that_make_tool_ownership_ambiguous() {
        let error = validate(&config(vec![
            entry("a", "a_", "http://127.0.0.1:19001/mcp"),
            entry("ab", "a_b_", "http://127.0.0.1:19002/mcp"),
        ]))
        .expect_err("prefix-of-prefix namespaces must fail closed");
        assert!(error.to_string().contains("overlapping backend prefixes"));
    }

    #[test]
    fn backend_credentials_must_be_referenced_and_resolved() {
        let token =
            || crate::config::model::BackendToken::new("t".repeat(40)).expect("valid token");
        let mut referenced = entry("a", "a_", "http://127.0.0.1:18701/mcp");
        referenced.bearer_token_env = Some("LOMWAY_BACKEND_A_TOKEN".to_string());

        let error = validate(&config(vec![referenced.clone()])).expect_err("unresolved reference");
        let message = error.to_string();
        assert!(message.contains("LOMWAY_BACKEND_A_TOKEN"), "{message}");
        assert!(message.contains("fails closed"), "{message}");

        referenced.bearer_token = Some(token());
        validate(&config(vec![referenced])).expect("resolved reference");

        let mut orphan = entry("a", "a_", "http://127.0.0.1:18701/mcp");
        orphan.bearer_token = Some(token());
        assert!(validate(&config(vec![orphan])).is_err());
    }

    #[test]
    fn bearer_token_env_accepts_only_prefixed_variable_names_and_never_echoes() {
        for good in ["LOMWAY_BACKEND_CODING_TOOLS_TOKEN", "LOMWAY_BACKEND_X1"] {
            validate_bearer_token_env_name("a", good).expect(good);
        }
        let pasted_token = "eyJhbGciOiJIUzI1NiJ9.c2VjcmV0LXRva2Vu.c2lnbmF0dXJl";
        for bad in [
            "",
            "LOMWAY_BACKEND_",
            "OPENAI_API_KEY",
            "lomway_backend_lower",
            "LOMWAY_BACKEND_${OTHER}",
            "LOMWAY_BACKEND_A-B",
            pasted_token,
        ] {
            let error = validate_bearer_token_env_name("a", bad).expect_err(bad);
            let message = error.to_string();
            assert!(message.contains("not shown"), "{message}");
            if !bad.is_empty() && bad != "LOMWAY_BACKEND_" {
                assert!(!message.contains(bad), "{message}");
            }
        }
        let long = format!("LOMWAY_BACKEND_{}", "A".repeat(120));
        assert!(validate_bearer_token_env_name("a", &long).is_err());
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
