# ADR-0005: Decouple the OAuth authority from Workbridge

- Status: Proposed
- Date: 2026-09-22

## Context

Lomway is the public MCP resource server and aggregation boundary, but the current OAuth authorization authority is provided by Workbridge. The public ingress routes `/mcp` and the protected-resource metadata endpoint to Lomway, while authorization-server endpoints such as `/authorize`, `/token`, `/register`, and `/revoke` are handled by the Workbridge-owned OAuth authority. Lomway validates bearer tokens by calling the authority's loopback-only RFC 7662 introspection endpoint.

This works, but it creates an ownership inversion: the gateway-wide authentication path depends on one optional backend product. Workbridge can therefore become a required dependency for clients that need to reach unrelated backends through Lomway.

The current public Lomway source already owns the resource-server half of this contract: protected-resource metadata, bearer challenge handling, resource/scope checks, and token introspection. The missing ownership question is the authorization-server half.

## Proposed decision

OAuth authority for the public Lomway endpoint should become part of the **Lomway authentication stack**, not a Workbridge responsibility.

This is an ownership decision, not yet a process-layout decision. The implementation should choose the smallest reviewed shape that preserves Lomway's thin-gateway principle:

1. Prefer an in-process authority only if an existing reviewed library can provide the required OAuth/MCP behavior without introducing a large custom security subsystem.
2. Otherwise, extract or implement a generic Lomway-owned authorization authority as a separate loopback-only process. Swibo remains responsible for process lifecycle.
3. Do not duplicate the Workbridge OAuth implementation into Lomway. Reuse or extract proven behavior where practical.

After migration, Workbridge must be usable as an ordinary optional backend. Stopping or removing Workbridge must not disable authentication for other Lomway backends.

## Target responsibility split

### Lomway resource server

Lomway continues to own:

- `/mcp` bearer authentication and authorization;
- `/.well-known/oauth-protected-resource/mcp`;
- resource binding and required-scope enforcement;
- the public `WWW-Authenticate` challenge;
- integration with the selected Lomway-owned authority.

### Lomway authentication authority

The selected authority implementation owns:

- authorization-server metadata;
- dynamic client registration where required by supported MCP clients;
- authorization code flow with PKCE S256;
- access-token and refresh-token issuance;
- refresh-token rotation;
- token revocation;
- authorization approval and owner authentication;
- registered-client and token persistence;
- redirect-URI validation and authorization-attempt rate limiting.

If the authority is a sidecar, token introspection remains loopback-only. If the authority is in-process, the resource server should use an internal interface instead of an HTTP introspection hop.

### Workbridge

Workbridge no longer owns gateway-wide OAuth. Its MCP backend lifecycle and product-specific capabilities remain independent from Lomway authentication.

## Deployment shape options

### Option A: Keep Workbridge as the authority

Smallest immediate change, but preserves the current cross-product dependency and failure coupling. This is the rollback shape, not the target shape.

### Option B: In-process Lomway authority

Provides the smallest runtime topology and removes the introspection network hop. Accept only if the required OAuth behavior can be supplied by reviewed upstream/library functionality with a small Lomway interface. Reject if it turns the gateway into a large custom identity server.

### Option C: Lomway-owned sidecar authority

Keeps the gateway thin and removes Workbridge coupling at the cost of one additional local process. The sidecar must expose only the minimum public authorization endpoints plus a loopback-only introspection interface. Swibo owns start/stop/restart.

## Migration plan

1. **Freeze the current contract.** Add integration tests that capture protected-resource metadata, authorization-server metadata, dynamic registration, PKCE authorization, token issuance, refresh, revocation, introspection, resource binding, scope enforcement, and restart behavior.
2. **Choose the physical authority shape.** Review the pinned MCP/OAuth dependencies first. Prefer reuse over new OAuth implementation code.
3. **Define state migration.** Decide explicitly whether registered clients and refresh tokens migrate, or whether the migration intentionally requires one-time client reauthorization. Do not silently invalidate persistent credentials.
4. **Move authority ownership.** Introduce the Lomway-owned authority while keeping the current Workbridge authority as a temporary rollback path only.
5. **Run parallel compatibility tests, not dual production writers.** Only one authority may be active for a public endpoint at a time.
6. **Cut over the public endpoint.** Verify a full client registration and authorization flow through the real ingress.
7. **Prove backend independence.** Stop Workbridge and confirm authenticated access to at least one non-Workbridge backend still succeeds.
8. **Remove the compatibility path.** Delete Workbridge gateway-authority coupling after the rollback window closes; do not retain permanent dual-authority branching.

## Acceptance criteria

- A supported remote MCP client can register, authorize, obtain tokens, refresh, revoke, and call `/mcp` through a stable HTTPS public URL.
- Workbridge can be stopped or absent without breaking OAuth for other Lomway backends.
- Unauthenticated `/mcp` requests return `401` with correct protected-resource metadata.
- Tokens are rejected when expired, revoked, resource-mismatched, or missing the required scope.
- PKCE S256 is required for the authorization-code flow.
- Redirect URIs are allowlisted and validated.
- Failed owner-authentication attempts are rate-limited.
- Persistent OAuth state survives normal process restart, unless the approved migration explicitly requires reauthorization.
- Secrets and token material are never committed or logged; canonical long-lived secrets remain external to the repository and are resolved through the existing secret mechanism.
- The core authentication design is tunnel-vendor-neutral. A tunnel provides HTTPS ingress; it does not own OAuth semantics.
- Swibo, not Lomway, remains the process supervisor for any separate authority process.

## Security constraints

- Keep Lomway and any authority/introspection listener loopback-only locally.
- Do not expose the introspection endpoint publicly.
- Do not add automatic retries around token issuance, refresh, or revocation unless idempotency is explicitly defined.
- Do not log bearer tokens, authorization codes, refresh tokens, owner credentials, or client secrets.
- Keep authorization state out of the Git working tree.
- Preserve explicit resource and scope binding; do not accept a token solely because introspection reports `active=true`.

## Open questions for the implementation task

- Can the pinned MCP/OAuth stack supply a complete authorization server with less risk than extracting the current proven Workbridge implementation?
- If a sidecar is selected, should it live in this repository as a separate binary or in a separate public repository under the Lomway product scope?
- Should existing registered clients and refresh tokens be migrated, or is one explicit reauthorization acceptable?
- Which current Workbridge OAuth settings are general gateway policy and which should remain Workbridge-specific?
- The current `openai-secure-mcp-tunnel` connection-mode name describes a transport that is no longer necessarily used. Should it be replaced by a transport-neutral mode as part of the migration?

## Non-goals

- Multi-user identity management.
- Social login or enterprise identity federation.
- Replacing the external HTTPS tunnel provider.
- Moving backend lifecycle ownership into Lomway.
- Maintaining two active authorization authorities for the same public endpoint.

## Consequences

- Gateway authentication no longer depends on one backend product.
- Workbridge returns to being an optional backend from Lomway's perspective.
- Authentication ownership aligns with the public gateway seam while physical deployment can remain minimal.
- The migration requires security-sensitive compatibility and persistence tests before cutover.
