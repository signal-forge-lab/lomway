# Requirements

Updated: 2026-09-13

## 1. Functional requirements

| ID | Requirement | Status |
|---|---|---|
| FR-01 | Expose one Streamable HTTP MCP endpoint at `127.0.0.1:17777/mcp` | PASS |
| FR-02 | Aggregate Workbridge, Memory Gateway, Microsoft UFO, Stealth Browser, XMind Workboard and Praxiom | PASS |
| FR-03 | Namespace tools as `<backend>_<tool>` and prevent collisions | PASS |
| FR-04 | Keep healthy namespaces usable when another backend fails | PASS |
| FR-05 | Preserve backend schemas/results without semantic rewriting | PASS |
| FR-06 | Apply endpoint/config changes by local config + restart, without reinstalling gateway code | PASS |
| FR-07 | Provide loopback process health at `/healthz`; backend health remains independently supervised | PASS |
| FR-08 | Support one Local MCP connector/Tunnel for the aggregate endpoint | PASS |
| FR-09 | Keep Google Drive independent | PASS |
| FR-10 | Remove client-visible upstream control-plane MCP tools | PASS |
| FR-11 | Keep backend lifecycle outside the gateway | PASS |
| FR-12 | Fail closed on unsafe production policy | PASS |

## 2. Security requirements

- bind only to `127.0.0.1`;
- no `/admin/*` management surface is served externally;
- remove the upstream `proxy` MCP backend before serving;
- ChatGPT cannot dynamically register backend URLs;
- no secret values in source, config examples, logs or tool results;
- Secure Tunnel credentials come from the canonical external SOPS store;
- public Git content contains no user-specific absolute paths, live tunnel IDs, machine-local endpoint inventory, local config or runtime logs.

## 3. Reliability requirements

- automatic retries/hedging are forbidden;
- tool-call results are not cached;
- fan-out/failover/composite dispatch is forbidden;
- timeout does not cause a second mutation dispatch;
- one failed backend at startup may be skipped if at least one backend initializes successfully;
- if every backend fails initial construction, startup fails closed;
- skipped/recovered backends are rediscovered by gateway restart in v1.

## 4. Maintainability requirements

- backend domain logic stays in each backend;
- backend process ownership stays in Swibo/target wrappers;
- upstream `mcp-proxy` is exact-pinned rather than forked;
- resolved dependencies are locked;
- adding a normal backend is configuration-first;
- public docs maintain English/Japanese counterparts.

## 5. Acceptance criteria and evidence

| Criterion | Evidence | Result |
|---|---|---|
| Single endpoint | gateway `/mcp` + six real backend initialization | PASS |
| Namespace collision | mock E2E same-name `status` tools | PASS |
| Failure isolation | one failed startup backend skipped while healthy backend remains callable | PASS |
| Control-plane absence | aggregate `proxy_*` count = 0 | PASS |
| Admin-plane isolation | `/admin/*` and `/mcp/admin/*` return 404 | PASS |
| No duplicate mutation | forced timeout counter = 1 | PASS |
| Real backend parity | 12 + 32 + 19 + 97 + 21 + 2 = 183 tools | PASS |
| Representative routing | one safe call from every namespace through aggregate gateway | PASS |
| Secure Tunnel | native runtime READY and MCP probe/initialize succeeds | PASS |
| Supervisor lifecycle | Swibo READY/restart/STOPPED/start/READY sequence | PASS |
| Static quality | fmt, clippy `-D warnings`, full tests, release build | PASS |
| Public hygiene | final secret/path/runtime scan | required by final review |

## 6. Explicit non-requirements

- no LLM tool router;
- no generic `call_tool(name,args)` dispatcher;
- no new GUI;
- no Google Drive proxy;
- no public/LAN listener;
- no admin/metrics control plane in v1;
- no hot reload/reconciliation loop in v1.
