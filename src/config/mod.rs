//! Versioned public configuration model for Lomway.
//!
//! The public schema is independent of any single deployment: it aggregates
//! 0..N arbitrary HTTP Streamable MCP backends with explicit namespace
//! prefixes. Unknown keys are rejected so configuration drift fails loudly
//! instead of being silently ignored.

mod constants;
pub mod load;
pub mod migrate;
pub mod model;
pub mod validate;

pub use load::{load_config, load_gateway_config, resolve_config_path};
pub use model::{BackendEntry, GatewayConfig, ObservabilityConfig, PolicyConfig, ServerConfig};
pub use validate::{
    ALLOWED_PREFIX_CHARSET, MAX_ARGUMENT_SIZE_BYTES, RESERVED_PREFIXES, SCHEMA_VERSION, validate,
};
