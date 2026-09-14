# OpenAI Secure MCP Tunnel Integration (optional)

Status: OPTIONAL INTEGRATION

This directory packages the OpenAI Secure MCP Tunnel workflow that exposes the
local gateway to ChatGPT through `tunnel-client`. The core gateway does **not**
require this integration: `src/` contains no OpenAI references and `Cargo.toml`
carries no OpenAI dependency, so `cargo build` / `cargo test` and a plain
Running `lomway serve --config <path>` needs no OpenAI credentials.

## Files

| File | Purpose |
| --- | --- |
| `configure-tunnel.ps1` | One-time creation/reuse of the Secure Tunnel alias, then connects it to the gateway MCP endpoint. |
| `tunnel.ps1` | `ensure` (reconnect if needed), `status`, `stop` for an already-configured alias. |

Both scripts are self-contained wrappers: they locate `tunnel-client`, resolve
API keys into the process environment, invoke the client once, and report
its JSON output. Neither script re-implements any gateway behavior.

## Prerequisites

- `tunnel-client` on `PATH`, or the `LOCAL_MCP_TUNNEL_CLIENT` process
  environment variable pointing at the executable.
- API keys in the process environment, or the SOPS integration
  (`integrations/sops/`) to resolve them from the canonical store:
  - `tunnel.ps1` needs `CONTROL_PLANE_API_KEY`.
  - `configure-tunnel.ps1` needs `OPENAI_ADMIN_KEY` and `CONTROL_PLANE_API_KEY`.

## Usage

One-time configuration (gateway must already be running and healthy):

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
```

Day-to-day operation:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action stop
```

Tunnel profiles are generated outside the repository under the user's
application-data directory; nothing tunnel-related is committed.

## Guarantees

- Each command attempts the client operation exactly once. If it fails, the
  operator re-runs it manually; no automatic retry of mutating operations.
- API keys live only in the process environment for the duration of the
  client call and are restored afterwards (`configure-tunnel.ps1`).
