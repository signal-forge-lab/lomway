# Deferred browser MCP reconnect verification (2026-10-17)

## Incident

On the Windows host the **stealth-browser-mcp** backend was restarted via
Swibo at 2026-10-17 21:10 JST after verifying that it had no active
browser instances. The replacement listened on 127.0.0.1:7691.
Direct streamable-HTTP MCP initialize, tools/list (98 tools) and the
count-only get_capture_retention_stats tool succeeded with HTTP 200.
Lomway's deferred `browser_list_instances` invocation, however,
timed out from the connected tool surface.

The Lomway gateway (ports 17777 and 17778) returned HTTP 200 on
both healthz and readyz at 22:08 JST. Readyz is a cached readiness
signal and does **not** test each deferred backend's ability to
execute a tool. These facts do not establish the precise failure
location within Lomway, the calling MCP tunnel, or the client.

## Source inspection

`src/gateway/reconnect.rs` has an explicit deferred `activate`
path and a recovery monitor. Active connections are refreshed after
configured idle time, a known readiness failure or pending reconnect.
`src/gateway/deferred.rs` dispatches through `proxy.call`, with
backend-specific timeout layers applied to configured transports.
The individual deferred handler itself does not impose a separate
whole-call deadline. This is a potential latency/error-observability
limitation, **not a proved deadlock or the proved incident cause**.

Backend restarts use the supervisor/backend namespace name and do
not retry the user's tool call. Do not bypass that protection for
mutating browser tools, and do not restart the gateway just to hide
the original failure.

## Added regression coverage

Two tests were added to `tests/proxy.rs`:

- `deferred_backend_recovers_after_external_restart`: stop and restart
  the backend on the same port, preserve the caller and gateway,
  and require deferred tool invocation to recover without hanging;
- `deferred_backend_recovers_from_expired_mcp_session`: invalidate
  the old MCP session without a port outage, then require bounded
  calls and reconnection.

The two tests **PASS** on the checked-out source. The entire
proxy integration suite also **PASS: 19/19**. The live production
timeout is therefore **not reproduced by these controlled cases**.
No gateway binary, production config, credential, or running
process was changed.

## Remaining discriminating work

1. Capture sanitized start/end/error spans for `activate`,
   `current_tools`, and `proxy.call` in a reproducible invocation.
   Preserve tool IDs and durations but no arguments or bearer tokens.
2. Correlate against the currently deployed Lomway binary revision,
   which may differ from this checkout's Git HEAD.
3. Confirm whether the calling tunnel's timeout is shorter than the
   gateway's configured backend deadline.
4. Add a failing, specifically representative test **before**
   changing reconnect policies or introducing a whole-call timeout.

No automatic retries of non-idempotent browser actions are allowed.

## Later live recheck (approximately 23:16 JST)

Lomway's deferred browser discovery successfully returned a cached
`browser_list_instances` descriptor, and an immediate read-only
`lomway_call_tool` invocation returned `{"result":[]}` without a
timeout. This shows the browser deferred path can recover in the
current live session without restarting the browser backend.
It does not identify why earlier calls timed out or prove durable
reliability across future backend restarts. No code or production
config was changed for this recheck.

A separate previously advertised direct `workbridge_exec_command`
capability now returned `Unknown tool: workbridge_exec_command` from
the connected Lomway tool surface, while the browser deferred call
succeeded. Device-level Windows inspection continued via the
already-connected Desktop Commander on the Windows host. Distinguish this direct
tool registration mismatch from the earlier browser MCP timeout;
there is not yet evidence of a common cause.
