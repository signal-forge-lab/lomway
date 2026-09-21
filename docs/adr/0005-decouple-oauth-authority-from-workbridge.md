# ADR-0005: Decouple the OAuth authority from Workbridge

- Status: Accepted
- Date: 2026-09-22

## Context

Lomway is the public MCP resource server and aggregation boundary, but the current OAuth authorization authority is provided by Workbridge. The public ingress routes `/mcp` and the protected-resource metadata endpoint to Lomway, while authorization-server endpoints such as `/authorize`, `/token`, `/register`, and `/revoke` are handled by the Workbridge-owned OAuth authority. Lomway validates bearer tokens by calling the authority's loopback-only RFC 7662 introspection endpoint.

This works, but it creates an ownership inversion: the gateway-wide authentication path depends on one optional backend product. Workbridge can therefore become a required dependency for clients that need to reach unrelated backends through Lomway.

The current public Lomway source already owns the resource-server half of this contract: protected-resource metadata, bearer challenge handling, resource/scope checks, and token introspection. The missing ownership question is the authorization-server half.

## Decision

OAuth authority for the public Lomway endpoint should become part of the **Lomway authentication stack**, not a Workbridge responsibility.

The first implementation will use a **Lomway-owned, loopback-only authorization sidecar**. Swibo owns its lifecycle. This keeps the Rust gateway thin while moving gateway-wide authentication ownership out of Workbridge.

The first migration does **not** put a new authorization server directly inside the Lomway Rust process. An in-process authority may be reconsidered by a later ADR if a mature, reviewed Rust authorization-server dependency can satisfy the full MCP/OAuth contract without creating a large custom security subsystem.

The accepted boundary is:

1. Lomway remains the public MCP resource server and aggregation boundary.
2. A Lomway-owned sidecar owns the authorization-server role and local token authority.
3. The public OAuth issuer and public endpoint shape should remain stable across cutover; changing the internal process must not unnecessarily change the OAuth identity seen by clients.
4. Workbridge becomes an ordinary optional backend and no longer supplies gateway-wide authentication.
5. Swibo, not Lomway, supervises the sidecar lifecycle.
6. There is never more than one active authorization authority for one public issuer.
7. Proven Workbridge behavior is a compatibility baseline, not code to copy permanently into Lomway.

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
- client identification/registration compatible with MCP `2026-07-28`: pre-registered clients where applicable, Client ID Metadata Documents (CIMD) as the preferred open-client mechanism, and Dynamic Client Registration (DCR) as a backwards-compatible fallback;
- authorization code flow with PKCE S256;
- RFC 8707 `resource` validation on authorization and token requests and resource-bound token issuance;
- RFC 9207 authorization-response `iss` and matching metadata advertisement when enabled;
- access-token and refresh-token issuance;
- refresh-token rotation;
- token revocation;
- authorization approval and owner authentication;
- registered-client and token persistence;
- redirect-URI validation and authorization-attempt rate limiting;
- safe CIMD retrieval and validation, including SSRF controls, exact client-document identity checks, and exact redirect-URI matching.

If the authority is a sidecar, token introspection remains loopback-only. If the authority is in-process, the resource server should use an internal interface instead of an HTTP introspection hop.

### Workbridge

Workbridge no longer owns gateway-wide OAuth. Its MCP backend lifecycle and product-specific capabilities remain independent from Lomway authentication.

## Deployment shape options

### Option A: Keep Workbridge as the authority

Smallest immediate change, but preserves the current cross-product dependency and failure coupling. This is the rollback shape, not the target shape.

### Option B: In-process Lomway authority

Provides the smallest runtime topology and removes the introspection network hop. **Not selected for the first migration.** Current reviewed Rust options do not yet provide a sufficiently mature, reusable authorization-server component for this deployment without Lomway owning substantial security-sensitive application code.

### Option C: Lomway-owned sidecar authority

Keeps the gateway thin and removes Workbridge coupling at the cost of one additional local process. The sidecar must expose only the minimum authorization surface plus a loopback-only introspection interface. Swibo owns start/stop/restart. **Selected for the first implementation.**

## Implementation research

Research performed before accepting this ADR produced the following conclusions:

