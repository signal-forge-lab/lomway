//! The gateway runtime: policy validation, router surface, and builder.

mod auth;
pub mod build;
mod deferred;
mod discovery;
pub mod policy;
mod reconnect;
pub mod router;

pub use build::{Gateway, build_proxy, build_proxy_with_monitor};
pub use policy::validate_proxy_policy;
pub use reconnect::MonitorHandle;
pub use router::{gateway_router, gateway_router_with_monitor, serve};
