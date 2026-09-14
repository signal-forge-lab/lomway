# ADR-0001: Use the `mcp-proxy` library as the base

- Status: Accepted
- Date: 2026-09-13

## Context

Aggregating multiple local MCP servers behind one endpoint requires both MCP client/server behavior, namespacing, backend connection handling, health, and configuration reload. Reimplementing these primitives creates protocol-tracking and maintenance cost.

## Decision

v1 pins `joshrotenberg/mcp-proxy` v0.4.3 as a library and explicitly enables MCP `2026-07-28` support.

Project-specific code is limited to a thin host.

## Consequences

Positive:

- Avoids reinventing protocol/routing behavior.
- Reuses upstream failure isolation and observability.
- Supports Windows.

Negative:

- Upstream API/config changes must be reviewed.
- Upstream defaults that do not fit this trust boundary, especially admin MCP tools, must be safely suppressed.

## Rejected alternatives

- Full custom Rust MCP proxy: unnecessary reimplementation.
- Put aggregation inside Workbridge: expands Workbridge responsibility and creates an unnecessary shared failure domain.
- Keep all existing individual connectors forever: does not meet the operations-simplification goal.
