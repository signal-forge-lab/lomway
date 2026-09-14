# Operations

Updated: 2026-09-13

## 1. Ownership

The six backend MCP services remain independent Swibo targets. `Lomway` is one Swibo target whose lifecycle contains two ordered components:

```text
start: gateway -> Secure MCP Tunnel
stop:  Secure MCP Tunnel -> gateway
```

The gateway never starts/stops backend MCP processes.

## 2. Normal status

Swibo should report the gateway target as:

```text
state  = READY
server = READY
tunnel = READY
error  = empty
```

The validated lifecycle sequence is:

```text
READY -> restart -> READY -> stop -> STOPPED -> start -> READY
```

## 3. Direct runtime commands

These are useful for diagnosis or initial setup; normal lifecycle can be operated from Swibo.

```powershell
pwsh -NoProfile -File scripts/check.ps1
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/integration-smoke.ps1
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
```

One-time Tunnel creation/reuse (optional integration):

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
```

## 4. Start / stop ordering

Before starting the aggregate target, the six backend services should normally be READY.

```text
Start:
  backend services -> gateway -> Secure MCP Tunnel

Stop aggregate target only:
  Secure MCP Tunnel -> gateway
```

Stopping the aggregate target does not stop any backend.

## 5. Health and incident isolation

### Gateway process

`GET http://127.0.0.1:17777/healthz` reports gateway process readiness. It is intentionally independent of per-backend health.

### One backend fails

- leave gateway/tunnel running;
- repair the affected backend target;
- restart the gateway after the backend is healthy if it had been skipped during gateway construction;
- run `scripts/integration-smoke.ps1` to refresh/verify the aggregate catalog.

### Gateway fails

- restart only the `Lomway` Swibo target;
- do not restart all backends by default.

### Tunnel fails

- keep the gateway and backends running;
- `integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure` or Swibo restart restores the tunnel.

### XMind reports schema drift

Preserve the XMind Workboard fail-closed behavior. Review old/new capability fingerprints and schema diff, then accept/update the XMind cache only after the change is understood. Do not make the aggregate gateway normalize or hide upstream drift.

## 6. Configuration changes

v1 does **not** hot reload. Apply configuration changes with:

1. edit the local-only configuration/environment source;
2. run `scripts/check.ps1`;
3. restart the `Lomway` Swibo target;
4. verify `/healthz`;
5. run `scripts/integration-smoke.ps1`;
6. confirm Swibo server/tunnel are READY.

## 7. Rollback

The gateway does not alter backend domain state or source as part of aggregation. If the aggregate path must be disabled:

1. stop/disable the aggregate gateway target or connector;
2. restore the previously used individual connector/tunnel configuration as needed;
3. leave backend processes and Google Drive unchanged.

Old individual connector removal is a separate migration decision and is not required for this implementation to operate.

## 8. Logs and state

- gateway logs/runtime PID live under ignored local directories;
- Tunnel profile/health/logs are stored outside the repository by `tunnel-client`;
- do not commit either set;
- do not enable argument dumps or secret-bearing config dumps for routine diagnosis.

## 9. Upgrade gate

For any `mcp-proxy`, `tower-mcp`, FastMCP interoperability, or protocol change:

1. review source/release notes and config schema;
2. regenerate `Cargo.lock` only intentionally;
3. run fmt/clippy/unit/mock/fault tests;
4. run the six-real-backend test;
5. run aggregate integration smoke;
6. verify Secure Tunnel READY/MCP probe;
7. verify Swibo lifecycle;
8. rerun public-repository hygiene review.
