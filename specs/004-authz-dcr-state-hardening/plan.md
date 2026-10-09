# Implementation and staged rollout

1. Limit DCR POST in the Sidecar and cap registered Client model records in
   the persistent adapter. Mark new registrations to distinguish them from
   migrated pre-existing Clients; prune only unapproved, unreferenced records.
2. Add a local runtime policy `client-tool-policy.json` with SHA-256 client
   ID digests and per-client tool lists. Missing policy restricts all OAuth
   clients to discovery; malformed policy fails closed.
3. Freeze current active Grant client IDs (including CIMD clients without a
   persistent Client record) once using
   `scripts/provision-legacy-oauth-clients.ps1`. Reconcile only before the
   first hardening cutover, never expand this snapshot after deployment.
   Verify directory ACL and keep the file out of Git.
4. Extend OAuth introspection with policy fields and apply the same tool
   filtering to non-PAT clients unless explicitly marked trusted legacy.
5. Add structured authorization audit metadata and sweep safely expired
   transient state buckets on serialized adapter operations. Never sweep
   refresh replay evidence.
6. Run isolated tests and build in staging. Preserve immutable pre-deploy
   Gateway binary and old Sidecar JavaScript. Restart the Sidecar alone;
   validate existing clients, then switch only Gateway binary through
   `deploy-gateway-isolated.ps1`. Roll back in reverse order on failed checks.

Follow-up: optional, mandatory gateway-to-sidecar introspection authentication,
per-client refresh absolute limits, and login fairness remain independent
design changes that need compatibility/secret management planning.
