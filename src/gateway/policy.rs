//! Policy validation for legacy (`mcp-proxy` shaped) configurations.
//!
//! This is the original gateway policy, decomposed into focused validators
//! and composed back together. Behavior is byte-for-byte compatible with
//! the pre-generalization deployment: the same inputs produce the same
//! accept/reject decisions and the same error text.

use anyhow::{Result, bail, ensure};
use mcp_proxy::ProxyConfig;
use mcp_proxy::config::{ToolExposure, TransportType};

use crate::config::validate::MAX_ARGUMENT_SIZE_BYTES;

/// Enforce the intentionally small v1 responsibility boundary on a legacy
/// configuration.
///
/// Upstream supports many routing/middleware features. This gateway does not:
/// arbitrary retries, fan-out, schema rewriting, dynamic reload, arbitrary
/// remote backends, or northbound auth belong outside this thin boundary.
pub fn validate_proxy_policy(config: &ProxyConfig) -> Result<()> {
    validate_proxy_policy_with_remote_backends(config, false)
}

/// Validate the legacy-shaped runtime config after a validated public config
/// explicitly opted into the narrow Tailscale HTTPS backend profile.
pub(crate) fn validate_proxy_policy_with_remote_backends(
    config: &ProxyConfig,
    allow_non_loopback_backends: bool,
) -> Result<()> {
    validate_listener(config)?;
    validate_namespace_contract(config)?;
    validate_exposure_features(config)?;
    validate_northbound_surface(config)?;
    validate_performance_features(config)?;
    validate_argument_bound(config)?;
    validate_backend_population(config)?;
    for backend in &config.backends {
        validate_backend_transport(backend)?;
        validate_backend_endpoint(backend, allow_non_loopback_backends)?;
        validate_backend_resilience(backend)?;
        validate_backend_surface(backend)?;
    }
    Ok(())
}

fn validate_listener(config: &ProxyConfig) -> Result<()> {
    ensure!(
        config.proxy.listen.host == "127.0.0.1",
        "proxy.listen.host must be exactly 127.0.0.1"
    );
    Ok(())
}

fn validate_namespace_contract(config: &ProxyConfig) -> Result<()> {
    ensure!(
        config.proxy.separator == "_",
        "proxy.separator must be '_' for the stable namespace contract"
    );
    Ok(())
}

fn validate_exposure_features(config: &ProxyConfig) -> Result<()> {
    ensure!(
        !config.proxy.hot_reload,
        "proxy.hot_reload is disabled in v1 because gateway-specific policy is validated only at startup"
    );
    ensure!(
        config.proxy.tool_exposure == ToolExposure::Direct,
        "proxy.tool_exposure must be 'direct' in v1"
    );
    ensure!(
        !config.proxy.tool_discovery,
        "proxy.tool_discovery is disabled in v1"
    );
    ensure!(
        config.proxy.rate_limit.is_none(),
        "proxy.rate_limit is outside the v1 timeout-only middleware policy"
    );
    Ok(())
}

fn validate_northbound_surface(config: &ProxyConfig) -> Result<()> {
    ensure!(
        config.auth.is_none(),
        "northbound MCP auth must remain outside the loopback gateway; Secure MCP Tunnel owns remote access"
    );
    ensure!(
        config.security.admin_token.is_none(),
        "security.admin_token must be absent because v1 serves no admin plane"
    );
    Ok(())
}

fn validate_performance_features(config: &ProxyConfig) -> Result<()> {
    ensure!(
        !config.performance.coalesce_requests,
        "performance.coalesce_requests is disabled in v1"
    );
    ensure!(
        config.composite_tools.is_empty(),
        "composite_tools are disabled in v1"
    );
    ensure!(
        config.cache.backend == "memory"
            && config.cache.url.is_none()
            && config.cache.prefix == "mcp-proxy:",
        "global cache backend configuration must remain at the inert default in v1"
    );
    Ok(())
}

fn validate_argument_bound(config: &ProxyConfig) -> Result<()> {
    let max_arguments = config.security.max_argument_size.unwrap_or(0);
    ensure!(
        (1..=MAX_ARGUMENT_SIZE_BYTES).contains(&max_arguments),
        "security.max_argument_size must be set to at most 1048576 bytes"
    );
    Ok(())
}

/// The legacy deployment schema keeps the pre-generalization population
/// rule: at least one backend is required. This matches the pinned upstream
/// `mcp_proxy::ProxyConfig` loader (which rejects an empty backend list) and
/// the current production baseline. The public schema generalizes this to
/// 0..N backends; see [`crate::config::validate`].
fn validate_backend_population(config: &ProxyConfig) -> Result<()> {
    ensure!(
        !config.backends.is_empty(),
        "at least one backend is required"
    );
    Ok(())
}

