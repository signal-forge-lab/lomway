# Plan

- Put policy checks ahead of secret generation and PAT issue.
- Require an exact, typed policy entry for every authorized mutation.
- Implement the lock with OS-exclusive file creation beside the SOPS store.
- Keep `File.Replace` backups after successful mutations; discard incomplete
  temporary stage files.
- Retry old-PAT revocation; signal unresolved overlap explicitly.
- Use disposable synthetic SOPS stores, test age keys, and isolated PAT stores.
- Document the rollout limitation: no real policy is installed automatically,
  so production mutations are denied until an operator selects the allowed
  key and permissions. Read-only status stays available.
