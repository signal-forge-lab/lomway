# Host SOPS privilege isolation: staged migration

## State as of 2026-10-26

HostSops suppresses plaintext tool output. However an arbitrary command
runner executing as the same Windows user can still directly invoke SOPS/age.
The local per-key policy guards against mistakes; it is not an OS security
boundary. The inspected coding-tools process was non-administrator, but this
alone does not prove that no elevation path exists.

## Read-only ACL preflight

Use `pwsh -NoProfile -File scripts/Test-HostSopsIsolation.ps1` to inspect
only ACL metadata for the encrypted source, age identity and PAT store.
No secret is decrypted or dumped, and output does not contain actual file
paths or account names. The result is always `NOT_VERIFIED` because ACL
inspection cannot prove a dedicated-identity broker is enforced.
`-RequireIsolated` returns exit code 2 while this remains unverified.

## Rollout gates

1. Inventory OS ownership and ACLs of the encrypted SOPS store, age identity,
   PAT store and active consumers without displaying secrets.
2. After operator authorization and encrypted backup/rollback preparation,
   configure a dedicated non-administrator service identity that exclusively
   owns the SOPS source and age identity.
3. Build a restricted IPC broker that authenticates the caller, binds exact
   approved key/action/PAT scope/expiry and requires human approval for
   privilege-increasing or destructive operations.
4. Never return plaintext, secret hashes, bearer tokens, or secret environment
   variables to an arbitrary AI caller.
5. Verify denied, approved, concurrent and crash-recovery paths against
   disposable synthetic identities and preserve existing Lomway connectivity
   during the rollout.

## Out of scope

- Claiming isolation from another script running as the same Windows user.
- Enabling an unrestricted AI shell to decrypt the underlying age identity.
- Letting an AI broaden its own authorization policy without approval.
- Migrating real age keys or revoking live PATs without rehearsed rollback.

## Coordinating writers updated on 2026-10-26

- Lomway HostSops
- Sopshelf GUI (a running instance loads revised code only on restart)
- Lomway backend-token provisioning
- Praxiom preview-token provisioning
- DSH PAT issuance delegates to HostSops

These cooperating **writer** paths use the same `.sops-write.lock`. An
unrelated direct SOPS CLI or newly added mutation path is not automatically
covered. Praxiom's standalone preview-token issuer does **not** enforce
HostSops's per-key **authorization** policy; locking is not authorization.
No service identity, MCP exposure or active production credentials have
been changed.