fn validate_backend_transport(backend: &mcp_proxy::config::BackendConfig) -> Result<()> {
    if !matches!(backend.transport, TransportType::Http) {
        bail!(
            "backend '{}' must use HTTP; backend process lifecycle is owned outside the gateway",
            backend.name
        );
    }
    Ok(())
}

fn validate_backend_endpoint(
    backend: &mcp_proxy::config::BackendConfig,
    allow_non_loopback_backends: bool,
) -> Result<()> {
    let url = backend.url.as_deref().unwrap_or_default();
    ensure!(
        crate::config::validate::is_exact_loopback_mcp_url(url)
            || (allow_non_loopback_backends
                && crate::config::validate::is_exact_tailnet_https_mcp_url(url)),
        "backend '{}' URL must be loopback HTTP or an explicitly allowed Tailscale HTTPS /mcp endpoint",
        backend.name
    );
    ensure!(
        backend.retry.is_none(),
        "backend '{}' configures retry; automatic tool retry is forbidden",
        backend.name
    );
    ensure!(
        backend.hedging.is_none(),
        "backend '{}' configures hedging; duplicate tool dispatch is forbidden",
        backend.name
    );
    ensure!(
        backend.cache.is_none(),
        "backend '{}' configures cache; tool-call caching is forbidden",
        backend.name
    );
    Ok(())
}

fn validate_backend_resilience(backend: &mcp_proxy::config::BackendConfig) -> Result<()> {
    ensure!(
        backend.circuit_breaker.is_none()
            && backend.rate_limit.is_none()
            && backend.concurrency.is_none()
            && backend.outlier_detection.is_none(),
        "backend '{}' configures middleware outside the v1 timeout-only policy",
        backend.name
    );
    ensure!(
        backend.mirror_of.is_none(),
        "backend '{}' configures mirror fan-out, which is forbidden",
        backend.name
    );
    ensure!(
        backend.failover_for.is_none(),
        "backend '{}' configures failover, which is forbidden",
        backend.name
    );
    ensure!(
        backend.canary_of.is_none(),
        "backend '{}' configures canary routing, which is forbidden",
        backend.name
    );
    ensure!(
        !backend.forward_auth,
        "backend '{}' enables forward_auth, which is outside the v1 trust boundary",
        backend.name
    );
    Ok(())
}

fn validate_backend_surface(backend: &mcp_proxy::config::BackendConfig) -> Result<()> {
    ensure!(
        backend.aliases.is_empty()
            && backend.default_args.is_empty()
            && backend.inject_args.is_empty()
            && backend.param_overrides.is_empty()
            && backend.expose_tools.is_empty()
            && backend.hide_tools.is_empty()
            && backend.expose_resources.is_empty()
            && backend.hide_resources.is_empty()
            && backend.expose_prompts.is_empty()
            && backend.hide_prompts.is_empty()
            && !backend.hide_destructive
            && !backend.read_only_only,
        "backend '{}' configures schema/capability rewriting; v1 preserves backend surfaces",
        backend.name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_policy_keeps_the_single_backend_population_rule() {
        // The public schema permits zero backends, but a legacy deployment
        // configuration still requires at least one — unchanged from the
        // pre-generalization baseline (no safety reduction).
        let public: crate::config::model::GatewayConfig =
            toml::from_str("schema_version = 1").expect("parse empty public config");
        let legacy = crate::config::migrate::to_legacy(&public).expect("map to legacy");
        assert!(legacy.backends.is_empty());
        let error = validate_proxy_policy(&legacy).expect_err("legacy gate keeps >=1 backend");
        assert!(error.to_string().contains("at least one backend"));
    }

    #[test]
    fn public_policy_can_explicitly_enable_tailnet_https_backend() {
        let public: crate::config::model::GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[policy]
allow_non_loopback_backends = true

[[backends]]
id = "workbridge_mac"
prefix = "workbridge_mac_"
url = "https://workbridge-mac.example-tailnet.ts.net/mcp"
required = false
timeout_seconds = 30
"#,
        )
        .expect("parse public config");
        let legacy = crate::config::migrate::to_legacy(&public).expect("map to runtime config");
        assert!(
            validate_proxy_policy(&legacy).is_err(),
            "legacy entry point remains loopback-only"
        );
        validate_proxy_policy_with_remote_backends(&legacy, true)
            .expect("public opt-in admits the narrow Tailscale HTTPS form");
    }
}
