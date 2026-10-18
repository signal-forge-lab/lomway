//! Public configuration schema (`schema_version = 1`).
//!
//! Example:
//!
//! ```toml
//! schema_version = 1
//!
//! [server]
//! host = "127.0.0.1"
//! port = 17777
//!
//! [policy]
//! allow_non_loopback_listener = false
//! allow_non_loopback_backends = false
//! hot_reload = false
//! max_argument_size_bytes = 1048576
//!
//! [[backends]]
//! id = "filesystem"
//! prefix = "fs_"
//! url = "http://127.0.0.1:8001/mcp"
//! required = true
//! ```
//!
//! A backend that requires a bearer token names the environment variable
//! that holds it (`bearer_token_env = "LOMWAY_BACKEND_..."`). The token value
//! itself is never part of the configuration file.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::constants::MAX_ARGUMENT_SIZE_BYTES;

pub const DEFAULT_LISTEN_HOST: &str = "127.0.0.1";
pub const DEFAULT_LISTEN_PORT: u16 = 17777;
pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;

/// How a backend's tools are presented to northbound MCP clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendExposure {
    /// Tools are listed and invoked directly with their normal Lomway prefix.
    #[default]
    Direct,
    /// Tools are hidden from the normal list/direct-call surface and are
    /// available only through Lomway's safe discovery meta-tools.
    Deferred,
}

/// Top-level public configuration. Unknown keys are rejected.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    /// Only schema version 1 is supported in this release.
    pub schema_version: u32,
    /// Northbound listener settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// Safety policy switches. Every escape hatch fails closed in v1.
    #[serde(default)]
    pub policy: PolicyConfig,
    /// Optional log settings for the gateway process itself.
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Low-overhead backend reconnection monitor.
    #[serde(default)]
    pub monitor: MonitorConfig,
    /// Aggregated backends. The public schema permits zero, one, or many
    /// entries; every present entry is fully validated.
    #[serde(default)]
    pub backends: Vec<BackendEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorConfig {
    /// TCP probe cadence for a healthy active backend.
    pub interval_seconds: u64,
    /// Stop monitoring deferred backends after this much inactivity.
    pub deferred_idle_minutes: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            interval_seconds: 30,
            deferred_idle_minutes: 10,
        }
    }
}

/// Listener settings.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Bind host. Only `127.0.0.1` is allowed in v1.
    #[serde(default = "default_host")]
    pub host: String,
    /// Bind port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Optional MCP `instructions` text passed through to clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Graceful shutdown timeout in seconds.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_seconds: u64,
    /// Optional northbound OAuth protection for the public MCP endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<NorthboundOAuthConfig>,
}

/// OAuth protected-resource settings for direct remote MCP access.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NorthboundOAuthConfig {
    /// Canonical public MCP resource URL, for example `https://host.example/mcp`.
    pub resource_url: String,
    /// Loopback-only RFC 7662 token introspection endpoint.
    pub introspection_url: String,
    /// Scope required on every accepted access token.
    #[serde(default = "default_required_scope")]
    pub required_scope: String,
}

/// Policy switches. Every non-default value fails closed in v1.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Must remain `false`; non-loopback listeners need a future security
    /// profile and threat model.
    #[serde(default)]
    pub allow_non_loopback_listener: bool,
    /// Must remain `false`; backends must stay on loopback in v1.
    #[serde(default)]
    pub allow_non_loopback_backends: bool,
    /// Must remain `false`; gateway policy is only validated at startup.
    #[serde(default)]
    pub hot_reload: bool,
    /// Upper bound for tool-call argument payloads in bytes.
    #[serde(default = "default_max_argument_size")]
    pub max_argument_size_bytes: usize,
}

/// Gateway process log settings. These never log tool arguments or secrets.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Emit audit lines (method, tool name, duration, status) for MCP calls.
    #[serde(default = "default_audit")]
    pub audit: bool,
    /// `tracing` filter, for example `info` or `lomway=debug`.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Emit structured JSON logs on stderr instead of plain text.
    #[serde(default)]
    pub json_logs: bool,
}

