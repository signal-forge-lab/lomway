# Tasks

- [x] Add fail-closed key-specific mutation policy.
- [x] Add cooperative SOPS store write lock shared with Sopshelf.
- [x] Retain encrypted backup snapshots after commit.
- [x] Add PAT revocation retry and overlap metadata.
- [x] Add E2E negative tests for policy absence, unlisted keys, scope mismatch
  and occupied lock.
- [x] Re-run HostSops + Secret Consumer suite (44 PASS) and PowerShell
  syntax validation (0 parse errors); inspect staged diff for secrets.
- [x] Commit source/docs without changing production SOPS or PAT records.
- [ ] Separate Windows account + IPC broker remains a distinct architecture
  migration that is not covered by this code-only task.
