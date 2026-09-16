# Configuration

Updated: 2026-09-13

## 1. Configuration layers

| Layer | Example | Git |
|---|---|---|
| Public gateway policy | namespace, listen port, timeout, disabled middleware | tracked |
| Machine-local endpoint inventory | real backend URLs/ports | local-only |
| Secure Tunnel credentials | `OPENAI_ADMIN_KEY`, `CONTROL_PLANE_API_KEY` | external SOPS only |
| Runtime state | logs, PID, Tunnel profile/health files | ignored / outside repo |

`config/proxy.example.toml` is the tracked example in the legacy deployment format (environment references only). Redistributable public-schema examples live under `test-fixtures/configs/`, and a minimal two-backend walkthrough is in [QUICKSTART.md](QUICKSTART.md). `config/proxy.local.toml` is machine-local and ignored.

## 2. Configuration formats and path precedence

The gateway accepts two on-disk formats and detects them automatically:

- **Public schema** — top-level `schema_version = 1`. Parsed strictly; unknown keys are rejected.
- **Legacy deployment schema** — top-level `[proxy]` table (the `mcp-proxy` configuration). Loaded against the original deployment policy (which keeps requiring at least one backend) and migrated **in memory**; the file itself is never modified. Every command reports which format it loaded.

Configuration path precedence (highest first) when `--config` is omitted:

1. explicit `--config <path>`;
2. the `LOMWAY_CONFIG` environment variable;
3. the first existing default candidate relative to the working directory: `config/proxy.local.toml`, then `gateway.toml`.

`${ENV_VAR}` references in backend URLs are resolved at load time; an unresolved variable fails the load loudly.

## 3. Public schema reference (`schema_version = 1`)

```toml
schema_version = 1

[server]
host = "127.0.0.1"                  # only 127.0.0.1 is allowed
port = 17777
instructions = "..."                # optional MCP instructions text
shutdown_timeout_seconds = 30

[policy]
allow_non_loopback_listener = false # must remain false
allow_non_loopback_backends = false # default false; true admits exact Tailscale HTTPS /mcp URLs only
hot_reload = false                  # must remain false
max_argument_size_bytes = 1048576   # <= 1 MiB

[observability]
audit = true
log_level = "info"
json_logs = false

[[backends]]
id = "filesystem"                   # unique, lowercase, no ambiguous forms
prefix = "fs_"                      # the namespace contract: <prefix><tool>
url = "http://127.0.0.1:8001/mcp"   # loopback by default; ${VAR} supported
required = true                     # default true; false = degrade-on-outage
timeout_seconds = 30                # default 30
```

Defaults: `[server]` listens on `127.0.0.1:17777`; `[policy]`, `[observability]`, and `backends` may be omitted entirely (the public schema accepts zero backends, while the deployment gate keeps requiring at least one). Unknown keys fail validation at every level.

Namespace prefix rules: non-empty lowercase stem + `_`, unique across backends, never normalized; reserved prefixes (`proxy_`, `lomway_`, legacy `lmg_`) are rejected.

Southbound remote backends are opt-in. With `allow_non_loopback_backends = true`, the only additional accepted URL form is exact `https://<machine>.<tailnet>.ts.net/mcp` on the default TLS port. Userinfo, query strings, fragments, custom ports, arbitrary Internet hosts, and non-TLS remote URLs remain rejected. This profile is intended for Tailscale Serve endpoints owned by trusted tailnet peers.

## 4. CLI reference

The binary implements every core mode itself (exit codes: `0` success, `1` failure, `2` usage error):

| Command | Purpose |
|---|---|
| `lomway serve --config <path>` | validate startup semantics and serve MCP on the loopback listener |
| `lomway check --config <path> [--probe]` | non-blocking validation (parse, policy, config-level collision preflight, no network I/O); `--probe` contacts each backend exactly once and verifies final tool names |
| `lomway list-backends --config <path>` | print the configured backends without contacting them |
| `lomway migrate --config <path> --output <path>` | convert a legacy configuration to the public schema as a new file (never overwrites; see [MIGRATION.md](MIGRATION.md)) |
| `lomway version` | print version, MCP protocol, and config schema information |