/// One aggregated backend.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendEntry {
    /// Stable identifier, unique across the configuration.
    pub id: String,
    /// Namespace prefix applied to every tool of this backend. Must be
    /// non-empty, end with `_`, and use only the allowed prefix charset.
    pub prefix: String,
    /// Loopback HTTP Streamable MCP endpoint rooted at `/mcp`
    /// (`http://127.0.0.1:<port>/mcp` or `/mcp/<scope>/`).
    /// Supports `${ENV_VAR}` references resolved at load time.
    pub url: String,
    /// Required backends fail startup when unreachable; optional backends
    /// degrade startup instead.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Per-backend request and startup-probe timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Client exposure mode for this backend.
    #[serde(default)]
    pub exposure: BackendExposure,
    /// Name of the environment variable that holds the bearer token sent to
    /// this backend as `Authorization: Bearer <token>`. Only the variable
    /// name is configured; the value is read when the configuration is
    /// loaded and is never serialized, logged, or written back. The name
    /// must start with `LOMWAY_BACKEND_`. An unset or malformed variable
    /// fails the load instead of connecting without credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token_env: Option<String>,
    /// The resolved credential. Filled only by the configuration loader from
    /// `bearer_token_env`; it can never be set from a file.
    #[serde(skip)]
    pub bearer_token: Option<BackendToken>,
}

/// A backend bearer token resolved from the environment.
///
/// The value has no `Display`, no `Serialize`, and a redacting `Debug`, so it
/// cannot reach logs, error messages, `check` output, or files by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct BackendToken(String);

/// Shortest accepted backend token. Generated tokens are 32 random bytes
/// (43 to 64 characters), so anything shorter is treated as a mistake.
pub const MIN_BACKEND_TOKEN_LEN: usize = 32;
/// Longest accepted backend token.
pub const MAX_BACKEND_TOKEN_LEN: usize = 4096;

impl BackendToken {
    /// Validate a token value. Errors never include the value.
    ///
    /// Accepted tokens are 32 to 4096 visible ASCII characters (0x21-0x7E),
    /// which also rules out whitespace and header injection.
    pub fn new(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        anyhow::ensure!(
            (MIN_BACKEND_TOKEN_LEN..=MAX_BACKEND_TOKEN_LEN).contains(&value.len())
                && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)),
            "a backend bearer token must be {MIN_BACKEND_TOKEN_LEN} to {MAX_BACKEND_TOKEN_LEN} visible ASCII characters without spaces (the value is not shown)"
        );
        Ok(Self(value))
    }

    /// The raw token, for building the outgoing `Authorization` header only.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackendToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackendToken(<redacted>)")
    }
}

fn default_host() -> String {
    DEFAULT_LISTEN_HOST.to_string()
}

fn default_port() -> u16 {
    DEFAULT_LISTEN_PORT
}

fn default_shutdown_timeout() -> u64 {
    30
}

fn default_required_scope() -> String {
    "devspace".to_string()
}

fn default_max_argument_size() -> usize {
    MAX_ARGUMENT_SIZE_BYTES
}

fn default_audit() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_required() -> bool {
    true
}

fn default_timeout_seconds() -> u64 {
    DEFAULT_BACKEND_TIMEOUT_SECONDS
}

impl Default for BackendEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            prefix: String::new(),
            url: String::new(),
            required: default_required(),
            timeout_seconds: default_timeout_seconds(),
            exposure: BackendExposure::Direct,
            bearer_token_env: None,
            bearer_token: None,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            instructions: None,
            shutdown_timeout_seconds: default_shutdown_timeout(),
            oauth: None,
        }
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            allow_non_loopback_listener: false,
            allow_non_loopback_backends: false,
            hot_reload: false,
            max_argument_size_bytes: default_max_argument_size(),
        }
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            audit: default_audit(),
            log_level: default_log_level(),
            json_logs: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::validate::SCHEMA_VERSION;

    fn parse(text: &str) -> Result<GatewayConfig, toml::de::Error> {
        toml::from_str(text)
    }

    const MINIMAL: &str = r#"
