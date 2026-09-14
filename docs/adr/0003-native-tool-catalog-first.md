# ADR-0003: Prefer the native full tool catalog in v1

- Status: Accepted
- Date: 2026-09-13

## Context

Aggregating many MCP servers can create a large tool catalog. However, collapsing everything into generic search/call tools hides per-tool JSON Schema, annotations, and operation identity from the client.

## Decision

v1 exposes backend tools natively with namespaces and starts with the full catalog.

Measure catalog size, ChatGPT scan behavior, and tool-selection quality. Move to search exposure only when real evidence shows the full catalog is problematic.

## Consequences

- Preserves typing and tool identity.
- Integration tests must track catalog growth.
- A move to search mode requires a separate ADR and security review.
