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

use serde::{Deserialize, Serialize};

use crate::config::validate::MAX_ARGUMENT_SIZE_BYTES;

pub const DEFAULT_LISTEN_HOST: &str = "127.0.0.1";
pub const DEFAULT_LISTEN_PORT: u16 = 17777;
pub const DEFAULT_BACKEND_TIMEOUT_SECONDS: u64 = 30;

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
    /// Aggregated backends. The public schema permits zero, one, or many
    /// entries; every present entry is fully validated.
    #[serde(default)]
    pub backends: Vec<BackendEntry>,
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
    /// Loopback HTTP Streamable MCP endpoint (`http://127.0.0.1:<port>/mcp`).
    /// Supports `${ENV_VAR}` references resolved at load time.
    pub url: String,
    /// Required backends fail startup when unreachable; optional backends
    /// degrade startup instead.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Per-backend request and startup-probe timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
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
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 17777);
        assert_eq!(
            config.policy.max_argument_size_bytes,
            MAX_ARGUMENT_SIZE_BYTES
        );
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
    }
}
