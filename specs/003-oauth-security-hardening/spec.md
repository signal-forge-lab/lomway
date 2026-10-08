# OAuth Security Hardening

## Context

Security review found owner-login brute-force bypasses, missing informed
consent, non-atomic refresh consumption, unintended browser-session binding,
plaintext bearer IDs in runtime JSON, and public access to an internal
introspection endpoint.

## Acceptance criteria

1. Unknown interactions cannot trigger password hash computation; login
   failures share an owner-wide, bounded rate budget, even across fresh uids
   and simultaneous requests. Password hashing is asynchronous.
2. Authentication and consent are separate, explicit steps. Consent displays
   unverified client name, exact client ID, redirect URI, and scope using
   HTML escaping, with browser anti-framing/cache headers.
3. At most one concurrent consumption of a one-time credential succeeds.
   A detected refresh reuse revokes the whole grant family and racing token
   issuance cannot revive it.
4. New refresh tokens use a sliding 30-day Grant and do not depend on the
   short-lived browser Session. Legacy refresh tokens are unbound only while
   their session/grant relationship still validates.
5. Bearer credentials are not persisted as plaintext JSON keys or jti
   values. Existing tokens migrate without client-visible replacement.
6. Machine-local OAuth state is ACL-restricted to owner, SYSTEM and
   Administrators. The internal introspection handler rejects Cloudflare
   traffic, and the public provider introspection feature is disabled.
7. Existing PAT, OAuth, MCP, and backend recovery tests continue to pass.
8. No production deployment is accepted without backup/rollback and live
   verification of public/private introspection, OAuth login/consent, and
   scoped PAT tool filtering.

## Non-goals / follow-up

- No forced expiration cap on actively refreshed Grants is introduced without
  a separate policy decision.
- DCR provisioning quotas, session database replacement, reconnect polling
  optimization, and deferred catalog caching remain separately tracked.
