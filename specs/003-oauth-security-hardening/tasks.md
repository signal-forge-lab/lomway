# Progress

- [x] Validate review findings against repository code and installed provider
- [x] Fix interaction validation, login rate limit, async hash, explicit consent
- [x] Add session-independent refresh and safe legacy migration
- [x] Add replay-safe consume and grant-family revocation
- [x] Hash bearer token IDs on disk; unit-test legacy migration
- [x] Restrict internal introspection and remove public introspection feature
- [x] Add Windows ACL protection script and validate on synthetic files
- [x] Update bilingual operations documentation and endpoint mapping
- [x] Pass sidecar unit/integration checks
- [x] Rust clippy/regression (86 PASS, one opt-in live test ignored) and staged-content secret scan
- [x] Deploy isolated, rollback-capable Gateway and smoke-test PAT over local and public MCP
- [ ] Validate a fresh real-user OAuth browser login after the new explicit consent flow (existing OAuth client access is working)
- [x] Restore OAuth after ACL startup failure; replace reapply with read-only ACL verification
- [x] Add local rollback-capable Gateway deployment script
- [x] Run isolated Gateway deployment, verify OAuth discovery + scope gate, and exercise PAT allow/deny/revocation
- [x] Commit/push core security-hardening and PAT implementation
