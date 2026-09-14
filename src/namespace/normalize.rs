//! Prefix normalization rules.
//!
//! The namespace contract is `<prefix><upstream tool name>` with `_` as the
//! only separator. A valid prefix matches the policy documented in
//! [`crate::config::validate`]: lowercase `a-z`, `0-9`, `_`, `-`, at least
//! one character, terminated by the `_` separator, and not one of the
//! reserved control-plane prefixes. Ambiguous inputs are rejected by
//! configuration validation; this module only applies the validated forms
//! and never silently normalizes an ambiguous prefix itself.

/// The single namespace separator used between backend prefix and tool name.
pub const NAMESPACE_SEPARATOR: &str = "_";

/// The `mcp-proxy` backend name that realizes `prefix`: the prefix without
/// its trailing separator.
///
/// Returns `None` unless `prefix` is unambiguously namespaced — exactly one
/// trailing `_` separator with a non-empty stem before it. Ambiguous forms
/// (`"fs"`, `"_"`, `""`) are rejected instead of being silently normalized.
pub fn mcp_name_from_prefix(prefix: &str) -> Option<&str> {
    let stem = prefix.strip_suffix(NAMESPACE_SEPARATOR)?;
    (!stem.is_empty()).then_some(stem)
}

/// Compute the client-visible tool name for an upstream tool.
///
/// Pure concatenation: the upstream name is never rewritten, so the final
/// name is fully determined by the validated prefix and the upstream name.
pub fn public_tool_name(prefix: &str, upstream_tool: &str) -> String {
    format!("{prefix}{upstream_tool}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_maps_to_mcp_name_and_back() {
        assert_eq!(mcp_name_from_prefix("fs_"), Some("fs"));
        assert_eq!(mcp_name_from_prefix("my_fs_"), Some("my_fs"));
        assert_eq!(
            public_tool_name("fs_", "read_file"),
            "fs_read_file",
            "final names are prefix + upstream name, no rewriting"
        );
    }

    #[test]
    fn ambiguous_prefixes_are_never_silently_normalized() {
        assert_eq!(
            mcp_name_from_prefix("fs"),
            None,
            "missing separator is rejected, not auto-appended"
        );
        assert_eq!(
            mcp_name_from_prefix("_"),
            None,
            "empty stem is rejected, not collapsed"
        );
        assert_eq!(mcp_name_from_prefix(""), None, "empty prefix is rejected");
    }
}
