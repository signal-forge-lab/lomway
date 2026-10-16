//! The gateway runtime: policy validation, router surface, and builder.

mod auth;
pub mod build;
mod capability;
mod deferred;
mod discovery;
mod manifest;
pub mod policy;
mod reconnect;
pub mod router;

pub use build::{Gateway, build_proxy, build_proxy_with_monitor};
pub use manifest::{STATE_DIR_ENV_VAR, default_state_dir};
pub use policy::validate_proxy_policy;
pub use reconnect::MonitorHandle;
pub use router::{gateway_router, gateway_router_with_monitor, serve};
