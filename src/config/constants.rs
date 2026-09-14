//! Schema-level constants shared by the public model and validators.

/// The only supported configuration schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Default (and maximum) tool-call argument payload size in bytes: 1 MiB.
pub const MAX_ARGUMENT_SIZE_BYTES: usize = 1024 * 1024;

/// Maximum backend request/probe timeout in seconds.
pub const MAX_BACKEND_TIMEOUT_SECONDS: u64 = 300;
