# Operations

Updated: 2026-10-07

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

## 10. Scoped personal access tokens for non-interactive clients

For AI hosts that cannot perform browser OAuth, issue an **individually
revocable PAT**. Existing OAuth clients continue unchanged.

```powershell
# The one-time secret goes to the local clipboard, not the console.
pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label external-ai -Days 30 -Tools 'lomway_search_tools,lomway_describe_tool'
# Deferred invocation requires both wrapper AND nested target to be allowed.
pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label browser-ai -Days 30 -Tools 'lomway_call_tool,chrome_devtools_list_pages'
# Explicit full-access PAT for a trusted client (ordinary direct + deferred tools).
pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label hark -Days 7 -AllTools
pwsh -NoProfile -File scripts/pat.ps1 -Action list
pwsh -NoProfile -File scripts/pat.ps1 -Action revoke -Id '<printed-token-id>'
```

Enter the PAT only in the other host's dedicated credential field, never
in chat, shell arguments, screenshots, or shared logs. The PAT audience is
`https://mcp.maiteneru.com/mcp`, its scope is `devspace`, and its tool
permissions use exact-name entries by default. The explicit `-AllTools`
option grants access to all ordinary direct and deferred tools, including
new ones added later, but never to reserved `proxy/` administration tools.
It does not enable MCP resources, batch requests or GET calls. Maximum expiry
is 90 days. Only SHA-256 digests and metadata are persisted in
`%LOCALAPPDATA%/Lomway/oauth-sidecar/runtime/pat-tokens.json` outside Git.
Revocation takes effect on the next introspection, not during in-flight
requests. PATs do not have refresh tokens and must be reissued at expiry.

To deploy the PAT-aware Gateway, first run
`pwsh -NoProfile -File scripts/deploy-gateway-isolated.ps1 -PreflightOnly`,
then run `pwsh -NoProfile -File scripts/deploy-gateway-isolated.ps1`
on Windows via an independent management plane. The legacy
`deploy-pat-gateway.ps1` entrypoint delegates to the new implementation.
Only the Gateway process is stopped: desktop-local (port 17778), OAuth,
Swibo, and Tunnel stay running. The new Gateway executable is immutable and
versioned under `runtime/bin`; the shared `target/release/lomway.exe` is
never overwritten. A pointer in `runtime/gateway-binary-path.txt` selects
the executable, and rollback restores the previous pointer and restarts the
Gateway. Confirm `runtime/pat-deployment.json` is `phase=success`, then
perform a live PAT authorization test before issuing an external credential.

Run `pwsh -NoProfile -File scripts/test-pat-live.ps1` and repeat with
`-Public` for the public HTTPS MCP endpoint. Each test issues a one-day,
single-read-only-tool disposable PAT in process memory, completes MCP
initialize and initialized notification, verifies tools/list, allowed and
denied tool calls, and checks HTTP 401 after revocation. It does not print
or copy the credential.

`scripts/test-pat-codex-client.ps1` separately probes a real Codex AI
client using an ephemeral, one-tool bearer PAT. It uses a bounded deadline
and revokes the temporary PAT even when the model does not respond. This
test requires a working Codex model session; HTTP checks alone cannot prove
that an independent AI invoked a tool.
The script now verifies a successful simple Codex model response *before*
issuing the PAT, so transient CLI/model failures never create credentials.

## 11. OAuth sidecar cutover (offline-prepared, live-gated)

The Lomway-owned authorization sidecar is loopback-only, binds
`127.0.0.1:7677` directly, and is supervised by Swibo. Do not start a second
authority or forwarding process for the same public issuer.

OAuth lifetime policy:

- Access tokens are short-lived (normally one hour).
- Refresh tokens have a 30-day lifetime and rotate on refresh.
- Grants start at 30 days and are extended to the newly issued refresh token's expiry whenever a refresh token is issued or rotated.
- Refresh tokens issued by the current provider are not bound to browser Session TTL. A legacy refresh token is unbound only after its live Session and Grant are validated.
- Active clients therefore remain valid on a sliding basis, while clients unused for 30 days expire naturally.
- A client whose grant has already expired and whose refresh token is gone is not resurrected; it must authenticate once again.

