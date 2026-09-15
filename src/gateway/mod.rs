//! The gateway runtime: policy validation, router surface, and builder.

mod auth;
pub mod build;
pub mod policy;
mod reconnect;
pub mod router;

pub use build::{Gateway, build_proxy};
pub use policy::validate_proxy_policy;
pub use router::{gateway_router, serve};
