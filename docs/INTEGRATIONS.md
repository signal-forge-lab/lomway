# Optional Integrations

Updated: 2026-09-13

The Lomway core is a thin loopback aggregation boundary. It builds, tests, validates, and serves with **no OpenAI credential, no Swibo, no SOPS store, and no PowerShell**. Every local-specific workflow is packaged as a separate optional integration under `integrations/`, each with English and Japanese documentation. Nothing in `src/` or `Cargo.toml` imports or references any of them.

Japanese: [INTEGRATIONS.ja.md](INTEGRATIONS.ja.md)

## The guides

| Integration | Path | Purpose |
|---|---|---|
| OpenAI Secure MCP Tunnel | [`integrations/openai-secure-tunnel/`](../integrations/openai-secure-tunnel/README.md) ([ja](../integrations/openai-secure-tunnel/README.ja.md)) | Remote ingress: expose the loopback gateway to ChatGPT through the Secure MCP Tunnel. Reads tunnel credentials process-locally; single-attempt client operations, never retried. |
| Swibo | [`integrations/swibo/`](../integrations/swibo/README.md) ([ja](../integrations/swibo/README.ja.md)) | External process supervision. Owns start/stop/restart of the gateway, tunnel, and backend targets; the template contains placeholders only. |
| SOPS | [`integrations/sops/`](../integrations/sops/README.md) ([ja](../integrations/sops/README.ja.md)) | Resolves named secrets from the canonical SOPS store into process environment variables at launch. Secret values never enter the repository or the gateway's files. |
| PowerShell wrappers | [`integrations/powershell/`](../integrations/powershell/README.md) ([ja](../integrations/powershell/README.ja.md)) | Thin `scripts/` conveniences around the core CLI and public HTTP endpoints. No duplicated policy, routing, or namespace logic. |

## Planned backend integration design

- [Jev Ultrafast MCP integration and recovery plan](JEV_ULTRAFAST_MCP_PLAN.md) — design-only milestone plan for an independently managed Jev browser backend, dual text modes, bounded Recovery LLM escalation, and exact Browser Harness target handoff. The backend logic remains outside Lomway core.

## Why the core stays independent

- The core reads plain process environment variables and configuration files only; secret resolution is a launch-time concern of the SOPS integration.
- The gateway only dials `http://127.0.0.1:<port>/mcp` endpoints; it never spawns or supervises backend processes, so lifecycle supervision is Swibo's job.
- Remote ingress is a separate deployment concern; the same gateway serves loopback-only without any tunnel.
- The binary implements every core mode (`serve`, `check`, `list-backends`, `migrate`, `version`) itself; the PowerShell wrappers only orchestrate.

This separation is enforced, not aspirational: repository hygiene tests scan `src/` and `Cargo.toml` for integration-specific identifiers, and the CI test suite never requires a private deployment.

## When you need which

- Running locally and attaching any MCP client → **core only** (see [QUICKSTART.md](QUICKSTART.md)).
- Exposing the gateway to ChatGPT over the internet → add the **OpenAI Secure MCP Tunnel** integration.
- Supervising gateway/backend/tunnel processes declaratively → add the **Swibo** integration.
- Keeping tunnel credentials in an encrypted store instead of the shell environment → add the **SOPS** integration.
- Preferring one-command install/check/start/status/stop → add the **PowerShell wrapper** integration.

Integrations compose: the tunnel wrappers automatically consult the SOPS helper only when required keys are absent from the environment, and Swibo can supervise the whole stack.

## Migration and rollback

For moving an existing legacy `[proxy]` deployment to the public schema (and rolling back), see [MIGRATION.md](MIGRATION.md).