## 5. Legacy deployment reference

The public example references:

```text
WORKBRIDGE_MCP_URL
MEMORY_GATEWAY_MCP_URL
MICROSOFT_UFO_MCP_URL
STEALTH_BROWSER_MCP_URL
XMIND_WORKBOARD_MCP_URL
PRAXIOM_MCP_URL
```

## 5. Legacy deployment reference

The sections below document the original six-backend regression baseline in the legacy `[proxy]` format. It is a compatibility reference, not a public backend-count limit. All of it applies equally to the public schema after migration (see [MIGRATION.md](MIGRATION.md)).

The public example references:

```text
WORKBRIDGE_MCP_URL
MEMORY_GATEWAY_MCP_URL
MICROSOFT_UFO_MCP_URL
STEALTH_BROWSER_MCP_URL
XMIND_WORKBOARD_MCP_URL
PRAXIOM_MCP_URL
```

The default production policy requires every resolved backend URL to be `http://127.0.0.1:<port>/mcp`. A public-schema deployment may explicitly opt into the narrow Tailscale HTTPS backend profile described above; legacy deployment files remain loopback-only.

The gateway itself has no application/admin secret because no admin surface is served. SOPS is used by the Secure Tunnel scripts only; they read `OPENAI_ADMIN_KEY` for one-time tunnel creation and `CONTROL_PLANE_API_KEY` for runtime connection.

## 6. Listener

```toml
[proxy.listen]
host = "127.0.0.1"
port = 17777
```

External local endpoints:

```text
http://127.0.0.1:17777/mcp
http://127.0.0.1:17777/healthz
http://127.0.0.1:17777/readyz
```

There is no public/admin `/admin/*` API in v1.

## 7. Stable backend names

```text
workbridge
memory
ufo
browser
xmind
praxiom
```

These names become external prefixes and therefore form part of the public MCP contract.

## 8. Timeout defaults

| Backend | Timeout |
|---|---:|
| Workbridge | 300 s |
| Memory Gateway | 90 s |
| Microsoft UFO | 180 s |
| Stealth Browser | 180 s |
| XMind Workboard | 90 s |
| Praxiom | 180 s |

These are single-call bounds, not retry windows.

## 9. Enforced v1 policy

Startup rejects configuration that enables or changes any of the following:

- non-loopback listener;
- non-HTTP loopback backends and any remote backend outside the explicit Tailscale HTTPS profile;
- namespace separator other than `_`;
- `hot_reload = true`;
- search/discovery exposure;
- northbound auth inside the gateway;
- retry, hedging, tool-call cache;
- rate limit/concurrency/circuit-breaker/outlier middleware;
- mirror/canary/failover/composite tools/request coalescing;
- backend aliases, injected/default args, parameter overrides, expose/hide filtering, read-only/destructive rewriting;
- argument-size limit above 1 MiB.

This keeps v1 as a thin timeout-only routing boundary.

## 10. FastMCP compatibility

Microsoft UFO and Stealth Browser expose standard MCP metadata on their HTTP endpoints by setting:

```text
FASTMCP_INCLUDE_FASTMCP_META=false
```

This is a backend launch setting, not a gateway schema transformer.

## 11. Commands

Core modes run from the binary alone (see section 4 for the full CLI reference):

```powershell
target\release\lomway.exe check --config config\proxy.local.toml
target\release\lomway.exe check --config config\proxy.local.toml --probe
target\release\lomway.exe list-backends --config config\proxy.local.toml
target\release\lomway.exe version
```

PowerShell wrappers (optional convenience; see `integrations/powershell/`):

Validate policy/config without connecting backends:

```powershell
pwsh -NoProfile -File scripts/check.ps1
```

Build/install:

```powershell
pwsh -NoProfile -File scripts/install.ps1
```

Runtime:

```powershell
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/stop.ps1
```

Secure Tunnel (optional integration — the core gateway does not require it):

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action stop
```

Tunnel profiles are generated outside the repository under the user's application-data directory.
