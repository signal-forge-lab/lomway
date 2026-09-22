# Operations

Updated: 2026-09-13

## 1. Ownership

The backend MCP services remain independent Swibo targets. `Lomway` owns the
gateway plus its OAuth authority sidecar. Swibo supervises the sidecar as a
separate `OAuth` component so its health is visible independently from the
gateway.

```text
start: OAuth sidecar -> gateway -> Secure MCP Tunnel
stop:  Secure MCP Tunnel -> gateway -> OAuth sidecar
```

The gateway never starts/stops backend MCP processes.
The OAuth sidecar binds directly to `127.0.0.1:7677`; no forwarding process is
part of the steady-state topology.

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
pwsh -NoProfile -File scripts/oauth-sidecar.ps1 -Action status
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
  backend services -> OAuth sidecar -> gateway -> Secure MCP Tunnel

Stop aggregate target only:
  Secure MCP Tunnel -> gateway -> OAuth sidecar
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
- OAuth sidecar logs/PID/state live under `%LOCALAPPDATA%\Lomway\oauth-sidecar`;
- OAuth signing material, cookie keys, and the owner credential are read only
  from `~/.config/sops/secrets/global.sops.json` at process launch;
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

## 10. OAuth sidecar cutover (offline-prepared, live-gated)

The Lomway-owned authorization sidecar is loopback-only, binds
`127.0.0.1:7677` directly, and is supervised by Swibo. Do not start a second
authority or forwarding process for the same public issuer.

Before cutover, validate the sidecar with synthetic state only, confirm the
stable public issuer and endpoint mapping, and run the repository's offline
fmt, clippy, test, build, and diff checks. Keep owner credentials, tokens,
state files, and SOPS material outside the repository.

During the later live cutover:

1. Stop or disable the Workbridge authority route while keeping Workbridge's
   optional MCP backend route available if needed.
2. Start the sidecar through Swibo on its loopback listener and verify its
   metadata, CIMD/DCR registration, PKCE S256 authorization, resource-bound
   token, refresh rotation, revocation, and loopback introspection behavior.
3. Route the existing public `/authorize`, `/token`, `/register`, and
   `/revoke` paths to the sidecar without changing the public issuer.
4. Verify Lomway returns `401` with its protected-resource challenge for
   missing/invalid credentials and `403` with `insufficient_scope` for an
   otherwise valid token missing the required scope.
5. Prove an authenticated non-Workbridge backend call, then perform the
   real-client refresh and tunnel checks. These live checks are not performed
   by the repository test suite.

Rollback is one authority only: restore the prior Workbridge route before
restarting it, and do not run both authorities for the same issuer.
