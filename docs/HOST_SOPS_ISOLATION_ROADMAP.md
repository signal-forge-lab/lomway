# Host SOPS privilege isolation: staged migration

## State as of 2026-10-26

HostSops suppresses plaintext tool output. However an arbitrary command
runner executing as the same Windows user can still directly invoke SOPS/age.
The local per-key policy guards against mistakes; it is not an OS security
boundary. The inspected coding-tools process was non-administrator, but this
alone does not prove that no elevation path exists.

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

## Remaining cooperating writers

The shared lock currently covers HostSops and Sopshelf only. Direct SOPS
scripts such as `provision-backend-token.ps1` and Praxiom provisioning must
be inventoried and migrated individually when they target the same canonical
store. This document does not change service accounts, MCP exposure, secrets
or active credentials.
