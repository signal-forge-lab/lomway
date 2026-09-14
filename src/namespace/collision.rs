//! Startup tool collision preflight.
//!
//! Final tool names are computed for every healthy backend before the
//! gateway serves a single request. Any collision fails startup while
//! naming both sources, so ambiguity can never surface at call time.

use std::collections::HashMap;

use anyhow::{Result, bail};

use crate::backend::probe::ProbeReport;
use crate::namespace::public_tool_name;

/// The complete precomputed tool surface of the gateway.
#[derive(Debug, Clone)]
pub struct ToolPlan {
    total_tools: usize,
}

impl ToolPlan {
    /// Total number of planned final tool names.
    pub fn total_tools(&self) -> usize {
        self.total_tools
    }
}

/// Compute the final tool plan from a startup probe report.
///
/// Fails fast when two upstream tools would map to the same final name; the
/// error identifies both sources.
pub fn plan_tools(report: &ProbeReport) -> Result<ToolPlan> {
    let mut seen: HashMap<String, (&str, &str)> = HashMap::new();
    let mut total = 0usize;

    for backend in &report.healthy {
        for tool in &backend.tools {
            let public_name = public_tool_name(backend.descriptor.prefix(), &tool.name);
            if let Some((owner, upstream)) = seen.get(&public_name) {
                bail!(
                    "final tool name '{public_name}' collides: backend '{}' (upstream '{}') vs backend '{}' (upstream '{}'); adjust backend prefixes",
                    owner,
                    upstream,
                    backend.descriptor.id(),
                    tool.name
                );
            }
            seen.insert(public_name, (backend.descriptor.id(), &tool.name));
            total += 1;
        }
    }

    Ok(ToolPlan { total_tools: total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::descriptor::BackendDescriptor;
    use crate::backend::probe::{ProbeReport, ProbedBackend};

    fn descriptor(id: &str, prefix: &str, url: &str) -> BackendDescriptor {
        BackendDescriptor::try_from_entry(&crate::config::model::BackendEntry {
            id: id.to_string(),
            prefix: prefix.to_string(),
            url: url.to_string(),
            required: true,
            timeout_seconds: 5,
        })
        .expect("valid descriptor")
    }

    fn tool(name: &str) -> crate::backend::probe::UpstreamTool {
        crate::backend::probe::UpstreamTool {
            name: name.to_string(),
        }
    }

    #[test]
    fn plans_deterministic_unique_names() {
        let report = ProbeReport {
            healthy: vec![
                ProbedBackend {
                    descriptor: descriptor("a", "a_", "http://127.0.0.1:19001/mcp"),
                    tools: vec![tool("status"), tool("extra")],
                },
                ProbedBackend {
                    descriptor: descriptor("b", "b_", "http://127.0.0.1:19002/mcp"),
                    tools: vec![tool("status")],
                },
            ],
            unavailable: Vec::new(),
        };
        let plan = plan_tools(&report).expect("no collision expected");
        assert_eq!(plan.total_tools(), 3);
    }

    #[test]
    fn collisions_fail_fast_and_identify_both_sources() {
        let report = ProbeReport {
            healthy: vec![
                ProbedBackend {
                    descriptor: descriptor("a", "a_", "http://127.0.0.1:19001/mcp"),
                    tools: vec![tool("b_c")],
                },
                ProbedBackend {
                    descriptor: descriptor("b", "a_b_", "http://127.0.0.1:19002/mcp"),
                    tools: vec![tool("c")],
                },
            ],
            unavailable: Vec::new(),
        };
        let error = plan_tools(&report).expect_err("cross-backend collision");
        let message = error.to_string();
        assert!(message.contains("'a_b_c' collides"), "{message}");
        assert!(
            message.contains("backend 'a' (upstream 'b_c')"),
            "{message}"
        );
        assert!(message.contains("backend 'b' (upstream 'c')"), "{message}");
    }

    #[test]
    fn duplicate_upstream_names_within_one_backend_also_fail() {
        let report = ProbeReport {
            healthy: vec![ProbedBackend {
                descriptor: descriptor("a", "a_", "http://127.0.0.1:19001/mcp"),
                tools: vec![tool("dup"), tool("dup")],
            }],
            unavailable: Vec::new(),
        };
        let error = plan_tools(&report).expect_err("intra-backend collision");
        assert!(error.to_string().contains("'a_dup' collides"));
    }
}
