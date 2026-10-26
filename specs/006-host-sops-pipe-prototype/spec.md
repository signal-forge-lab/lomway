# Synthetic-only HostSops named-pipe contract

## Purpose

Validate a metadata-only, strictly bounded IPC exchange **before** the
production secret broker is created under a separate Windows user.

## Requirements

- Test executable must refuse production execution and the actual Windows
  profile. It requires explicit synthetic mode, a test age identity and a
  synthetic USERPROFILE, and only accepts names under `iw-sops-test-*`.
- Named pipe ACL explicitly grants server and designated client identities.
  No TCP listener or HTTP endpoint is created.
- Incoming request schema is exactly `version=1, action=Status,
  provider=RandomHex, key=UPPER_SNAKE_NAME`, without extra fields.
- Reply is exclusively `{ok,present}` or `{ok:false,code:DENIED}`.
  No secret, decrypted text, token digest or provider exception is returned.
- Bound request length (2048 bytes), read deadline and request count.
- Disallowed actions such as Generate never call the HostSops mutation path.
- All real SOPS/age/PAT files and live IPC services are out of scope.

## Non-goals and explicit release blocker

This runs as the **same** user in tests. An ACL test with one Windows identity
cannot establish a separate-user security boundary. A production release
requires privileged service-account installation, independent secret ACLs,
authenticated calling identity and a two-identity negative-access test.
This prototype **must not** be deployed as a production broker.
