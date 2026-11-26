# Read-only tool catalog snapshot and diff

## Problem

Operators cannot see the final client-visible catalog (prefixed names,
direct vs deferred) without starting the gateway and connecting a client.
Backend upgrades can silently change tool descriptions/schemas, and PAT tool
allowlists must be transcribed by hand.

## Requirements

1. `lomway catalog --config <path>` performs exactly one MCP `initialize` +
   `tools/list` per selected backend. It never sends `tools/call`, never
   retries, and never starts the listener.
2. Report per backend: id, prefix, exposure, `required`, availability, and
   final tool names; summarize direct and deferred counts.
3. An unavailable backend (required or optional) is reported, not fatal.
   `serve` fail-closed semantics are unchanged. Final-name collisions remain
   an error.
4. Each tool carries a SHA-256 fingerprint of its canonical client-relevant
   definition (excluding `name`, `icons`, `_meta`; key order ignored).
5. `--output` writes a `lomway.catalog.v1` JSON snapshot containing no URLs,
   credentials, descriptions, or schemas, and never overwrites a file.
6. `--diff <snapshot>` reports backend add/remove, exposure changes, and tool
   add/remove/change. Backends unavailable in either snapshot are "not
   compared" rather than reported as removals. `--fail-on-change` exits
   non-zero when the diff is not empty.
7. `--format names` prints one final tool name per line; `--backend <id>`
   (repeatable) filters, and an unknown id is an error.

## Out of scope

Auto-applying allowlists, changing PATs, hot-reloading config, or exposing
the catalog as an MCP tool.