schema_version = 1

[[backends]]
id = "echo"
prefix = "echo_"
url = "http://127.0.0.1:18701/mcp"
"#;

    #[test]
    fn schema_version_one_is_supported() {
        let config = parse(MINIMAL).expect("parse minimal");
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert_eq!(config.monitor.interval_seconds, 30);
        assert_eq!(config.monitor.deferred_idle_minutes, 10);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 17777);
        assert_eq!(
            config.policy.max_argument_size_bytes,
            MAX_ARGUMENT_SIZE_BYTES
        );
    }

    #[test]
    fn monitor_settings_parse_and_reject_unknown_keys() {
        let config = parse(
            "schema_version = 1\n[monitor]\ninterval_seconds = 15\ndeferred_idle_minutes = 8\n",
        )
        .expect("monitor settings");
        assert_eq!(config.monitor.interval_seconds, 15);
        assert_eq!(config.monitor.deferred_idle_minutes, 8);
        assert!(parse("schema_version = 1\n[monitor]\ninterval_millis = 500\n").is_err());
    }

    #[test]
    fn monitor_cadence_is_bounded() {
        for seconds in [0, 301] {
            let config = parse(&format!(
                "schema_version = 1\n[monitor]\ninterval_seconds = {seconds}\n"
            ))
            .expect("parse numeric cadence");
            assert!(crate::config::validate::validate(&config).is_err());
        }
        let config =
            parse("schema_version = 1\n[monitor]\ninterval_seconds = 15\n").expect("valid monitor");
        crate::config::validate::validate(&config).expect("accepted cadence");
    }

    #[test]
    fn zero_backends_parse_and_pass_policy() {
        let config = parse("schema_version = 1").expect("parse zero backends");
        assert!(config.backends.is_empty());
        crate::config::validate::validate(&config)
            .expect("the public schema permits an empty backend list");
    }

    #[test]
    fn one_and_many_backends_parse() {
        let one = parse(MINIMAL).expect("parse one backend");
        assert_eq!(one.backends.len(), 1);
        assert!(one.backends[0].required, "required defaults to true");

        let many = parse(&format!(
            "{MINIMAL}\n[[backends]]\nid = \"calc\"\nprefix = \"calc_\"\nurl = \"http://127.0.0.1:18702/mcp\"\nrequired = false\ntimeout_seconds = 5\n"
        ))
        .expect("parse two backends");
        assert_eq!(many.backends.len(), 2);
        assert!(!many.backends[1].required);
        assert_eq!(many.backends[1].timeout_seconds, 5);
        assert_eq!(many.backends[0].exposure, BackendExposure::Direct);
    }

    #[test]
    fn parses_deferred_backend_exposure() {
        let config = parse(
            r#"
schema_version = 1

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:7691/mcp"
exposure = "deferred"
"#,
        )
        .expect("deferred exposure should be part of the public schema");
        assert_eq!(config.backends.len(), 1);
        assert_eq!(config.backends[0].exposure, BackendExposure::Deferred);
    }

    #[test]
    fn unsupported_schema_versions_are_rejected_at_parse() {
        // Version gating is enforced by validation; the model parses the
        // number so the validator can report the exact unsupported value.
        let config = parse("schema_version = 2").expect("parse future schema");
        assert_eq!(config.schema_version, 2);
    }

    #[test]
    fn unknown_keys_are_rejected_strictly() {
        let error = parse("schema_version = 1\nmystery = true").expect_err("unknown top key");
        assert!(error.to_string().contains("unknown field `mystery`"));

        let error = parse("schema_version = 1\n[server]\nhost = \"127.0.0.1\"\nmode = \"fast\"\n")
            .expect_err("unknown server key");
        assert!(error.to_string().contains("unknown field `mode`"));

        let error = parse(
            "schema_version = 1\n[[backends]]\nid = \"a\"\nprefix = \"a_\"\nurl = \"http://127.0.0.1:1/mcp\"\nbearer_token = \"x\"\n",
        )
        .expect_err("unknown backend key");
        assert!(error.to_string().contains("unknown field `bearer_token`"));
    }

    #[test]
    fn bearer_token_env_parses_but_the_token_itself_never_does() {
        let config = parse(
            "schema_version = 1\n[[backends]]\nid = \"a\"\nprefix = \"a_\"\nurl = \"http://127.0.0.1:1/mcp\"\nbearer_token_env = \"LOMWAY_BACKEND_A_TOKEN\"\n",
        )
        .expect("bearer_token_env is part of the public schema");
        assert_eq!(
            config.backends[0].bearer_token_env.as_deref(),
            Some("LOMWAY_BACKEND_A_TOKEN")
        );
        assert!(
            config.backends[0].bearer_token.is_none(),
            "only the loader resolves the credential"
        );
    }

    #[test]
    fn backend_token_is_redacted_and_never_serialized() {
        let secret = "s3cr3t-value-0123456789-abcdefghijklmnop";
        let token = BackendToken::new(secret).expect("valid token");
        assert_eq!(format!("{token:?}"), "BackendToken(<redacted>)");
        let entry = BackendEntry {
            id: "a".to_string(),
            prefix: "a_".to_string(),
            url: "http://127.0.0.1:1/mcp".to_string(),
            bearer_token_env: Some("LOMWAY_BACKEND_A_TOKEN".to_string()),
            bearer_token: Some(token),
            ..Default::default()
        };
        let debug = format!("{entry:?}");
        assert!(!debug.contains(secret), "{debug}");
        let serialized = toml::to_string(&entry).expect("serialize entry");
        assert!(!serialized.contains(secret), "{serialized}");
        assert!(serialized.contains("bearer_token_env"));
        assert!(!serialized.contains("bearer_token ="));
    }

    #[test]
    fn backend_token_values_are_checked_without_echoing_them() {
        for bad in [
            String::new(),
            "short".to_string(),
            format!("{} with space", "x".repeat(40)),
            format!("{}\r\nX-Injected: 1", "x".repeat(40)),
            format!("{}\u{e9}", "x".repeat(40)),
            "x".repeat(MAX_BACKEND_TOKEN_LEN + 1),
        ] {
            let error = BackendToken::new(bad.clone()).expect_err("malformed token");
            let message = error.to_string();
            assert!(message.contains("not shown"), "{message}");
            if !bad.is_empty() {
                assert!(!message.contains(&bad), "{message}");
            }
        }
        BackendToken::new("A".repeat(MIN_BACKEND_TOKEN_LEN)).expect("minimum length");
    }

    #[test]
    fn missing_schema_version_is_rejected() {
        let error =
            parse("[[backends]]\nid = \"a\"\nprefix = \"a_\"\nurl = \"http://127.0.0.1:1/mcp\"\n")
                .expect_err("missing schema_version");
        assert!(error.to_string().contains("schema_version"));
    }

    #[test]
    fn defaults_are_conservative() {
        let config = parse("schema_version = 1").expect("parse");
        assert!(!config.policy.allow_non_loopback_listener);
        assert!(!config.policy.allow_non_loopback_backends);
        assert!(!config.policy.hot_reload);
        assert!(config.observability.audit);
        assert_eq!(config.observability.log_level, "info");
        assert!(!config.observability.json_logs);
        assert_eq!(config.backends.len(), 0);
        assert!(config.server.oauth.is_none());
    }

    #[test]
    fn parses_optional_northbound_oauth() {
        let config = parse(
            r#"
schema_version = 1

[server.oauth]
resource_url = "https://example.test/mcp"
introspection_url = "http://127.0.0.1:7677/oauth/introspect"
"#,
        )
        .expect("parse oauth");
        let oauth = config.server.oauth.expect("oauth config");
        assert_eq!(oauth.resource_url, "https://example.test/mcp");
        assert_eq!(
            oauth.introspection_url,
            "http://127.0.0.1:7677/oauth/introspect"
        );
        assert_eq!(oauth.required_scope, "devspace");
    }
}
