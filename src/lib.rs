use std::path::Path;

use anyhow::Result;
use mcp_proxy::ProxyConfig;

mod backend;
pub mod cli;
pub mod config;
pub mod gateway;
mod namespace;

/// Server name the gateway reports for itself during MCP initialization.
pub const DEFAULT_SERVER_NAME: &str = "lomway";

/// Load, resolve environment references, and enforce Lomway's legacy
/// deployment policy. Kept at the crate root for compatibility with the
/// original public API; the canonical implementation lives in `config`.
pub fn load_config(path: &Path) -> Result<ProxyConfig> {
    config::load_config(path)
}

/// Enforce the intentionally small v1 gateway responsibility boundary.
/// Kept at the crate root for compatibility with the original public API.
pub fn validate_policy(config: &ProxyConfig) -> Result<()> {
    gateway::validate_proxy_policy(config)
}

// Stable crate-root exports retained for callers and integration tests while
// the implementations live with their actual gateway responsibilities.
pub use gateway::{build_proxy, gateway_router, serve};