- MCP `2026-07-28` prefers Client ID Metadata Documents over Dynamic Client Registration. DCR remains a compatibility mechanism and must not be the only planned client-registration path.
- The same revision hardens authorization with RFC 9207 issuer validation and explicit authorization-server credential binding. The public issuer is therefore part of the migration contract, not a cosmetic URL.
- The official Rust MCP SDK has strong OAuth client support and useful authorization-server examples, but it does not provide a drop-in, production authorization-server component that covers Lomway's persistence, owner approval, rate limiting, CIMD fetch hardening, and migration requirements. Using the examples in-process would still make Lomway own substantial authorization-server application code.
- Newly emerging Rust authorization-server crates remain below the maturity bar for this security boundary. They may be re-evaluated later, but they are not the first-migration dependency choice.
- MCP TypeScript SDK v2 moved its v1 authorization-server helpers to a deprecated `server-legacy` package and explicitly recommends migrating authorization servers to a dedicated OAuth/IdP library. The existing Workbridge SDK v1 authority therefore should not simply become the permanent Lomway implementation.
- A mature dedicated authorization-server library in a sidecar is the preferred dependency shape. `oidc-provider` is a candidate because it covers RFC 8414 discovery, PKCE, DCR, revocation, introspection, RFC 8707 Resource Indicators, RFC 9207, and experimental CIMD support. It must pass a focused MCP `2026-07-28` compatibility/security spike before it is pinned; the ADR accepts the sidecar boundary, not an unreviewed library version.
- The current Workbridge implementation remains valuable as a behavioral and migration baseline: single-owner approval, strong random opaque tokens, resource/scope binding, refresh-token rotation, revocation, rate limiting, and durable OAuth state already work in the deployed environment.

The result is that Option C is accepted now. Library pinning is an implementation task gated by MCP conformance, ChatGPT interoperability, security review, and the repository's normal dependency-adoption checks.

## Compatibility contract with the current Workbridge authority

The ownership migration is not intended to redesign the user-facing login flow.

- The public MCP URL remains unchanged.
- The public OAuth issuer should remain unchanged.
- The existing authorization endpoint shape (`/authorize`, `/token`, `/revoke`, and DCR `/register` while compatibility requires it) remains available through the public ingress.
- The single-user approval experience remains an owner-password approval flow. Branding changes from Workbridge-owned authentication to Lomway-owned authentication, but the interaction model stays the same.
- The canonical owner credential moves out of Workbridge-owned state into the Lomway authentication secret boundary. Its value may be securely migrated so the user does not need to learn a new password; Workbridge must no longer be the source of truth.
- CIMD support and RFC 9207 `iss` are additive protocol-hardening changes. They should not remove DCR until supported clients no longer require it.
- Existing client registrations and token state should be migrated when that can be proven safe. If seamless token migration cannot be proven, one explicit reauthorization is acceptable, but silent credential breakage is not.

## Migration plan

1. **Characterize the current contract and its specification gaps.** Add integration tests for protected-resource metadata, authorization-server metadata, current DCR behavior, PKCE authorization, token issuance, refresh, revocation, introspection, resource binding, scope enforcement, restart behavior, and current HTTP status/challenge behavior. Do not freeze known obsolete behavior merely because it exists today.
2. **Run the sidecar dependency spike.** Verify MCP `2026-07-28` discovery, CIMD, DCR fallback, PKCE S256, RFC 8707 resource binding, RFC 9207 `iss`, refresh/revocation/introspection, persistence hooks, and owner-approval integration before pinning the authority library.
3. **Define state migration.** Explicitly cover registered clients, access tokens, refresh tokens, the client-registration integrity key, and the owner credential. Short-lived authorization codes are not migrated. If any durable credential is intentionally invalidated, require an explicit reauthorization plan.
4. **Create Lomway-owned state and secret boundaries.** OAuth state must live outside the Workbridge state directory; long-lived secrets resolve through Lomway's canonical external secret mechanism.
5. **Introduce the sidecar with the same public issuer.** Keep the Workbridge authority as a temporary rollback path only. Do not expose the sidecar's loopback introspection endpoint publicly.
6. **Run parallel compatibility tests, not dual production writers.** Only one authority may be active for a public issuer at a time.
7. **Cut over ingress routing.** Keep the stable public issuer/endpoints while changing the internal authorization route from Workbridge to the Lomway-owned sidecar. Verify full client identification/registration and authorization through the real ingress.
8. **Prove backend independence.** Stop Workbridge and confirm authenticated access to at least one non-Workbridge backend still succeeds.
9. **Prove refresh continuity.** Verify that a real ChatGPT connection survives access-token expiry by refreshing without manual reauthorization when the chosen client flow supports refresh tokens.
10. **Remove the compatibility path.** Delete Workbridge gateway-authority coupling after the rollback window closes; do not retain permanent dual-authority branching.

