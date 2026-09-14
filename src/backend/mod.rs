//! Backend domain: validated descriptors, the registry, and startup probes.

pub mod descriptor;
pub mod probe;
pub mod registry;

pub use descriptor::BackendDescriptor;
pub use probe::{
    ProbeReport, ProbedBackend, UnavailableBackend, UpstreamTool, probe_backend, probe_registry,
};
pub use registry::BackendRegistry;
