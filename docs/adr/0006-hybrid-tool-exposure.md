# ADR-0006: Use hybrid direct and deferred tool exposure

- Status: Accepted
- Date: 2026-09-27

## Context

ADR-0003 deliberately started with the native full tool catalog and required
measurement before changing the exposure model. The live Lomway Windows
catalog grew to 259 ChatGPT-visible tools, including 97 Stealth Browser tools.

A Web Chat probe temporarily replaced the 97-tool browser backend with a
one-tool MCP backend. Lomway's own tools/list changed to one browser tool, but
the active Web Chat response continued to use its previously loaded 97 browser
schemas. This proves that same-response tools/list_changed refresh cannot be
the required mechanism for deferred loading.

The pinned mcp-proxy library already contains global search exposure, but those
search/call tools live in the same upstream proxy backend as administrative
tools such as proxy/config and proxy/add_backend. Lomway intentionally removes
that control-plane backend.

## Decision

Use a hybrid exposure model:

- frequent backends use direct exposure and keep their native schemas in
  tools/list;
- low-frequency backends use deferred exposure and are hidden from normal
  tools/list and direct calls;
- Lomway publishes exactly three safe meta-tools:
  lomway_search_tools, lomway_describe_tool, and lomway_call_tool;
- deferred search/describe reads the exact currently registered tool
  definitions from the raw proxy catalog and filters them by configured
  deferred backend prefixes;
- lomway_call_tool accepts only exact names currently listed under a
  configured deferred prefix and forwards the backend result without retry;
- configured optional backends that were offline at startup remain watched by
  the reconnect monitor and are adopted after their external supervisor makes
  them reachable;
- the upstream proxy admin backend remains removed and cannot be invoked
  through the meta-tool.

Backend process lifecycle remains owned by Swibo. Deferred exposure does not
start or stop a backend.

## Consequences

- Web Chat sees a stable, smaller schema set and does not need same-turn tool
  catalog refresh.
- Frequently used tools keep native typing and direct selection.
- Low-frequency tools need one search/describe step before generic invocation.
- Discovery is registration-aware rather than a separate backend-health oracle;
  an offline registered backend still fails normally at call time.
- Deferred tools remain protected by a configured-prefix plus current-catalog allowlist rather than an
  arbitrary tool-name router.
- The public schema gains per-backend exposure = "direct" | "deferred", with
  direct as the compatibility default.
