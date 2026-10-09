# PAT development tasks

- [x] Inspect existing OAuth, gateway and MCP tool surfaces
- [x] Preserve/push diverged main histories after regression verification
- [x] Implement named PAT generation, hash-only persistence, listing and revocation
- [x] Integrate PAT with OAuth sidecar introspection
- [x] Add PAT-specific tools/list and tools/call restrictions
- [x] Add explicit nested deferred-tool authorization
- [x] Add synthetic unit and negative-security tests
- [x] Complete final full test suite (46 sidecar PASS; 86 Rust PASS, one opt-in live test ignored)
- [x] Deploy isolated Gateway binary without interrupting desktop-local, OAuth or the management plane
- [x] Verify live Gateway/Sidecar health, Swibo READY, public OAuth 200, public unauthenticated MCP 401, and public introspection 404
- [x] Validate live PAT initialize, notifications/initialized, tools/list filtering, permitted and forbidden tool calls, and revocation (loopback and public URL)
- [x] Revoke all disposable PATs and confirm zero active credentials
- [ ] Validate another AI's actual token-based MCP connection (requires user action)
- [x] Prepare an isolated Codex CLI test with disposable PAT and forced timeout/revocation
- [ ] Obtain a completed external model tool call; initial Codex CLI attempt was unresponsive and the PAT was revoked
- [x] Diagnose the Codex CLI model independently of MCP: doctor authentication/connectivity healthy, but basic model turns timed out even with an explicit model
- [x] Refuse to mint test PAT until the independent Codex model preflight completes; verified no new PAT record was created on timeout
