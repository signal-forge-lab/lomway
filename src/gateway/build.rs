//! Gateway construction.
//!
//! Both entry points (public configuration and legacy deployment
//! configuration) converge on the same fail-closed pipeline:
//!
//! 1. policy validation;
//! 2. backend registry construction (validated descriptors only);
//! 3. startup probe (required backends must be up; optional backends may
//!    degrade; all-unavailable fails);
//! 4. tool collision preflight (final names computed before serving);
//! 5. upstream proxy composition with the `proxy` control-plane backend
//!    removed — refusal to serve if that removal cannot be proven.

use anyhow::{Result, ensure};
use axum::Router;
use mcp_proxy::{Proxy, ProxyConfig};

use crate::backend::registry::BackendRegistry;
use crate::config::migrate;
use crate::config::model::GatewayConfig;
use crate::gateway::policy::validate_proxy_policy;
use crate::gateway::reconnect;
use crate::namespace::collision::plan_tools;

/// Construct the upstream proxy and fail closed unless its client-visible
/// control-plane backend can be removed before serving.
///
/// Backend transport recovery is attached here so every serving entry point
/// receives identical long-lived recovery semantics without retrying client
/// tool calls.
pub async fn build_proxy(config: ProxyConfig) -> Result<Proxy> {
    validate_proxy_policy(&config)?;
    let reconnect_specs = reconnect::specs(&config);
    let proxy = Proxy::from_config(config).await?;
    ensure!(
        proxy.mcp_proxy().remove_backend("proxy").await,
        "upstream control-plane MCP backend 'proxy' was not present; refusing to serve because admin-tool removal cannot be proven"
    );
    reconnect::spawn(proxy.mcp_proxy().clone(), reconnect_specs);
    Ok(proxy)
}

/// A constructed, validated gateway that is not yet bound to a port.
#[derive(Debug, Clone)]
pub struct Gateway {
    router: Router,
    listen_host: String,
    listen_port: u16,
    namespaces: Vec<String>,
}

impl Gateway {
    /// Build from the public configuration model.
    pub async fn build(config: GatewayConfig) -> Result<Self> {
        Self::finish(config).await
    }

    /// Build from a legacy deployment configuration (the current six-backend
    /// reference deployment keeps using this path).
    pub async fn build_from_legacy(config: ProxyConfig) -> Result<Self> {
        validate_proxy_policy(&config)?;
        let public = migrate::to_public(&config)?;
        Self::finish(public).await
    }

    async fn finish(public: GatewayConfig) -> Result<Self> {
        // The registry and probe always operate on the public view of the
        // configuration so both entry points share identical startup
        // semantics. The public model is the probe source of truth, so the
        // public schema's `required` flag is honored here: a required backend
        // that is down fails startup, exactly as the public documentation
        // promises. The legacy entry point maps first, and that mapping keeps
        // the deployment's historical degrade-on-outage behavior by marking
        // migrated backends optional.
        let registry = BackendRegistry::from_config(&public)?;
        let report = crate::backend::probe::probe_registry(&registry).await?;
        let plan = plan_tools(&report)?;
        tracing::info!(
            backends = report.healthy.len(),
            degraded = report.unavailable.len(),
            tools = plan.total_tools(),
            "startup validation complete"
        );

        let listen_host = public.server.host.clone();
        let listen_port = public.server.port;
        // Reuse the established production proxy path so the new public CLI
        // keeps backend restart recovery and live readiness semantics. The
        // legacy mapping runs after startup validation and re-validates the
        // public model.
        let oauth = public.server.oauth.clone();
        let legacy = migrate::to_legacy(&public)?;
        let proxy = build_proxy(legacy).await?;
        let namespaces = proxy.mcp_proxy().backend_namespaces();
        let router = crate::gateway::router::gateway_router_with_oauth(proxy, oauth);
        Ok(Self {
            router,
            listen_host,
            listen_port,
            namespaces,
        })
    }

    /// The externally served HTTP surface.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Configured listener address.
    pub fn listen_addr(&self) -> (&str, u16) {
        (&self.listen_host, self.listen_port)
    }

    /// Client-visible backend namespaces (after control-plane removal).
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    /// Bind the configured loopback listener and serve until Ctrl-C.
    pub async fn serve(self) -> Result<()> {
        crate::gateway::router::serve_router(
            self.router,
            self.listen_host.as_str(),
            self.listen_port,
        )
        .await
    }
}
