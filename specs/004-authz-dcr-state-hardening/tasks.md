# Tasks

- [x] Verify current repository, OAuth and PAT tests, active state shape
- [x] Add DCR registration burst gate and model capacity/TTL constraints
- [x] Implement policy-driven OAuth client tool rights and legacy snapshot tool
- [x] Add bounded state and PAT maintenance, audit hooks
- [x] Add explicit PAT full-tool option with admin boundary tests and CLI
- [x] Complete local Rust lint/integration, privacy checks and Chromium
- [x] Provision frozen runtime legacy client snapshot (four active grant clients, including CIMD) and verify ACL
- [x] Deploy reviewed Sidecar and isolated, versioned Gateway binary (2026-10-10 production receipts: `oauth-hardening-deployment.json` and `pat-deployment.json`, both `phase=success`; rollback path retained without overwriting active binaries)
- [x] Verify live Gateway / Sidecar readiness, public OAuth discovery, unauthenticated MCP challenge, internal introspection isolation, and existing ChatGPT-connected Workbridge tool execution
- [x] Verify full-access PAT over local and public MCP (`initialize`, `tools/list`, allowed call, denied admin call, revoke) with disposable tokens; no long-lived PAT issued
- [ ] Verify other legacy OAuth clients (such as Spark/Paperclip) from their originating applications when accessible; a preserved policy snapshot alone is not proof of successful live use
- [ ] Plan optional introspection secret and refresh policy separately
