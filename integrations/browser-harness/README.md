# Browser Harness MCP over Lomway (Windows)
The Browser Harness CLI is installed as a uv tool with Python 3.12 and the `[mcp]` extra.
Its official `browser-harness-mcp` stdio server is bridged to loopback-only Streamable HTTP by `mcp-proxy`.

## Startup
Run `powershell -NoProfile -File integrations/browser-harness/start.ps1`.
Stop with `powershell -NoProfile -File integrations/browser-harness/stop.ps1`.
The external Swibo supervisor owns the lifecycle. Do not spawn this backend from Lomway itself.

The Lomway local config uses:
```toml
[[backends]]
id = "browser-harness"
prefix = "bh_"
url = "http://127.0.0.1:17685/mcp"
required = false
timeout_seconds = 180
exposure = "deferred"
```

MCP protocol smoke (23 expected tools in v0.1.13):
```powershell
& (Join-Path $env:APPDATA 'uv/tools/browser-harness/Scripts/python.exe') scripts/browser-harness-mcp-smoke.py
```
Browser access additionally requires running Chrome 144+ with permission enabled at `chrome://inspect/#remote-debugging` and approving the attach prompt. Local recordings remain disabled. Never expose port 17685 on LAN.
