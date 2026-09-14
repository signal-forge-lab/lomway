# ADR-0004: Remove the upstream control plane from the northbound surface

- Status: Accepted / Implemented
- Date: 2026-09-13

## Context

`mcp-proxy` 0.4.3 creates two control-plane surfaces that are unnecessary for ChatGPT aggregation:

1. an MCP backend named `proxy` with management tools such as configuration inspection and dynamic backend registration;
2. HTTP `/admin/*` routes containing management operations.

Keeping either surface northbound would increase authority without serving the gateway's aggregation purpose.

## Decision

- After `Proxy::from_config()` and before serving, call `proxy.mcp_proxy().remove_backend("proxy")` and fail closed unless it succeeds.
- Do not expose upstream `/admin/*` from the project-owned listener.
- Reject nested `/mcp/admin/*` before upstream route dispatch.
- Expose only `/mcp` plus the project-owned read-only `/healthz` process check.

Therefore no gateway admin token is required in v1.

## Consequences

- ChatGPT cannot dynamically add/remove/reconfigure backends through the aggregate MCP.
- A compromised MCP client does not gain the upstream proxy control plane.
- Local monitoring uses `/healthz` and Swibo instead of upstream admin APIs.
- Every upstream upgrade must rerun both MCP-tool-absence and admin-path negative tests.
