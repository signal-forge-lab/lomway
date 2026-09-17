# Security

Updated: 2026-09-13

## 1. Trust boundaries

```text
ChatGPT
  -> OpenAI Secure MCP Tunnel
  -> Lomway
  -> six explicitly configured loopback MCP backends
```

Google Drive remains outside this boundary as its own connector.

## 2. Network boundary

- gateway listener is exactly `127.0.0.1`;
- v1 rejects LAN, `0.0.0.0`, Tailscale or public binds;
- every southbound backend URL must also be loopback HTTP `/mcp`;
- the Secure Tunnel is the only remote ingress path.

## 3. Upstream control plane removal

`mcp-proxy` 0.4.3 creates a `proxy` MCP backend containing management tools such as configuration inspection and dynamic backend registration. The host calls `remove_backend("proxy")` and fails startup unless removal succeeds.

The upstream HTTP router also includes `/admin/*`. The project-owned router does not expose it: top-level `/admin/*` has no route, and middleware rejects nested `/mcp/admin/*` before upstream dispatch. Negative E2E proves these paths return 404 even when an arbitrary authorization header is supplied.

Consequences:

- no gateway admin token is required;
- ChatGPT cannot add/remove/reconfigure backends through this gateway;
- local health uses `/healthz`, not an admin API.

## 4. Secrets

The gateway runtime needs no secret. Secure Tunnel setup/runtime needs:

- `OPENAI_ADMIN_KEY` for one-time remote tunnel create/reuse;
- `CONTROL_PLANE_API_KEY` for the managed local tunnel runtime.

Both are resolved from `%USERPROFILE%\.config\sops\secrets\global.sops.json` by PowerShell into process environment only. Scripts restore/clear process values after use and do not write plaintext values to the repository or command line.

## 5. Tool-surface safety

- native typed tools remain visible; no generic dispatcher is added;
- backend schemas are not rewritten by the gateway;
- no dynamic backend registration tool exists;
- no retry/hedge/fan-out/failover/cache can duplicate a mutating call;
- maximum tool argument size is bounded to 1 MiB by production policy;
- backend aliases/filtering/default-argument injection are rejected in v1.

## 6. FastMCP interoperability

UFO and Stealth Browser disabled framework-private `_fastmcp` metadata through FastMCP's supported `FASTMCP_INCLUDE_FASTMCP_META=false` setting. This is safer than teaching the gateway to sanitize arbitrary third-party schemas.

## 7. Supply chain

- `mcp-proxy` exact-pinned to 0.4.3;
- upstream protocol feature explicitly selected;
- `Cargo.lock` tracked;
- dependency updates require source/config/security review and the complete regression gate;
- no `latest` production install path.

## 8. Public repository gate

Before public push, scan tracked source/docs/history candidates for:

- API keys, bearer tokens, private keys and secret-like assignments;
- email addresses or other personal identifiers;
- Windows absolute paths containing a real username;
- real machine-local endpoint inventory;
- tunnel IDs and generated tunnel profile/log paths;
- `.env`, SOPS plaintext, local TOML/JSON, logs, PID/lock files and build artifacts.

Branches are never treated as a secrecy boundary.
