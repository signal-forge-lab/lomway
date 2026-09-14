# Lomway — Public Generalization Design

Status: DESIGN BASELINE

This document is the canonical design for turning Lomway (formerly Local MCP Gateway) into a distributable, general-purpose MCP aggregation gateway without breaking the existing deployment. Historical task IDs retain the `LMG-G*` prefix for traceability only.

## Goals

Public Lomway must aggregate 0..N arbitrary MCP backends from configuration without depending on the current six backends, Swibo, SOPS, or OpenAI Secure MCP Tunnel.

The public Core provides arbitrary HTTP Streamable MCP backends, explicit namespace prefixes, fail-fast collision detection, one northbound MCP endpoint, standalone operation, optional integrations, and conservative defaults.

## Non-goals for public v1

- owning backend process lifecycle;
- requiring OpenAI Secure MCP Tunnel, Swibo, or SOPS;
- failover/mirroring/canary/fan-out;
- automatic retry of mutation tools;
- semantic schema rewriting;
- default internet-facing listeners;
- hot reload;
- collapsing tools into a generic `call_tool(name,args)` dispatcher.

## Architecture

```text
MCP Client
   |
   v
Lomway Host
   +-- Core Config
   +-- Backend Registry
   +-- Namespace / Collision Guard
   +-- MCP Aggregation Router
   +-- Health / Readiness
   |
   +--> Backend A
   +--> Backend B
   +--> Backend C ...

Optional integrations
   +-- OpenAI Secure MCP Tunnel
   +-- Swibo
   +-- SOPS
   +-- PowerShell helpers
```

Core must run with no integration installed.

## Target module boundaries

```text
src/
  lib.rs
  config/{mod.rs,model.rs,load.rs,validate.rs}
  backend/{mod.rs,registry.rs,descriptor.rs,probe.rs}
  namespace/{mod.rs,normalize.rs,collision.rs}
  gateway/{mod.rs,build.rs,router.rs,policy.rs}
  health/{mod.rs,state.rs}
  cli/{mod.rs,args.rs}
  main.rs

integrations/
  openai-secure-tunnel/
  swibo/
  sops/
  powershell/
```

Integrations are not compile-time requirements of Core.

## Configuration model

```toml
[server]
host = "127.0.0.1"
port = 17777

[policy]
allow_non_loopback_backends = false
allow_non_loopback_listener = false
hot_reload = false
max_argument_size_bytes = 1048576

[[backends]]
id = "filesystem"
prefix = "fs_"
url = "http://127.0.0.1:8001/mcp"
required = true

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:8002/mcp"
required = false
```

Backend IDs and prefixes are unique; empty prefixes are rejected in public v1; final tool names are globally unique; listener/backend URLs are loopback by default; required backend failure blocks startup; optional backend failure may degrade startup; backend transports recover after a backend restart without requiring a Lomway restart.

## Tool identity

Lomway preserves direct tools rather than replacing them with a generic dispatcher.

```text
upstream: navigate
prefix: browser_
public: browser_navigate
```

Descriptions and input schemas remain semantically intact. Reserved prefixes such as `proxy_`, `lomway_`, and legacy `lmg_` are policy-controlled.

## Failure and health model

Invalid configuration, policy violations, duplicate IDs/prefixes, final tool collisions, or unavailable required backends fail closed. Core performs no automatic retry. `/healthz` reports liveness; `/readyz` reports validated startup readiness. No HTTP admin plane is added in public v1.

## Security baseline

Loopback listener/backends by default, no auth forwarding, no secret logging, no admin endpoint, no hot reload, no retry/hedging/coalescing, and a 1 MiB default argument bound. Non-loopback operation requires a future explicit security profile and threat model.

## Integration boundaries

OpenAI Secure MCP Tunnel, Swibo, SOPS, and PowerShell are external/optional integrations. Core must not know their credentials, registries, or machine-local state.

## CLI contract

```text
lomway serve --config <path>
lomway check --config <path>
lomway list-backends --config <path>
lomway version
```

`check` performs parse, policy, and collision preflight without serving.

## Compatibility

Introduce `schema_version`, reject unknown top-level keys by default, preserve field meaning in minor releases, and retain a migration path for the current deployment.

## Testing and release

Unit tests cover configuration, validation, namespace behavior, collision detection, and required/optional semantics. Mock E2E covers 1/N/0 backends, optional/required failure, collisions, timeout behavior, exactly-once mutations, and absence of admin routes. Portable real-backend tests use public fixtures; the current six-backend suite remains a local regression suite.

Release gates include fmt, clippy with warnings denied, locked build/test, secret/privacy scan, license review, vulnerability scan, and a clean configuration/install smoke test.

## Distribution

Initial target: GitHub source release, Windows x86_64 binary, checksums, example configurations, and optional integration documentation. Linux/macOS CI follows after Core becomes PowerShell-independent.

## Existing deployment

The original six-backend deployment remains a reference regression baseline, not a public inventory limit. Generalization is complete only when the existing environment still works and a new user can configure arbitrary backends without private machine-specific data.

