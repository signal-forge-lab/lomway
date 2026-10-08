# PAT technical plan

1. Extend the already loopback-only sidecar introspection to recognize
   lompat_-prefixed tokens. The existing OAuth provider and tokens are
   unchanged. Validate fixed audience and devspace.
2. Store 256-bit random token SHA-256 digests and metadata in an external
   machine-local runtime JSON file, with atomic replacement and a bounded
   exclusive administrative edit lock. No plaintext PAT persistence.
3. Use the local scripts/pat.ps1 wrapper for issuance, listing and revocation.
   It copies newly issued PATs to the Windows clipboard and prints only ID,
   label and expiry.
4. Extend gateway introspection data with pat and allowed_tools fields.
   For PAT callers only, inspect JSON-RPC methods and filter tools/list
   responses in JSON or SSE form. Reject unsupported patterns fail-closed.
5. Require nested deferred tool names to pass the same explicit allowlist;
   never grant proxy control-plane methods.
6. Test with synthetic tokens, then restart OAuth sidecar and gateway via
   the existing lifecycle. Verify a live expiring and revoked PAT without
   leaking its value in logs.

## Tradeoffs

SHA-256 over a 256-bit random token is resistant to offline guessing without
additional key management; unlike passwords, PATs have uniformly high
entropy. The state file stores hashes, never bearer secrets. PAT revocation is
checked on every request via loopback introspection.

For the PAT route, tools/list buffering is bounded (4 MiB, 10 seconds).
If a client's transport produces unbounded SSE or unsupported framing, it
receives a fail-closed gateway error, and compatibility must be validated
without weakening tool filtering.
