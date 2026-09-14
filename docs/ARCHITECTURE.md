# Architecture

Updated: 2026-09-14

## 1. Responsibility boundary

`Lomway` is a connection aggregation layer. It is not a process supervisor, workflow orchestrator, semantic router, or public multi-tenant gateway.

```text
ChatGPT
  | OpenAI Secure MCP Tunnel
  v
127.0.0.1:17777/mcp
  |
  v
Lomway
  |- namespace + route
  |- timeout only
  |- startup policy validation
  |- /healthz + /readyz + structured logs
  `- no retry / hedge / cache / semantic rewrite
       |
       +-> arbitrary local MCP backend A
       +-> arbitrary local MCP backend B
       `-> ...

Process lifecycle: Swibo
Remote ingress: OpenAI Secure MCP Tunnel
Google Drive: independent connector
```

All configured southbound connections are loopback HTTP MCP endpoints. The gateway never spawns a backend process.

## 2. Upstream library boundary

v1 exact-pins `mcp-proxy = 0.4.3` with `default-features = false` and `protocol-2026-07-28`. The committed lockfile fixes the complete dependency resolution; the validated lock currently resolves `tower-mcp` 0.18.2.

The thin Rust host adds only the project-specific safety boundary that upstream cannot express directly. Both configuration entry points (public `schema_version = 1` and legacy `[proxy]`) converge on the same fail-closed startup pipeline:

```text
load config (format auto-detected; legacy migrated in memory)
  -> resolve environment references
  -> validate local project policy
  -> backend registry (validated descriptors only, configuration order)
  -> startup probe (required must be up; optional may degrade; all-down fails)
  -> tool collision preflight (final names computed before serving)
  -> startup acceptance (required backends healthy; optional backends may degrade; at least one backend healthy)
  -> Proxy::from_config(...)
  -> remove_backend("proxy") and require success
  -> wrap upstream router with /admin rejection
  -> expose /mcp, /healthz, /readyz on loopback
```

Hot reload is deliberately disabled. Otherwise upstream reload could accept a configuration that has passed upstream validation but not this project's stricter policy validation.

## 3. External HTTP surface

The project-owned listener exposes only:

- `POST/GET/DELETE /mcp` as required by Streamable HTTP MCP;
- `GET /healthz` for liveness;
- `GET /readyz` for readiness (required backends must be up; optional degradation is reported; the status is recomputed live from backend health).

It does **not** expose upstream `/admin/*`. Both top-level `/admin/*` and nested `/mcp/admin/*` are rejected with 404, as is every other unrouted path. Therefore no gateway admin secret is required.

## 4. Namespace contract

The stable separator is `_`. Final tool names are `<prefix><upstream tool name>`; they are precomputed at startup and a collision fails fast while naming both sources. Prefix rules: non-empty lowercase stem followed by `_`, unique across backends, never silently normalized — ambiguous inputs are rejected. Reserved prefixes (`proxy_`, `lomway_`, and the legacy `lmg_`) are policy-controlled and cannot be claimed by a backend.

The original six-backend regression baseline (not a public product limit):

| Backend | Prefix | Validated tool count |
|---|---|---:|
| Workbridge | `workbridge_` | 12 |
| Memory Gateway | `memory_` | 32 |
| Microsoft UFO | `ufo_` | 19 |
| Stealth Browser | `browser_` | 97 |
| XMind Workboard | `xmind_` | 21 |
| Praxiom | `praxiom_` | 2 |

The original baseline aggregate count is 183. At the 2026-09-14 re-review, the workstation also had an optional 30-tool Chrome DevTools namespace, producing 213 tools total. Neither count is a public contract: Lomway supports the validated public 0..N configuration model. Prefix changes are breaking changes. Backend tool descriptions and input schemas pass through verbatim.

## 5. Module map

