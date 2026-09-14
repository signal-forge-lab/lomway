# Migration and Rollback Guide

Updated: 2026-09-13

How to move an existing legacy deployment configuration (`mcp-proxy` `[proxy]` schema) to the public gateway schema (`schema_version = 1`), and how to roll back — with no destructive step at any point.

Japanese: [MIGRATION.ja.md](MIGRATION.ja.md)

## 1. The two configuration formats

| | Legacy deployment schema | Public schema |
|---|---|---|
| Marker | top-level `[proxy]` table | top-level `schema_version = 1` |
| Backends | `[[backends]]` with `name`, `transport`, `url`, `[backends.timeout]` | `[[backends]]` with `id`, `prefix`, `url`, `required`, `timeout_seconds` |
| Population rule | at least one backend (deployment gate, matching the pinned upstream loader) | zero, one, or many backends |
| Required/optional | implicit: an unreachable backend degrades startup | explicit per backend; `required = true` fails startup when unreachable |
| Detection | automatic at load time | automatic at load time |

Both formats remain first-class: `serve`, `check`, and `list-backends` detect the on-disk format automatically, report which one was used, and migrate a legacy file **in memory** — the legacy file itself is never modified by any command.

## 2. One-time migration to the public schema

```powershell
lomway migrate --config config/proxy.local.toml --output gateway.public.toml
```

Guarantees (all enforced and unit-tested):

- **Never overwrites.** The input is never mutated; an existing output file is refused, never overwritten. Delete the output explicitly to re-run. A migration is a mutation, and mutations are never automatically retried or forced.
- **`${VAR}` references are preserved verbatim** in the written file instead of baked-in resolved machine values. The fully resolved form is validated end to end before anything is written, and a second resolution pass must produce the identical URL or the migration refuses to write.
- **Degrade-on-outage behavior is preserved**: every migrated backend is written with `required = false`, so an unreachable backend degrades startup exactly as the legacy deployment behaved. Promote a backend to `required = true` only when you explicitly want fail-closed startup for it.
- **Migration is a pure in-memory transformation.** No private values are introduced into public files; the output contains only your configuration content (keep machine-local endpoint inventories in gitignored files, as before).

## 3. Field mapping

| Legacy | Public | Notes |
|---|---|---|
| `proxy.listen.host` / `proxy.listen.port` | `server.host` / `server.port` | loopback-only policy unchanged |
| `proxy.instructions` | `server.instructions` | passed through |
| `proxy.shutdown_timeout_seconds` | `server.shutdown_timeout_seconds` | passed through |
| `proxy.separator` | fixed to `_` | migration refuses any other separator |
| `security.max_argument_size` | `policy.max_argument_size_bytes` | default 1 MiB |
| `backends[].name` | `backends[].id` and `prefix = "<name>_"` | the prefix is the namespace contract |
| `backends[].timeout.seconds` | `backends[].timeout_seconds` | default 30 s |
| `backends[].transport` | HTTP only | non-HTTP backends refuse to migrate |
| `backends[].url` | `backends[].url` | must be `http://127.0.0.1:<port>/mcp` |
| — | `backends[].required` | new in the public schema; migration writes `false` |
| — | `policy.allow_non_loopback_listener` / `allow_non_loopback_backends` / `hot_reload` | always `false`; escape hatches fail closed |

Legacy-only middleware fields (retry, hedging, caching, mirror/canary, aliases, argument injection, exposure filters, auth forwarding) are deliberately **not** carried over: the public schema has no fields for them, and the gateway policy rejects them at startup.

## 4. Verify the migrated configuration

```powershell
lomway check --config gateway.public.toml --probe
lomway list-backends --config gateway.public.toml
lomway serve --config gateway.public.toml
```

`check --probe` contacts each configured backend exactly once and verifies final tool names; the printed report also states which on-disk format was loaded. The served tool catalog must be identical to the legacy one (`<name>_<tool>` names, unchanged descriptions and input schemas).

## 5. Rollback

Rollback is trivial because the migration is additive:

1. **Configuration rollback** — the original legacy file was never modified. Stop serving the public file and start from the legacy file again (or simply delete `gateway.public.toml`). Legacy format support is permanent, not a compatibility shim scheduled for removal.
2. **In-flight format behavior** — any command that finds a legacy file keeps working; no flag or conversion is needed to stay on the legacy path indefinitely.
3. **Source rollback** — the pre-generalization baseline is preserved as the root commit of `main` (LMG-G0 rollback point). Rebuilding the binary from that commit reproduces the recorded baseline behavior. The full worktree/rollback policy, including what must never be reset or discarded, is specified in [PUBLIC_GENERALIZATION_ROLLBACK.md](PUBLIC_GENERALIZATION_ROLLBACK.md) ([ja](PUBLIC_GENERALIZATION_ROLLBACK.ja.md)).
4. **Optional integrations are unaffected** — the OpenAI Secure MCP Tunnel, Swibo, SOPS, and PowerShell wrapper integrations live outside the core and require no migration.

## 6. Safety summary

- No step in migration or rollback touches backend processes; they are independently supervised and stay up throughout.
- No command retries automatically. A failed migration exits with an actionable error and can be re-run manually after fixing the cause.
- The deployment gate (legacy policy) keeps requiring at least one backend; the public schema alone accepts the zero-backend configuration.
