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
use mcp_proxy::ProxyConfig;

use crate::backend::registry::BackendRegistry;
use crate::config::migrate;
use crate::config::model::GatewayConfig;
use crate::gateway::policy::validate_proxy_policy;
use crate::health::{HealthState, HealthTracker};
use crate::namespace::collision::plan_tools;

/// A constructed, validated gateway that is not yet bound to a port.
#[derive(Debug, Clone)]
pub struct Gateway {
    router: Router,
    listen_host: String,
    listen_port: u16,
    health: HealthState,
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

        let health = HealthTracker::from_startup(&registry, &report).snapshot();
        ensure!(
            health.ready(),
            "startup validation failed; readiness was never achieved"
        );

        let listen_host = public.server.host.clone();
        let listen_port = public.server.port;
        // Reuse the established production proxy path so the new public CLI
        // keeps backend restart recovery and live readiness semantics. The
        // legacy mapping runs after startup validation and re-validates the
        // public model.
        let legacy = migrate::to_legacy(&public)?;
        let proxy = crate::build_proxy(legacy).await?;
        let namespaces = proxy.mcp_proxy().backend_namespaces();
        let router = crate::gateway_router(proxy);
        Ok(Self {
            router,
            listen_host,
            listen_port,
            health,
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

    /// Startup health snapshot served by `/readyz`.
    pub fn health(&self) -> &HealthState {
        &self.health
    }

    /// Client-visible backend namespaces (after control-plane removal).
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    /// Bind the configured loopback listener and serve until Ctrl-C.
    pub async fn serve(self) -> Result<()> {
        let listener =
            tokio::net::TcpListener::bind((self.listen_host.as_str(), self.listen_port)).await?;
        tracing::info!(listen = %listener.local_addr()?, mcp_path = "/mcp", "gateway ready");
        axum::serve(listener, self.router)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("shutdown signal received");
            })
            .await?;
        Ok(())
    }
}