```text
src/
|- config/      public schema model, strict validation, format detection,
|               `${VAR}` resolution, path precedence, legacy <-> public migration
|- backend/     validated backend descriptors, deterministic registry,
|               startup probe semantics
|- namespace/   prefix policy, reserved prefixes, collision preflight
|- gateway/     policy validation (decomposed validators), gateway builder
|               (startup pipeline), router surface
|- health/      liveness vs readiness state model
|- cli/         argument definitions and command dispatch (serve/check/
|               list-backends/migrate/version)
`- lib.rs       proxy composition, control-plane removal, backend restart
                recovery monitor
```

`lib.rs` also runs the backend restart recovery monitor: a loopback port probe plus stability hysteresis decides when an offline or unhealthy backend's transport is replaced, so a recovered backend is re-adopted without a gateway restart.

## 6. Request flow

### `tools/list`

1. The client initializes through `/mcp`.
2. `mcp-proxy` returns the cached capabilities discovered from successfully initialized backends.
3. Backend namespaces are applied.
4. The removed `proxy` control-plane backend contributes no tools.
5. The aggregate catalog is returned unchanged apart from namespace qualification.

### `tools/call`

1. Resolve the backend from the prefix.
2. Strip the namespace and call the original backend tool.
3. Apply only that backend's configured timeout.
4. Never retry, hedge, fail over or cache the tool call.
5. Return the backend result/error without semantic rewriting.

## 7. Protocol behavior

The binary is compiled with the upstream `protocol-2026-07-28` feature. Protocol negotiation remains transport/client driven. The validated OpenAI Secure MCP Tunnel session negotiated MCP `2025-11-25` with the gateway while the six-backend regression baseline initialized successfully through the pinned client stack; additional configured backends are not part of that minimum regression contract. Project code does not custom-deserialize/rebuild MCP JSON-RPC bodies.

## 8. Backend compatibility

Microsoft UFO and Stealth Browser are FastMCP servers. Their default FastMCP metadata included a `_fastmcp` metadata key that the pinned `tower-mcp` client rejected. The backend HTTP launch paths now use FastMCP's supported `FASTMCP_INCLUDE_FASTMCP_META=false` setting. This removes framework-private metadata at the source instead of adding gateway-side schema rewriting.

XMind Workboard intentionally fails closed when the official upstream capability fingerprint changes. During integration the upstream fingerprint changed only because `xmind_get_topic` description gained task read-back semantics; tool names/counts and input schemas were unchanged. The cache was refreshed after diff review, and Workboard task verification was strengthened to semantic read-back.

## 9. Failure model

- **Backend fails during a routed call:** that call fails; unrelated namespaces remain available.
- **One required backend is unreachable at startup:** startup fails closed.
- **One optional backend is unreachable at startup:** startup degrades and the gateway serves the healthy set.
- **Every backend fails at startup:** the startup probe fails, so startup fails closed.
- **A backend goes down after startup, then recovers:** the restart recovery monitor re-adopts it automatically (port probe + stability hysteresis, then transport replacement). No gateway restart is required; this is covered by E2E for brief restarts and sustained outages.
- **Gateway fails:** backend processes stay alive; Swibo restarts only the gateway/tunnel target.
- **Tunnel fails:** gateway/backends stay local and healthy; restart only the tunnel.

## 10. Lifecycle

Normal order:

```text
1. backend MCP services
2. Lomway
3. Lomway Secure MCP Tunnel
```

Swibo owns steps 2 and 3 as one declarative target and keeps backend targets independent.

## 11. Observability

- `/healthz`: liveness of the gateway process.
- `/readyz`: readiness, recomputed live from backend health (2 s bound); required-backend failure never reports ready.
- structured gateway logs: backend initialization, routing/audit and errors without secret values.
- Swibo: independent server/tunnel state for the gateway and independent health for each backend.
- no public/admin metrics API in v1.

## 12. Upgrade boundary

Every `mcp-proxy` or `tower-mcp` resolution change requires config/schema review, control-plane suppression regression, fault tests, six-real-backend smoke and Secure Tunnel smoke before the lockfile is updated.
