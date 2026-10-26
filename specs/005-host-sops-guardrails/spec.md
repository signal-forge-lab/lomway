# Host SOPS mutation guardrails

## Requirements

1. Keep SOPS/age as the canonical encrypted secret store and never return the
   secret value to AI-facing outputs.
2. Retain read-only `Status` behavior without a mutation policy.
3. Fail closed on mutations unless the non-secret, host-local v1 policy
   explicitly authorizes the exact key, provider and (when applicable)
   rotation, PAT label, max lifetime and tool names.
4. Serialize cooperating writers with a common exclusive create-only lock.
   Never automatically break a stale lock.
5. Retain encrypted pre-change backups with the original ACL on successful
   mutation. No plaintext files.
6. Report unresolved PAT revocation as degraded without claiming a
   cross-store atomic transaction.
7. Do not change production secrets, IAM, OAuth, PATs or tunnel access as part
   of the software update or tests.

## Security boundary

The local policy and lock are operational guards only. They do not sandbox an
AI that can run arbitrary code as the same Windows user. A separate identity
and reviewed, restricted IPC broker would be required for that stronger model.