## Acceptance criteria

- A supported remote MCP client can identify/register by an MCP `2026-07-28`-compatible mechanism, authorize, obtain tokens, refresh, revoke, and call `/mcp` through a stable HTTPS public URL.
- CIMD is supported as the preferred open-client registration mechanism; DCR remains available as a compatibility fallback until explicitly removed by a later compatibility decision.
- The public authorization-server issuer remains stable across the Workbridge-to-Lomway authority cutover unless an explicit migration decision documents why it cannot.
- Workbridge can be stopped or absent without breaking OAuth for other Lomway backends.
- Unauthenticated `/mcp` requests return `401` with correct protected-resource metadata.
- Tokens are rejected when expired, revoked, or resource-mismatched. Missing/invalid authentication returns the correct `401` challenge; insufficient scope follows the current MCP/RFC 6750 `403 insufficient_scope` behavior rather than being frozen as a legacy `401` behavior.
- PKCE S256 is required for the authorization-code flow.
- Authorization and token requests enforce RFC 8707 resource binding.
- Authorization responses include RFC 9207 `iss` when the authority advertises `authorization_response_iss_parameter_supported`.
- Redirect URIs are allowlisted and validated.
- Failed owner-authentication attempts are rate-limited.
- Persistent OAuth state survives normal process restart, unless the approved migration explicitly requires reauthorization.
- A real ChatGPT OAuth connection can refresh after access-token expiry without depending on Workbridge.
- Secrets and token material are never committed or logged; canonical long-lived secrets remain external to the repository and are resolved through the existing secret mechanism.
- The core authentication design is tunnel-vendor-neutral. A tunnel provides HTTPS ingress; it does not own OAuth semantics.
- Swibo, not Lomway, remains the process supervisor for any separate authority process.

## Security constraints

- Keep Lomway and any authority/introspection listener loopback-only locally.
- Do not expose the introspection endpoint publicly.
- Treat CIMD retrieval as an SSRF boundary: HTTPS only, strict URL validation, no credential-bearing URLs, no unsafe redirect following, DNS/private-address defenses, bounded response size/time, exact `client_id` document-URL matching, and exact redirect-URI matching.
- Do not add automatic retries around token issuance, refresh, or revocation unless idempotency is explicitly defined.
- Do not log bearer tokens, authorization codes, refresh tokens, owner credentials, or client secrets.
- Keep authorization state out of the Git working tree.
- Preserve explicit resource and scope binding; do not accept a token solely because introspection reports `active=true`.
- Preserve issuer binding. DCR/pre-registered credentials must not be silently reused against a different authorization-server issuer.

## Implementation details still to resolve

- Which dedicated authorization-server library/version passes the focused MCP `2026-07-28` spike and repository dependency-review gate?
- Should the sidecar live in this repository as a sibling package/binary or in a separate public repository under the Lomway product scope? Either way, ownership remains Lomway and lifecycle remains Swibo's responsibility.
- What exact migration mechanism copies or re-establishes existing registered clients, access tokens, refresh tokens, and registration-integrity state without keeping Workbridge as a runtime dependency?
- Which current Workbridge OAuth settings are general gateway policy and which should remain Workbridge-specific?
- The current `openai-secure-mcp-tunnel` connection-mode name describes a transport that is no longer necessarily used. Should it be replaced by a transport-neutral mode as part of the migration?

These are implementation details, not blockers to the accepted ownership/process boundary. Any decision that changes the public issuer, introduces a second active authority, or moves process supervision into Lomway requires a new or amended ADR.

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
- The first implementation adds one supervised local process but avoids making the Lomway Rust gateway itself an identity-server codebase.

## References

- MCP Authorization Specification `2026-07-28`: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
- MCP `2026-07-28` release notes: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- Official Rust MCP SDK OAuth support: https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md
- MCP TypeScript SDK v1-to-v2 auth migration: https://ts.sdk.modelcontextprotocol.io/v2/migration/upgrade-to-v2
- `oidc-provider` implemented specifications: https://github.com/panva/node-oidc-provider
