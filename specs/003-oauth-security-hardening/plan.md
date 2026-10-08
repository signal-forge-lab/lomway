# Implementation and verification plan

1. Fix the sidecar interaction handler: validate signed interaction before
   password; owner-wide limiter; async scrypt; explicit consent rendering and
   action; CSP/anti-framing/no-store; HTTP timeouts.
2. Keep oidc-provider's built-in OAuth semantics while setting
   expiresWithSession=false on newly issued credentials. Carefully migrate
   legacy Refresh Tokens only if owner Session and Grant still match and are
   unexpired.
3. Serialize all adapter state mutations; fail the second concurrent consume,
   revoke its grant family and persist a bounded revocation tombstone so a
   late racing upsert cannot reissue credentials.
4. Hash opaque bearer IDs for AccessToken, RefreshToken and AuthorizationCode
   records, strip their plaintext jti, and reconstruct only on adapter read.
   Migrate old records atomically on first read, after a protected backup.
5. Protect runtime files using Windows ACLs on sidecar start. Reject requests
   with Cloudflare proxy identity at internal introspection. Disable public
   introspection; keep OAuth discovery/registration/token/revocation unchanged.
6. Run sidecar verification, Rust tests/clippy, and offline-to-live checks.
   Deploy the Gateway PAT-aware build before accepting real PATs.

## Rollback

Keep an external, owner-only backup of the original state and previous
release binary. If sidecar or gateway health fails, restore the previous
binary/state with a single authority and verify normal OAuth MCP access.
