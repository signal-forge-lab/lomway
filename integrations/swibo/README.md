# Swibo Integration (optional)

Status: OPTIONAL INTEGRATION — EXTERNAL SUPERVISOR TEMPLATE

Swibo is the external process supervisor used by the reference deployment to
own the lifecycle of the local MCP backend services and of the gateway target.
The core gateway never starts, stops, or supervises backend processes; Swibo
(or any equivalent supervisor, or plain manual startup) owns that
responsibility outside this repository.

## Core independence

- `src/` imports no Swibo code and `Cargo.toml` declares no Swibo dependency;
  the gateway compiles, tests, and serves with no Swibo installation.
- `scripts/` wrappers call the gateway CLI and HTTP endpoints only; they know
  nothing about Swibo targets.
- Swibo interacts with the gateway purely as an external process manager: it
  launches `lomway serve --config <path>` and observes
  `GET /healthz`.

## Reference lifecycle ordering

```text
Start: backend services -> gateway -> Secure MCP Tunnel (optional integration)
Stop:  Secure MCP Tunnel -> gateway
```

Stopping the aggregate gateway target never stops any backend.

## Template

Adapt the following placeholder parameters to a local Swibo installation.
This template intentionally contains no machine-specific absolute paths; all
values are relative to the checkout or read from the environment.

| Parameter | Placeholder value | Meaning |
| --- | --- | --- |
| Target name | `<gateway-target>` | Swibo target wrapping the gateway. |
| Command | `lomway serve --config <config-path>` | Current gateway launch command. |
| Working directory | `<repo-root>` | Checkout root of this repository. |
| Health probe | `http://127.0.0.1:<port>/healthz` | Liveness endpoint (default port `17777`). |
| Ordered components | `<gateway-target>`, `<tunnel-target>` | Start/stop ordering for the aggregate target. |

Replace each `<placeholder>` with local values in the local supervisor
configuration only; do not commit machine-specific values.
