# PAT development tasks

- [x] Inspect existing OAuth, gateway and MCP tool surfaces
- [x] Preserve/push diverged main histories after regression verification
- [x] Implement named PAT generation, hash-only persistence, listing and revocation
- [x] Integrate PAT with OAuth sidecar introspection
- [x] Add PAT-specific tools/list and tools/call restrictions
- [x] Add explicit nested deferred-tool authorization
- [x] Add synthetic unit and negative-security tests
- [x] Complete final full test suite (46 sidecar PASS; 86 Rust PASS, one opt-in live test ignored)
- [ ] Live sidecar/gateway lifecycle verification and public/private introspection probes
- [ ] Run scripts/deploy-pat-gateway.ps1 locally on Windows; detached remote launch is blocked by host safety policy
- [ ] Validate another AI's actual token-based MCP connection (requires user action)
