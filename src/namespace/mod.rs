//! Namespace policy: prefix charset, reserved prefixes, and collision
//! detection for final tool names.

pub mod collision;
pub mod normalize;

pub use collision::{PlannedBackend, PlannedTool, ToolPlan, plan_tools};
pub use normalize::{NAMESPACE_SEPARATOR, mcp_name_from_prefix, public_tool_name};
