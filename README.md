# Lomway

An MCP aggregation boundary that exposes **0..N independently managed local MCP servers** through one loopback MCP endpoint. Remote access can optionally be consolidated behind **one OpenAI Secure MCP Tunnel**.

> Status: implemented and validated on 2026-09-13. The final release verdict is recorded in [docs/FINAL_REVIEW.md](docs/FINAL_REVIEW.md).

Japanese: [README.ja.md](README.ja.md) · New to the project? Start with the [Quickstart](docs/QUICKSTART.md).

## Goal

```text
ChatGPT
|- Google Drive                         # independent connector, not aggregated
`- Local MCP
   `- OpenAI Secure MCP Tunnel
      `- 127.0.0.1:17777/mcp
         `- Lomway
            |- alpha_*                 -> arbitrary local MCP backend A
            |- beta_*                  -> arbitrary local MCP backend B
            `- ...                     -> additional configured backends
```

Google Drive deliberately stays outside the gateway so its native connector behavior is preserved.

## v1 implementation decisions

- Use `joshrotenberg/mcp-proxy` **0.4.3** as a library instead of reimplementing MCP proxying.
- Build with `default-features = false` and the explicit `protocol-2026-07-28` feature. `Cargo.lock` fixes the full resolved dependency set, including `tower-mcp` 0.18.2.
- Remove the upstream `proxy` MCP backend before serving. Client-visible `proxy_*` management tools are therefore absent.
- Serve only `/mcp`, `/healthz`, and `/readyz` from the project-owned northbound router. Upstream `/admin/*` is not exposed at all; `/admin/*` and `/mcp/admin/*` return 404.
- Bind exactly to `127.0.0.1:17777` in v1.
- Namespace tools as `<backend>_<tool>` with `_` as the stable separator.
- Keep native backend schemas. The gateway does not rewrite backend tools or collapse them into a generic dispatcher.
- Disable hot reload in v1 because project-specific safety policy is validated only at startup.
- Disable automatic retry, hedging, fan-out/failover and tool-call caching. A timed-out mutation must never be transparently replayed.
- Keep backend process lifecycle outside the gateway. Swibo owns start/stop/restart; the gateway only connects to loopback HTTP MCP endpoints.
- Implement every core mode in the binary itself: `serve`, `check` (`--probe`), `list-backends`, `migrate`, and `version`, with actionable exit codes and no PowerShell requirement.
- The gateway itself needs no secret. Secure Tunnel credentials are read process-locally from the external canonical SOPS store and are never written into this repository.

## Validated deployment regression

The public product does not pin a backend inventory or tool count. The original six-backend workstation regression baseline contains **183 tools**; the current reviewed workstation also includes an optional Chrome DevTools backend with 30 tools, for **213 tools total**:

| Namespace | Tools |
|---|---:|
| `workbridge_` | 12 |
| `memory_` | 32 |
| `ufo_` | 19 |
| `browser_` | 97 |
| `xmind_` | 21 |
| `praxiom_` | 2 |
| `chrome_` | 30 |
| **Current total** | **213** |

Validated facts:

- all six baseline backends initialize successfully through the same `tower-mcp` client stack;
- the current seven configured namespaces, including optional `chrome_`, all succeed through the aggregate gateway;
- `proxy_*` count is 0;
- same-named tools are namespace-isolated;
- one failed backend can be skipped when another backend is healthy;
- if every configured backend fails during initial construction, upstream cannot build the proxy and startup fails closed;
- a forced timeout of a mutating mock tool produces exactly one backend invocation;
- one Lomway OpenAI Secure MCP Tunnel is configured and reaches READY while probing this MCP endpoint;
- Swibo lifecycle validation passes `READY -> restart -> READY -> stop -> STOPPED -> start -> READY`.

Microsoft UFO and Stealth Browser use FastMCP's supported `FASTMCP_INCLUDE_FASTMCP_META=false` setting on their HTTP aggregation endpoints so their tool metadata remains interoperable with the pinned `tower-mcp` client. The gateway does not rewrite their schemas.

## Repository layout

```text
lomway/
|- Cargo.toml
|- Cargo.lock
|- LICENSE
|- src/
|- config/
|  `- proxy.example.toml
|- scripts/
|  |- install.ps1
|  |- check.ps1
|  |- start.ps1
|  |- status.ps1
|  |- stop.ps1
|  |- common.ps1
|  `- integration-smoke.ps1
|- integrations/
|  |- openai-secure-tunnel/
|  |- swibo/
|  |- sops/
|  `- powershell/
|- test-fixtures/
|- tests/
|- docs/
|- AGENTS.md
|- .gitignore
|- README.md
`- README.ja.md
```

`config/proxy.local.toml`, runtime state, logs, PID files and generated Secure Tunnel profiles are machine-local and ignored or stored outside the repository.

## Local operation

Core modes run from the binary alone — no PowerShell required:

```powershell
cargo build --locked --release
target\release\lomway.exe check --config <path-to-config>
target\release\lomway.exe serve --config <path-to-config>
```

Optional PowerShell wrappers remain available:

```powershell
pwsh -NoProfile -File scripts/install.ps1
pwsh -NoProfile -File scripts/check.ps1
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/integration-smoke.ps1
```

The Secure MCP Tunnel workflow is an optional integration and lives in its own directory:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
```

Swibo is the normal lifecycle entry point after registration. Its live target is `lomway`; workstation-specific target data is intentionally not committed to the public example registry.

## Documentation

- [Quickstart](docs/QUICKSTART.md) — two arbitrary backends, no integrations
- [Requirements](docs/REQUIREMENTS.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Security](docs/SECURITY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Configuration](docs/CONFIGURATION.md)
- [Testing](docs/TESTING.md)
- [Operations](docs/OPERATIONS.md)
- [Optional integrations](docs/INTEGRATIONS.md)
- [Migration & rollback](docs/MIGRATION.md)
- [Implementation plan / completion record](docs/IMPLEMENTATION_PLAN.md)
- [Public generalization design](docs/PUBLIC_GENERALIZATION_DESIGN.md)
- [Public generalization task plan](docs/PUBLIC_GENERALIZATION_TASKS.md)
- [Public generalization task manifest](docs/PUBLIC_GENERALIZATION_TASKS.yaml)
- [Release checklist](docs/RELEASE.md)
- [Changelog](docs/CHANGELOG.md)
- [Final review](docs/FINAL_REVIEW.md)
- [ADRs](docs/adr/)

## Primary references

- MCP 2026-07-28: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- OpenAI custom MCP / Secure MCP Tunnel: https://help.openai.com/en/articles/12584461
- mcp-proxy: https://github.com/joshrotenberg/mcp-proxy
- mcp-proxy v0.4.3: https://github.com/joshrotenberg/mcp-proxy/releases/tag/v0.4.3
