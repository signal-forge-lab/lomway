# OAuth client authorization and DCR capacity protection

## Goal
Preserve the running, previously authorized MCP clients while denying newly
registered clients blanket MCP tool execution rights. Bound public DCR writes
and state growth without invalidating refresh replay evidence.

## Acceptance criteria

1. Public DCR POST has a process-wide fixed-window ceiling of 20 attempts
   per rolling 60 seconds. An exceeded request returns HTTP 429 + Retry-After.
2. At most 128 clients can be registered in the state adapter, enforced in its
   serialized update queue. Never evict a client with an existing grant.
3. Unapproved registrations older than 48 hours are deleted on subsequent
   registration only when no grant or active credential references them.
4. All new and unrecognized OAuth clients get only tool discovery/read-only
   metadata permission. An immutable operator-issued snapshot of previously
   approved client IDs preserves existing client privileges through cutover.
5. OAuth and PAT callers both pass through the same fail-closed tool allowlist
   enforcement. A restricted client cannot invoke deferred targets not named
   in its allowlist.
6. Audit events use pseudonymous actor digests, tool names and decisions.
   Never log bearer tokens, tool arguments or raw client IDs.
7. Garbage collection covers only short-lived expired state buckets, with a
   5-minute grace period. Refresh tokens, grant data, and revocation evidence
   must be preserved.
8. Expired/revoked PAT records are reclaimed only when issuing new PATs, so
   the 128-record limit cannot be exhausted by revoked disposable probes.
9. Unit tests, type checking, clippy, Rust integration, browser flow, privacy
   scan, and GitHub CI must pass before production cutover.
10. A trusted operator may explicitly issue a full-tool PAT using
    `-AllTools`. This covers all current and future ordinary direct/deferred
    tools, but excludes proxy administration. All normal HTTP method,
    batch, introspection and revocation constraints continue to apply.

## Non-goals in this rollout

- Forcing 90-day reauthorization on already connected OAuth clients.
- Changing login failure limits in ways that reduce brute-force resistance.
- Deploying an unprovisioned shared introspection secret.
- Automatically increasing privileges for newly registered clients.