Security hardening:

- Public DCR POST requests are limited to 20 per rolling minute per Sidecar
  process. The persistent adapter also caps all clients at 128; registrations
  never approved within 48 hours are pruned on a later registration if no
  active authorization refers to them. Historic clients without a registration
  timestamp are preserved rather than guessed about.
- New OAuth clients are restricted to `lomway_search_tools` and
  `lomway_describe_tool`. Unlike scoped PATs, existing OAuth clients need
  a one-time compatibility snapshot before the upgraded Gateway is deployed:
  `pwsh -NoProfile -File scripts/provision-legacy-oauth-clients.ps1 -VerifyOnly`,
  followed by the same command without `-VerifyOnly`. The owner-only
  `%LOCALAPPDATA%/Lomway/oauth-sidecar/runtime/client-tool-policy.json`
  stores only SHA-256 client IDs. Existing clients with active grants are
  preserved; new clients are never auto-exempted. Operators can replace the
  legacy exemption for a specific hashed client ID with an exact
  `clientToolAllowlists` entry.
- Roll out the updated Sidecar first using
  `scripts/deploy-oauth-csp.ps1 -Hardening` only after staging the prior
  `server.js` as `runtime/oauth-rollback/server.pre-004.js`, then deploy
  the immutable Gateway executable via
  `scripts/deploy-gateway-isolated.ps1`. Preserve independent Workbridge
  recovery throughout; these operations are not all-service restarts.
- Gateway logs structured authorization decisions with pseudonymous actor
  digests and sanitized tool names; bearer credentials and arguments are
  never logged. Per-client policies apply to both direct and deferred calls.
- The serialized adapter sweeps expired short-lived tokens, interactions and
  replay markers only after a 5-minute grace period. Refresh replay tokens,
  grants and grant-revocation tombstones are not swept. Revoked/expired PAT
  records are reclaimed when issuing the next credential.
- Owner login validates the interaction before async password verification and applies a uid-independent, owner-wide rate limit.
- Login and consent are distinct steps; consent displays client ID, unverified name, redirect URI and scopes, requiring explicit approval.
- Chromium can block the OAuth redirect chain if the interaction form has
  a self-only CSP. `form-action` therefore allows only the local origin and
  the origin of the OIDC-validated redirect URI. Exercise the full login,
  informed consent, code exchange and token-introspection path in isolated
  Chromium with `python scripts/test-oauth-browser.py` (synthetic credentials).
- Opaque AccessToken, RefreshToken and AuthorizationCode IDs are stored as hashes; a legacy state file is migrated on the first adapter read.
- The OAuth runtime directory and files are restricted at startup to owner, SYSTEM and Administrators.
- Startup only verifies those ACLs. The one-time ACL provisioning script must
  be run separately under the Windows owner context; Swibo does not reapply
  ACLs because Set-Acl may require unavailable SeSecurityPrivilege.
- Internal `/oauth/introspect` rejects Cloudflare-proxied public requests, while public provider introspection is disabled.
- Concurrent refresh-token consumption revokes the grant family on reuse and prevents reviving it through a later token save.

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
3. Route the actual public `/auth`, `/token`, `/reg`, and
   `/token/revocation` paths to the sidecar without changing the public issuer.
   Block the internal `/oauth/introspect` at the public tunnel too.
4. Verify Lomway returns `401` with its protected-resource challenge for
   missing/invalid credentials and `403` with `insufficient_scope` for an
   otherwise valid token missing the required scope.
5. Prove an authenticated non-Workbridge backend call, then perform the
   real-client refresh and tunnel checks. These live checks are not performed
   by the repository test suite.

Rollback is one authority only: restore the prior Workbridge route before
restarting it, and do not run both authorities for the same issuer.
