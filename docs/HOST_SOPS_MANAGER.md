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
