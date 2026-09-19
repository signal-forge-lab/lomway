# Jev Ultrafast MCP backend

This optional integration registers the independently managed Jev Ultrafast
MCP server with Lomway. Lomway does not start, stop, retry, cache, or alter the
backend's browser operations or results.

Japanese: [README.ja.md](README.ja.md)

## Backend

From the `signal-forge-lab/jev-ultrafast` checkout on branch
`feature/lomway-mcp-recovery`:

```powershell
uv sync
$env:JEV_MCP_HOST = "127.0.0.1"
$env:JEV_MCP_PORT = "18766"
uv run jev-mcp
```

Use the existing external supervisor for long-running operation. Do not add
process lifecycle management to Lomway.

The backend publishes exactly these tools:

- `jev_browser_start`
- `jev_browser_step`
- `jev_browser_run`
- `jev_browser_resume_text`
- `jev_browser_inspect`
- `jev_browser_close`

The tracked example is [`config/jev-ultrafast.example.toml`](../../config/jev-ultrafast.example.toml).
Its `jev_` Lomway namespace means an aggregated tool such as
`jev_browser_start` is exposed as `jev_jev_browser_start`; Lomway preserves the
backend schema and result without rewriting them.

## Model configuration

`TYPESAFE_API_KEY` remains the normal Jev decision credential. Internal text
mode uses `TEXT_MODEL_*`. Recovery uses provider-neutral `RECOVERY_MODEL_*`
settings and falls back to the corresponding `TEXT_MODEL_*` settings. Inject
real values through the existing external SOPS/supervisor path; never write
them into this repository or the Jev repository.

## Verification

With the backend running:

```powershell
cargo run -- check --config config/jev-ultrafast.example.toml --probe
cargo run -- serve --config config/jev-ultrafast.example.toml
```

An MCP client connected to `http://127.0.0.1:17777/mcp` should list the six
namespaced tools. Stopping Jev must not make Lomway retry mutating calls; the
backend is optional so the existing mixed healthy/unhealthy startup policy is
unchanged.
