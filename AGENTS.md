# Repository Guidance

## Purpose

This repository owns the local MCP aggregation boundary only. It must remain a thin, infrastructure-level component between ChatGPT's Secure MCP Tunnel and independently managed local MCP backends.

## Design constraints

- Prefer configuration and the pinned upstream `mcp-proxy` library over custom proxy logic.
- Add custom Rust only where a documented requirement cannot be met safely by upstream behavior.
- Do not move backend domain logic into this repository.
- Do not make this gateway the backend process supervisor; Swibo or another existing supervisor owns backend lifecycle.
- Google Drive stays outside this gateway.
- Default listener is loopback only. Never change to `0.0.0.0` or a LAN address without an explicit security review and user approval.
- Keep MCP control-plane/admin tools out of the client-visible tool catalog. In particular, `proxy/config` and `proxy/add_backend` must never be exposed to ChatGPT in the production profile.
- Do not enable automatic retries, hedging, or tool-call response caching unless tool-level idempotency is explicitly modeled and tested.
- Preserve upstream tool schemas and results by default; avoid wrapper-specific argument/result reshaping.

## Secrets and local state

- Never commit API keys, bearer tokens, tunnel credentials, private keys, local machine paths, local config, logs, or generated runtime state.
- The canonical secret source is the user's external global SOPS store. Resolve secrets through SOPS at process launch and inject them only into the process environment.
- Example configuration must contain dummy values or environment-variable references only.
- Public documentation must have English and Japanese counterparts where applicable.

## Implementation baseline

- Windows-first.
- Rust 2024 edition.
- Pin `mcp-proxy` to the reviewed version before implementation; initial reviewed target is v0.4.3.
- Build with MCP `2026-07-28` support explicitly enabled.
- Use the smallest feature set that satisfies the requirements.
- A thin host may construct `mcp_proxy::Proxy`, remove the `proxy` MCP admin backend, optionally enable config hot reload, and serve. Do not fork upstream unless a proven blocker cannot be solved through its public library API.

## Required verification before completion

- Config validation.
- Unit tests for local host behavior.
- E2E tests with mock MCP backends.
- Mixed healthy/unhealthy backend tests.
- Namespace collision tests.
- Protocol negotiation tests.
- Proof that mutating tools are not automatically retried.
- Proof that `proxy/config` and `proxy/add_backend` are absent from `tools/list`.
- Admin API authentication negative/positive tests.
- Secure MCP Tunnel + ChatGPT integration smoke test.
- Git/public-release hygiene scan across files and history before any public push.
