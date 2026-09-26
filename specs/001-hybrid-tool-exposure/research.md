# Research: Hybrid Tool Exposure

## Web Chat probe

A live probe replaced the Stealth Browser backend on port 7691 with a one-tool MCP server. Lomway's own `tools/list` changed from 97 `browser_*` tools to one `browser_status` tool, proving dynamic backend refresh inside Lomway. The active ChatGPT Web conversation continued to expose the original 97 browser schemas during the same response. The environment was then restored and Stealth Browser returned to READY.

Decision: do not depend on same-turn `notifications/tools/list_changed` refresh for Web Chat.

## Upstream capability

Pinned `mcp-proxy 0.4.3` contains global `tool_exposure = "search"`, BM25 discovery, and `proxy/call_tool`. It also registers search tools inside the upstream `proxy` admin backend. Lomway deliberately removes that entire backend to keep `proxy/config` and `proxy/add_backend` away from ChatGPT.

Decision: reuse upstream capability filtering and raw `McpProxy` invocation behavior, but expose Lomway-owned safe meta-tools rather than enabling the upstream admin backend.

## Minimal implementation

- Add per-backend public `exposure` with `direct` default and `deferred` option.
- Map deferred entries to upstream `hide_tools = ["*"]`. Upstream capability filtering then hides the tools and rejects direct calls.
- Keep a raw `McpProxy` clone for the Lomway-owned meta backend; like upstream search-mode `call_tool`, it bypasses the outer list/call filter only after Lomway's deferred allowlist check.
- Keep only the configured deferred backend ids/prefixes as the allowlist. Search/describe asks the raw `McpProxy` for its currently registered catalog so backend reconnects become visible without changing the Web Chat catalog. Registration is intentionally not a health promise: backend call failure remains the authoritative runtime signal. Search is a small deterministic name/description/schema matcher; no new search dependency is needed.
- Extend the existing reconnect monitor to keep watching configured loopback backends even when they were skipped at startup. It still never starts a process; it only adopts a backend after Swibo or another supervisor makes the endpoint reachable.
- Do not expose lifecycle or backend-add/remove operations.
