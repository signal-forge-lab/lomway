# Quickstart

Updated: 2026-09-13

Aggregate any two local MCP servers behind **one loopback MCP endpoint** using nothing but the `lomway` binary and one TOML file. This walkthrough deliberately uses **no OpenAI Secure MCP Tunnel, no Swibo, no SOPS, and no PowerShell** — the core gateway does not need any of them.

Japanese: [QUICKSTART.ja.md](QUICKSTART.ja.md)

## 1. Prerequisites

- A stable Rust toolchain (`rustup`). The validated baseline is Windows; the core builds and runs anywhere stable Rust does.
- Two local MCP servers that expose Streamable HTTP MCP endpoints on loopback, for example `http://127.0.0.1:18701/mcp` and `http://127.0.0.1:18702/mcp`. Any independently managed MCP server works — the gateway never spawns backend processes.

## 2. Build

```powershell
cargo build --locked --release
```

The binary is `target/release/lomway.exe` on Windows (`target/release/lomway` elsewhere). Examples below write `lomway` for brevity.

## 3. Minimal two-backend configuration

Save as `gateway.quickstart.toml` (any path works; see section 7 for how the binary finds a configuration when `--config` is omitted):

```toml
schema_version = 1

[[backends]]
id = "alpha"
prefix = "alpha_"
url = "http://127.0.0.1:18701/mcp"
required = true
timeout_seconds = 30

[[backends]]
id = "beta"
prefix = "beta_"
url = "http://127.0.0.1:18702/mcp"
required = false
timeout_seconds = 30
```

What this means:

- `schema_version = 1` is the only supported public schema version.
- Every backend URL must be `http://127.0.0.1:<port>/mcp` (loopback-only policy). `${ENV_VAR}` references are also accepted and resolved at load time.
- `alpha` is **required**: if it is unreachable, startup fails closed.
- `beta` is **optional**: if it is unreachable, startup degrades and the gateway serves the healthy set.
- `[server]`, `[policy]`, and `[observability]` are omitted here; conservative defaults apply (loopback listener on port `17777`, hot reload off, 1 MiB argument bound). See [CONFIGURATION.md](CONFIGURATION.md) for the full schema.

Redistributable variants of this file (zero/one/many backends) live under `test-fixtures/configs/` and are exercised by the test suite.

## 4. Validate without serving

```powershell
lomway check --config gateway.quickstart.toml
```

`check` is non-blocking: parse, policy validation, and config-level collision preflight with zero network I/O. Add `--probe` for the doctor-style diagnostic that contacts each backend exactly once (probes never retry) and verifies final tool names:

```powershell
lomway check --config gateway.quickstart.toml --probe
```

To print the configured backends without contacting them:

```powershell
lomway list-backends --config gateway.quickstart.toml
```

## 5. Serve

```powershell
lomway serve --config gateway.quickstart.toml
```

The gateway exposes exactly:

| Endpoint | Purpose |
|---|---|
| `http://127.0.0.1:17777/mcp` | the aggregate MCP endpoint (Streamable HTTP) |
| `http://127.0.0.1:17777/healthz` | liveness |
| `http://127.0.0.1:17777/readyz` | readiness (required backends must be up; optional degradation is reported) |

There is no `/admin/*` surface: upstream admin routes are removed and return `404`, so the gateway itself needs no secret.

## 6. Verify

With any MCP client connected to `/mcp`:

- `tools/list` returns `alpha_status` and `beta_status` — final names are `<prefix><upstream tool name>`.
- Backend tool descriptions and input schemas pass through verbatim; the gateway never rewrites them.
- The upstream `proxy` control-plane backend contributes no tools (`proxy_*` count is 0).
- A timed-out mutating call is dispatched exactly once and surfaces as an error; it is never transparently replayed.

Without an MCP client:

```powershell
curl.exe http://127.0.0.1:17777/healthz
curl.exe http://127.0.0.1:17777/readyz
```

The live two-backend end-to-end path (mock MCP backends over the same public schema) is proven by:

```powershell
cargo test --locked --test fixtures
```

## 7. Configuration path precedence

When `--config` is omitted, the binary resolves the configuration in this order (highest first):

1. explicit `--config <path>`;
2. the `LOMWAY_CONFIG` environment variable;
3. the first existing default candidate relative to the working directory: `config/proxy.local.toml`, then `gateway.toml`.

## 8. What this quickstart did not use

- **OpenAI Secure MCP Tunnel** — optional remote ingress ([INTEGRATIONS.md](INTEGRATIONS.md)).
- **Swibo** — optional external process supervisor; the gateway only dials loopback HTTP endpoints.
- **SOPS** — optional secret resolution; the core reads plain environment variables only.
- **PowerShell** — the `scripts/` wrappers are conveniences; the binary implements every core mode itself.

## Next steps

- [CONFIGURATION.md](CONFIGURATION.md) — full public schema and policy reference.
- [ARCHITECTURE.md](ARCHITECTURE.md) — responsibility boundary and request flow.
- [TESTING.md](TESTING.md) — test suites and how to run them.
- [INTEGRATIONS.md](INTEGRATIONS.md) — optional integrations, each packaged separately.
