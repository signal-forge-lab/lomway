//! Migration adapters between the legacy deployment configuration and the
//! public schema.
//!
//! The original six-backend reference deployment remains a first-class citizen: its
//! configuration validates under the original policy and maps losslessly to
//! the public model. Migration never copies private values into public
//! files; it is a pure in-memory transformation.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use mcp_proxy::ProxyConfig;
use mcp_proxy::config::{
    BackendConfig, ListenConfig, ProxySettings, TimeoutConfig, ToolExposure, TransportType,
};

use crate::DEFAULT_SERVER_NAME;
use crate::config::model::{
    BackendEntry, GatewayConfig, ObservabilityConfig, PolicyConfig, ServerConfig,
};
use crate::config::validate::{self, MAX_ARGUMENT_SIZE_BYTES, SCHEMA_VERSION};
use crate::namespace::NAMESPACE_SEPARATOR;

/// Map a legacy deployment configuration to the public schema.
///
/// Callers must have applied the legacy policy validation first (see
/// [`crate::gateway::policy::validate_proxy_policy`]); migration maps the
/// safe subset and preserves the original degrade-on-outage behavior by
/// marking migrated backends `required = false`.
pub fn to_public(legacy: &ProxyConfig) -> Result<GatewayConfig> {
    ensure!(
        legacy.proxy.separator == NAMESPACE_SEPARATOR,
        "legacy separator must be '{NAMESPACE_SEPARATOR}' to map onto the public namespace policy"
    );

    let backends = legacy
        .backends
        .iter()
        .map(|backend| {
            ensure!(
                matches!(backend.transport, TransportType::Http),
                "backend '{}' must use HTTP to migrate to the public schema",
                backend.name
            );
            let url = backend.url.clone().unwrap_or_default();
            validate::validate_backend_url(&url)
                .with_context(|| format!("backend '{}' cannot migrate", backend.name))?;
            Ok(BackendEntry {
                id: backend.name.clone(),
                prefix: format!("{}{NAMESPACE_SEPARATOR}", backend.name),
                url,
                // Preserve today's behavior: an unreachable backend degrades
                // startup instead of failing it.
                required: false,
                timeout_seconds: backend
                    .timeout
                    .as_ref()
                    .map(|timeout| timeout.seconds)
                    .unwrap_or(crate::config::model::DEFAULT_BACKEND_TIMEOUT_SECONDS),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(GatewayConfig {
        schema_version: SCHEMA_VERSION,
        server: ServerConfig {
            host: legacy.proxy.listen.host.clone(),
            port: legacy.proxy.listen.port,
            instructions: legacy.proxy.instructions.clone(),
            shutdown_timeout_seconds: legacy.proxy.shutdown_timeout_seconds,
        },
        policy: PolicyConfig {
            allow_non_loopback_listener: false,
            allow_non_loopback_backends: false,
            hot_reload: false,
            max_argument_size_bytes: legacy
                .security
                .max_argument_size
                .unwrap_or(MAX_ARGUMENT_SIZE_BYTES),
        },
        observability: ObservabilityConfig {
            audit: legacy.observability.audit,
            log_level: legacy.observability.log_level.clone(),
            json_logs: legacy.observability.json_logs,
        },
        backends,
    })
}

/// Construct the legacy `mcp-proxy` configuration implied by a validated
/// public configuration. Only the safe subset is ever emitted; every
/// middleware, rewriting, fan-out, and auth feature stays at its inert
/// default.
pub fn to_legacy(public: &GatewayConfig) -> Result<ProxyConfig> {
    validate::validate(public)?;

    let backends = public
        .backends
        .iter()
        .map(|entry| -> Result<BackendConfig> {
            let name = crate::namespace::mcp_name_from_prefix(&entry.prefix)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "backend '{}' prefix {:?} is not a valid namespace prefix: it must end with the '_' separator and contain a non-empty stem",
                        entry.id,
                        entry.prefix
                    )
                })?
                .to_string();
            Ok(BackendConfig {
                name,
                transport: TransportType::Http,
                command: None,
                args: Vec::new(),
                url: Some(entry.url.clone()),
                env: HashMap::new(),
                timeout: Some(TimeoutConfig {
                    seconds: entry.timeout_seconds,
                }),
                circuit_breaker: None,
                rate_limit: None,
                concurrency: None,
                retry: None,
                outlier_detection: None,
                hedging: None,
                mirror_of: None,
                mirror_percent: 100,
                cache: None,
                bearer_token: None,
                forward_auth: false,
                aliases: Vec::new(),
                default_args: serde_json::Map::new(),
                inject_args: Vec::new(),
                param_overrides: Vec::new(),
                expose_tools: Vec::new(),
                hide_tools: Vec::new(),
                expose_resources: Vec::new(),
                hide_resources: Vec::new(),
                expose_prompts: Vec::new(),
                hide_prompts: Vec::new(),
                hide_destructive: false,
                read_only_only: false,
                failover_for: None,
                priority: 0,
                canary_of: None,
                weight: 100,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(ProxyConfig {
        proxy: ProxySettings {
            name: DEFAULT_SERVER_NAME.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            separator: NAMESPACE_SEPARATOR.to_string(),
            listen: ListenConfig {
                host: public.server.host.clone(),
                port: public.server.port,
            },
            instructions: public.server.instructions.clone(),
            shutdown_timeout_seconds: public.server.shutdown_timeout_seconds,
            hot_reload: false,
            import_backends: None,
            rate_limit: None,
            tool_discovery: false,
            tool_exposure: ToolExposure::Direct,
        },
        backends,
        auth: None,
        performance: Default::default(),
        security: mcp_proxy::config::SecurityConfig {
            max_argument_size: Some(public.policy.max_argument_size_bytes),
            admin_token: None,
        },
        cache: Default::default(),
        observability: mcp_proxy::config::ObservabilityConfig {
            audit: public.observability.audit,
            log_level: public.observability.log_level.clone(),
            json_logs: public.observability.json_logs,
            ..Default::default()
        },
        composite_tools: Vec::new(),
        source_path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_six_style() -> ProxyConfig {
        let text = r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
hot_reload = false
tool_exposure = "direct"
tool_discovery = false

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "alpha"
transport = "http"
url = "http://127.0.0.1:19001/mcp"
[backends.timeout]
seconds = 20

[[backends]]
name = "beta"
transport = "http"
url = "http://127.0.0.1:19002/mcp"
[backends.timeout]
seconds = 20

[security]
max_argument_size = 1048576

[observability]
audit = true
log_level = "info"
json_logs = false
"#;
        toml::from_str(text).expect("parse legacy fixture")
    }

    #[test]
    fn current_style_deployment_maps_to_public_schema() {
        let legacy = legacy_six_style();
        crate::gateway::policy::validate_proxy_policy(&legacy).expect("legacy policy");
        let public = to_public(&legacy).expect("migrate");

        assert_eq!(public.schema_version, SCHEMA_VERSION);
        assert_eq!(public.server.host, "127.0.0.1");
        assert_eq!(public.server.port, 17777);
        assert_eq!(public.backends.len(), 2);
        assert_eq!(public.backends[0].id, "alpha");
        assert_eq!(public.backends[0].prefix, "alpha_");
        assert_eq!(public.backends[0].url, "http://127.0.0.1:19001/mcp");
        // Degrade-on-outage behavior of the current deployment is preserved.
        assert!(!public.backends[0].required);
        assert_eq!(public.backends[0].timeout_seconds, 20);
        validate::validate(&public).expect("migrated config passes public policy");
    }

    #[test]
    fn public_config_maps_back_to_equivalent_legacy_config() {
        let public: GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[server]
host = "127.0.0.1"
port = 18888

[[backends]]
id = "fs"
prefix = "fs_"
url = "http://127.0.0.1:18701/mcp"
required = true
timeout_seconds = 15

[[backends]]
id = "calc"
prefix = "calc_"
url = "http://127.0.0.1:18702/mcp"
"#,
        )
        .expect("parse public fixture");

        let legacy = to_legacy(&public).expect("construct legacy");
        assert_eq!(legacy.proxy.listen.host, "127.0.0.1");
        assert_eq!(legacy.proxy.listen.port, 18888);
        assert_eq!(legacy.backends.len(), 2);
        assert_eq!(legacy.backends[0].name, "fs");
        assert_eq!(
            legacy.backends[0].url.as_deref(),
            Some("http://127.0.0.1:18701/mcp")
        );
        assert_eq!(
            legacy.backends[0]
                .timeout
                .as_ref()
                .expect("timeout")
                .seconds,
            15
        );
        assert!(legacy.auth.is_none());
        assert!(legacy.security.admin_token.is_none());
        assert!(!legacy.proxy.hot_reload);
        assert!(!legacy.proxy.tool_discovery);
        crate::gateway::policy::validate_proxy_policy(&legacy).expect("legacy policy passes");
    }

    #[test]
    fn empty_public_configuration_maps_to_a_backend_free_legacy_proxy() {
        let public: GatewayConfig = toml::from_str("schema_version = 1").expect("parse empty");
        let legacy = to_legacy(&public).expect("empty public maps to legacy");
        assert!(
            legacy.backends.is_empty(),
            "migration never invents backends"
        );
        assert_eq!(legacy.proxy.separator, NAMESPACE_SEPARATOR);

        let round_tripped = to_public(&legacy).expect("legacy maps back to public");
        assert!(round_tripped.backends.is_empty());
        assert_eq!(round_tripped.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn migration_rejects_unmappable_backends() {
        let mut legacy = legacy_six_style();
        legacy.backends[0].transport = TransportType::Stdio;
        legacy.backends[0].url = None;
        let error = to_public(&legacy).expect_err("stdio cannot migrate");
        assert!(error.to_string().contains("must use HTTP"));
    }
}
