# Feature Specification: Hybrid Tool Exposure

**Feature Branch**: `001-hybrid-tool-exposure`
**Created**: 2026-09-27
**Status**: Accepted

## User Scenarios & Testing

### User Story 1 - Keep common tools direct (Priority: P1)

As a ChatGPT Web user, I want frequently used Lomway backends to remain normal MCP tools so common work stays simple and low-latency.

**Independent Test**: Start Lomway with mixed direct/deferred backends and verify direct tools are present in `tools/list` and callable by their normal names.

**Acceptance Scenarios**:

1. **Given** a backend with `exposure = "direct"`, **When** a client calls `tools/list`, **Then** that backend's tools are listed with their normal Lomway prefixes.
2. **Given** a direct tool, **When** a client calls it normally, **Then** Lomway forwards it without requiring a meta-tool.

### User Story 2 - Hide low-frequency tools behind safe discovery (Priority: P1)

As a ChatGPT Web user, I want low-frequency backend schemas omitted from the normal tool catalog while retaining access when needed.

**Independent Test**: Mark a backend deferred and verify its tools disappear from `tools/list`, direct calls are rejected, search/describe can discover schemas, and call_tool can invoke only those deferred tools.

**Acceptance Scenarios**:

1. **Given** a backend with `exposure = "deferred"`, **When** a client calls `tools/list`, **Then** its individual tools are absent.
2. **Given** a deferred tool name, **When** a client calls it directly, **Then** the request is rejected before backend dispatch.
3. **Given** a relevant text query, **When** `lomway_search_tools` is called, **Then** matching deferred tools are returned with stable fully-qualified names and short descriptions.
4. **Given** an exact deferred tool name, **When** `lomway_describe_tool` is called, **Then** its input schema and descriptive metadata are returned.
5. **Given** an exact deferred tool name and valid arguments, **When** `lomway_call_tool` is called, **Then** the result is forwarded from that backend.

### User Story 3 - Preserve the security boundary (Priority: P1)

As the gateway owner, I want discovery to expose only safe invocation metadata and never restore the upstream proxy control plane.

**Independent Test**: Verify `proxy/config`, `proxy/add_backend`, and other upstream admin tools remain absent and uncallable while Lomway meta-tools cannot invoke direct or control-plane tools.

**Acceptance Scenarios**:

1. **Given** the production profile, **When** `tools/list` is requested, **Then** no upstream `proxy/*` admin tool appears.
2. **Given** `lomway_call_tool`, **When** a caller supplies a direct, unknown, `proxy_*`, or `lomway_*` target, **Then** Lomway rejects it.

### Edge Cases

- Optional deferred backend unavailable at startup: it contributes no searchable tools while unavailable. If its supervised process becomes reachable later, Lomway's existing reconnect monitor adopts it and subsequent search/describe/call operations see its live catalog without a gateway restart.
- Duplicate final tool names remain a startup error, regardless of exposure mode.
- Empty or whitespace-only search queries are rejected.
- Search result limits are bounded.
- Exact schema metadata is captured from startup `tools/list`; no schema is synthesized by Lomway.

## Requirements

### Functional Requirements

- **FR-001**: Each public backend entry MUST support `exposure = "direct" | "deferred"`, defaulting to `direct`.
- **FR-002**: Direct backends MUST preserve existing listing and direct-call behavior.
- **FR-003**: Deferred backend tools MUST be absent from client `tools/list`.
- **FR-004**: Direct calls to deferred backend tools MUST be rejected.
- **FR-005**: Lomway MUST expose exactly the safe meta-tool surface needed for deferred discovery: `lomway_search_tools`, `lomway_describe_tool`, and `lomway_call_tool`.
- **FR-006**: `lomway_search_tools` MUST search only the current raw-proxy catalog for backends configured as deferred and support optional backend filtering plus a bounded result limit.
- **FR-007**: `lomway_describe_tool` MUST return the exact current MCP tool definition for one registered deferred tool.
- **FR-008**: `lomway_call_tool` MUST invoke only exact tool names that are both currently listed and owned by a configured deferred backend, and MUST forward the backend result without retry.
- **FR-009**: Optional deferred backends that become reachable after gateway startup MUST be adopted by the existing reconnect monitor without requiring a gateway restart.
- **FR-010**: Upstream `proxy/*` control-plane/admin tools MUST remain removed and unreachable.
- **FR-011**: Existing timeout, loopback, OAuth, namespace, no-retry, no-cache, and no-fanout policies MUST remain unchanged.
- **FR-012**: Backend prefixes MUST be non-overlapping as well as unique so exact tool ownership remains unambiguous for both direct routing and deferred allowlisting.

## Success Criteria

- **SC-001**: A mixed fixture proves direct tools remain listed while deferred tools do not.
- **SC-002**: Direct calls to deferred tools fail without backend invocation, while the same tool succeeds through `lomway_call_tool`.
- **SC-003**: Search + describe is sufficient for an MCP client that never receives the deferred schema in its normal `tools/list`.
- **SC-004**: An optional deferred backend started after the gateway is already serving becomes searchable and callable without restarting the gateway.
- **SC-005**: Existing repository verification remains green and no upstream admin tool is reintroduced.
- **SC-006**: Production configuration can move Browser, CUA Windows, Chrome DevTools, UFO, Jev, and XMind to deferred exposure without changing their backend URLs or lifecycle ownership.

## Assumptions

- ChatGPT Web may cache its MCP tool catalog and does not reliably apply `tools/list_changed` within the same response. This feature therefore uses a stable meta-tool surface rather than mid-turn tool-list mutation.
- Swibo continues to own backend process lifecycle; Lomway does not start or stop deferred backends.
- The raw proxy's currently registered catalog is the schema source of truth for deferred discovery; backend lifecycle/health remains external and a registered backend may still fail a call after going offline.
