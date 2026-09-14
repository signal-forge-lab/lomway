# ADR-0002: The gateway does not own backend process lifecycle

- Status: Accepted
- Date: 2026-09-13

## Context

Each local MCP already has its own runtime/supervision. Having the gateway spawn, kill, or restart those processes would duplicate responsibility with the existing supervisor.

## Decision

The gateway only connects to backend HTTP MCP endpoints. Start/stop/restart remains owned by Swibo or the existing supervisor.

## Consequences

- Gateway restarts do not cascade into backend restarts.
- Backend-specific restart policy remains intact.
- Supervisor start order is backend → gateway → tunnel.
- E2E tests must verify behavior when a backend is absent at gateway startup.
