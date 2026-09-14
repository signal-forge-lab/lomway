# Lomway — Public Generalization Task Plan

Status: READY FOR MULTI-AI EXECUTION

Use `PUBLIC_GENERALIZATION_TASKS.yaml` for machine-readable assignment and progress tracking. This Markdown file remains the human-readable source for execution intent and acceptance detail.

This backlog is structured so another AI can continue from repository state without relying on chat history.

## Shared rules

Preserve the current production baseline; publish no private identifiers or secrets; keep backend lifecycle out of Core; keep OpenAI/Swibo/SOPS optional; never retry mutations automatically; run focused tests for each change; avoid unrelated refactors; preserve the six-backend real regression deployment.

## Parallel lanes

- Lane A: Config/Core model
- Lane B: Backend registry/namespace
- Lane C: Host/router/health
- Lane D: CLI/platform independence
- Lane E: Optional integrations
- Lane F: Tests/release/docs

After the public config model is fixed, most lanes can proceed in parallel.

## G0 — Baseline freeze

- **LMG-G0-01** Capture fmt/clippy/unit/mock/real-backend baseline and namespace counts. Acceptance: zero baseline failures and no secrets in artifacts.
- **LMG-G0-02** Document generalization branch/worktree and rollback policy.

## G1 — Configuration schema

- **LMG-G1-01** Add versioned public config structs for server, policy, and `Vec<BackendConfig>` with id/prefix/url/required. Acceptance: 0/1/N parse, strict unknown-key handling, serde tests.
- **LMG-G1-02** Split listener/backend URL/id/prefix/size/unsupported-feature validators without reducing current safety.
- **LMG-G1-03** Add a temporary legacy-config migration adapter without publishing private data.

## G2 — Backend registry

- **LMG-G2-01** Create validated runtime backend descriptors with no integration-specific types.
- **LMG-G2-02** Implement deterministic 0..N registry and remove hard-coded backend names.
- **LMG-G2-03** Implement required/optional startup probe semantics.

## G3 — Namespace/collision

- **LMG-G3-01** Define prefix and reserved-prefix policy.
- **LMG-G3-02** Precompute final public tool names and fail fast on collision while naming both sources.
- **LMG-G3-03** Add description/input-schema preservation regression tests.

## G4 — Gateway host

- **LMG-G4-01** Extract mcp-proxy composition into a gateway builder while removing the upstream proxy control backend.
- **LMG-G4-02** Expose only `/mcp`, `/healthz`, and `/readyz`; admin routes remain inaccessible.
- **LMG-G4-03** Separate liveness and readiness with required/optional backend semantics.

## G5 — CLI/platform independence

- **LMG-G5-01** Implement `serve`, `check`, `list-backends`, and `version` with actionable exit codes and no PowerShell requirement for binary execution.
- **LMG-G5-02** Define deterministic config path precedence without private user paths.
- **LMG-G5-03** Remove required Windows-only path/process behavior from Core.

## G6 — Optional integrations

- **LMG-G6-01** Package OpenAI Secure MCP Tunnel scripts/docs as optional integration; Core requires no credentials/client.
- **LMG-G6-02** Package a Swibo template separately with no machine-specific absolute path.
- **LMG-G6-03** Package SOPS separately; Core also works with ordinary env/config.
- **LMG-G6-04** Reduce PowerShell helpers to wrappers around Core CLI.

## G7 — Portable testing

- **LMG-G7-01** Public mock fixtures for 1/N backends, optional/required failure, collision, and timeout.
- **LMG-G7-02** Exactly-once mutation regression proves counter remains one on timeout/error paths.
- **LMG-G7-03** Portable real-backend test with at least two redistributable/public fixtures and no private credential.
- **LMG-G7-04** Preserve the current six-backend/OpenAI/Swibo deployment regression; no destructive migration without separate authorization.

## G8 — Security/supply chain

- **LMG-G8-01** Threat-model untrusted metadata/schema, oversized args, SSRF, non-loopback exposure, credential leakage, namespace spoofing, and replay/retry risk.
- **LMG-G8-02** Scan the public candidate set for user paths, emails, live IDs, API keys, and private keys.
- **LMG-G8-03** Review the exact lockfile for vulnerabilities/licenses; unresolved high/critical findings block release.

## G9 — Packaging/CI

- **LMG-G9-01** Clean Windows binary + example config smoke for `check` and `serve`.
- **LMG-G9-02** Locked fmt/clippy/test/build CI; add Linux/macOS after Core portability is proven.
- **LMG-G9-03** Release version/changelog/checksums/license/source/examples.

## G10 — Documentation

- **LMG-G10-01** Quickstart with two arbitrary backends and no Swibo/OpenAI/SOPS.
- **LMG-G10-02** Separate guides for OpenAI Secure MCP Tunnel, Swibo, SOPS, and Windows helpers.
- **LMG-G10-03** Migration and rollback guide for the current deployment.

## G11 — Final release gate

- **LMG-G11-01** PASS requires zero functional regression, clean-machine setup, current deployment regression, security/privacy gate, docs matching actual CLI/config, no mandatory integration in Core, and a clean intended release tree.

## Dependency summary

```text
G0
 -> G1
    -> G2 -> G3 -> G4
    -> G5
       -> G6
       -> G9
G2/G4 -> G7
G4 -> G8
G6/G7/G8/G9 -> G10 -> G11
```

## Handoff template

```text
Task ID:
Status: PASS | BLOCKED | PARTIAL
Files changed:
Behavior changed:
Tests run + result:
Security/privacy impact:
Compatibility impact:
Known follow-ups:
Do not infer / unresolved decisions:
```

