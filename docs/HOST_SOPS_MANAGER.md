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
