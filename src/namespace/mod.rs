//! Namespace policy: prefix charset, reserved prefixes, and collision
//! detection for final tool names.

pub(crate) mod collision;
pub(crate) mod normalize;

pub(crate) use normalize::{NAMESPACE_SEPARATOR, mcp_name_from_prefix, public_tool_name};
