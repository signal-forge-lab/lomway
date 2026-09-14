# PowerShell Wrapper Integration (optional)

Status: OPTIONAL INTEGRATION — CLI WRAPPERS ONLY

All PowerShell in this repository is optional wrapper tooling. The gateway
binary is self-sufficient: `lomway serve|check|list-backends|migrate|version`.
works with no PowerShell at all, which keeps the core PowerShell-independent
and portable to non-Windows CI later.

## Wrapper inventory

| Script | Wraps | Integration knowledge |
| --- | --- | --- |
| `scripts/install.ps1` | `cargo build --release --locked` (+ toolchain check) | none |
| `scripts/check.ps1` | `lomway check --config <path>` via `cargo run` | none |
| `scripts/start.ps1` | background `lomway serve --config <path>` + pid file + `GET /healthz` wait | none |
| `scripts/status.ps1` | pid lookup + `GET /healthz` | none |
| `scripts/stop.ps1` | pid lookup + `Stop-Process` | none |
| `scripts/common.ps1` | shared pid-file helpers | none |
| `scripts/integration-smoke.ps1` | MCP `initialize` / `tools/list` / `tools/call` against a running gateway (local six-backend regression wrapper) | none |

Integration-specific scripts no longer live in `scripts/`; they are packaged
under their own optional integration directories:

- `integrations/openai-secure-tunnel/` — Secure Tunnel wrappers;
- `integrations/sops/` — secret resolution wrapper;
- `integrations/swibo/` — supervisor template (documentation only).

## No duplicated core business logic

The wrappers never re-implement configuration validation, policy, namespace,
routing, or health semantics; they only invoke `cargo`, the gateway CLI, or
the gateway's public HTTP endpoints and report the result. Policy decisions
have exactly one source of truth: the Rust core. The lifecycle wrappers add
only process plumbing (pid file, window hiding, readiness polling) that the
CLI deliberately does not own.

## Guarantees

- Wrappers start or stop the gateway process; they never retry mutating tool
  calls and never add retry logic to the gateway.
- Wrapper defaults reference repository-relative paths and environment
  variables only; machine-local values stay in gitignored local config.
