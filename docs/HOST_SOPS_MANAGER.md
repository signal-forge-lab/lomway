# Host-local SOPS management

Reusable Windows script: scripts/Invoke-HostSops.ps1

The interface accepts only non-secret parameters such as key name and issuance
policy. Secret generation, encrypted SOPS updates and readback validation happen
inside the MCP host. Responses never include plaintext or token digests.

## Example

~~~powershell
pwsh -NoProfile -File scripts/Invoke-HostSops.ps1 -Action Status -Key DSH_LOMWAY_PAT -Provider LomwayPAT
pwsh -NoProfile -File scripts/Invoke-HostSops.ps1 -Action Generate -Key EXAMPLE_HOST_TOKEN -Provider RandomHex
~~~

LomwayPAT also supports Generate with Label, AllowedTools and Days. Rotating an
existing key requires the explicit Rotate switch. PAT issuance, encrypted storage
and revocation of the previous credential stay on the host.

The canonical SOPS path is fixed. Updates use encrypted staging, an ACL check,
concurrent-modification detection, readback verification and atomic replacement.
Failure output contains only an error type and source line, not secret contents.

**Limit:** this protects tool responses, not all code running under the same
Windows account. A general unrestricted shell in the same account can independently
decrypt SOPS. Strong isolation requires a separate OS identity plus a restricted
IPC/MCP interface. Plaintext exists transiently in process memory.

The script does not bypass platform-level operation restrictions.

## Validation (2026-10-25)

Six end-to-end tests PASS with real SOPS 3.13.3, disposable age keys and
isolated USERPROFILE/LOCALAPPDATA. Tests cover encrypted key presence, random
generation/rotation, PAT issue/rotation and revocation, invalid key names,
forbidden tool lists without mutation, and key presence without decryption.
No production credential was changed.

The tests exposed an ACL difference on the second update. The manager now
copies the canonical ACL onto staging and checks permission equivalence,
normalizing only Windows' auto-inherited DACL metadata marker. It never
relaxes access rules to make the update succeed.

Test command: py -3.14 -m pytest -q tests/test_host_sops_manager.py

## 2026-10-26: Mutation allowlist and cooperative locking

The read-only `Status` action remains available. `Generate` now requires
`%USERPROFILE%\.config\sops\host-sops-policy.json`. A missing policy,
unknown key, provider mismatch, unapproved rotation or PAT configuration fails
closed. Set `version: 1` and key-specific entries under `keys`; each entry
has `provider` and `allowRotate`. A `LomwayPAT` entry also requires
`label`, `maxDays`, and the exact `allowedTools` set.

Example with **synthetic names only**:

```json
{
  "version": 1,
  "keys": {
    "EXAMPLE_HOST_TOKEN": {
      "provider": "RandomHex",
      "allowRotate": false
    },
    "EXAMPLE_SCOPED_PAT": {
      "provider": "LomwayPAT",
      "allowRotate": false,
      "label": "example-agent",
      "maxDays": 7,
      "allowedTools": ["lomway_search_tools", "lomway_describe_tool"]
    }
  }
}
```

HostSops and Sopshelf share the exclusive create-only
`.config\sops\secrets\.sops-write.lock`. No automatic removal of abandoned
locks is attempted; investigate whether a writer is active before manually
clearing one. Other SOPS writers are not automatically covered.

Each successful mutation preserves the previous **encrypted** document as
`global.sops.json.backup-<UTC>-<id>.sops.json`, with ACL-preserving
`File.Replace`. This history is deliberately retained for operator recovery.

PAT rotation attempts old-token revocation up to three times after the new
encrypted value commits. If revocation fails, the command returns
`HOST_SOPS_PAT_REVOCATION_PENDING`. Both PATs may remain valid and require
explicit reconciliation; there is no false atomicity claim across PAT and SOPS.

**Security boundary:** this per-user allowlist is operational protection,
not isolation from arbitrary code executing as the same Windows user. A truly
restricted AI interface needs a separate OS identity, restrictive ACLs, and
an approved narrowly scoped IPC broker.

A **non-active draft** corresponding to the current 16-tool DSH scope is at
`docs/HOST_SOPS_POLICY_DRAFT_DSH.json`. It includes the powerful
`praxiom_praxiom_execute` permission, so `allowRotate=false`. The draft
is never copied to the production policy location automatically. Review and
approve the intended scope before enabling mutation or rotating a live PAT.
The independent Praxiom preview-token issuer uses the shared file lock and
checks the same per-key policy before creating a missing token. Its permission
is not included in this DSH-only draft; add it only after operator approval.
