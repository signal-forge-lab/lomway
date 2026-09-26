# Implementation Plan: Hybrid Tool Exposure

**Branch**: `001-hybrid-tool-exposure` | **Date**: 2026-09-27

## Summary

Add per-backend hybrid exposure. Frequent backends remain direct. Deferred backends are hidden by upstream capability filtering and reachable only through three Lomway-owned meta-tools backed by the raw proxy's current catalog and dispatch path.

## Technical Context

**Language/Version**: Rust 2024, rust-version 1.90
**Primary Dependencies**: pinned `mcp-proxy = 0.4.3`, `tower-mcp = 0.18.2`
**Storage**: none
**Testing**: cargo unit/integration tests with localhost mock MCP backends
**Target Platform**: Windows-first, loopback HTTP MCP gateway
**Project Type**: infrastructure service / MCP gateway
**Constraints**: no new admin plane, no retry/fanout/cache, no backend lifecycle ownership, preserve existing schemas/results

## Constitution Check

- Minimal Correct Change: PASS. Reuse upstream capability filtering, the raw proxy path, and the existing backend reconnect monitor.
- Verification Is Part of Completion: tests are added before implementation and full gates run afterward.
- Security Boundaries: PASS. Upstream proxy admin backend remains removed; meta-call uses an explicit deferred-tool allowlist.
- Authority: this spec/plan/tasks set owns the new behavior contract.

## Project Structure

```text
src/config/model.rs           # public exposure enum
src/config/migrate.rs         # map deferred -> upstream capability filter
src/gateway/policy.rs         # permit only the generated safe "*" hide filter
src/backend/probe.rs          # retain exact tool definitions
src/gateway/deferred.rs       # safe search / describe / call meta backend
src/gateway/build.rs          # attach safe meta backend before serving
tests/proxy.rs                # mixed direct/deferred E2E
config/proxy.direct.local.toml
docs/*                        # EN/JA public behavior
```

## Design

1. Public `BackendEntry.exposure` defaults to `direct`.
2. Migration maps deferred backends to upstream `hide_tools=["*"]`; direct backends remain unchanged.
3. Build a `DeferredCatalog` containing only configured deferred backend ids/prefixes.
4. Each meta-tool reads the raw proxy's current tool definitions and filters them through that configured deferred allowlist.
5. Add one in-process `lomway` backend with:
   - `search_tools(query, backend?, limit?)`
   - `describe_tool(name)`
   - `call_tool(name, arguments?)`
6. The meta call verifies the exact name exists in the current deferred-filtered raw catalog, then dispatches through raw `McpProxy`; no retry is added.
7. External capability filtering rejects direct calls to deferred tools.
8. The reconnect monitor continues watching configured endpoints that were offline at startup so a later supervisor start can be adopted without a gateway restart.

## Complexity Tracking

No exception to the repository constitution is required.
