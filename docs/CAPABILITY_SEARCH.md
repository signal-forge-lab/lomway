# Capability search

[日本語](CAPABILITY_SEARCH.ja.md)

`lomway_search_tools` is the single place an agent asks "does this gateway
already have X?". It searches three surfaces with one ranking:

| `invocation` | What it is | How to use a result |
|---|---|---|
| `direct` | A tool already listed in `tools/list` | Call it directly by name. Never through `lomway_call_tool`. |
| `deferred` | A tool of an `exposure = "deferred"` backend | `lomway_describe_tool`, then `lomway_call_tool`. |
| `internal` | A declared capability with no MCP tool of its own (for example a CLI reached through a shell tool) | Use the tool named in `via`. |

## Guarantees

- **Search never starts a backend.** Direct tools come from the gateway's
  cached aggregated catalog. Deferred tools come from the last observed
  manifest (`status = "cached"`, with `manifest_seen_at` in RFC 3339 UTC).
  A deferred backend whose tools were never observed is returned as a
  `<prefix>*` placeholder with `status = "unverified"`; describing that
  placeholder is the explicit action that connects the backend.
- **Results are scoped to the caller.** For PAT and restricted OAuth
  callers, the northbound authorization layer passes the caller's tool
  allowlist to search/describe (overwriting any caller-supplied value).
  Tools the caller may not use are removed before ranking, and describing
  one returns the same error as a nonexistent tool.
- **Unrelated queries return nothing.** Results below a score threshold are
  dropped.

## Ranking

Signals, strongest first: upstream OSS name, declared aliases, action names
and action aliases, tool/backend name, description text. Matching is exact,
containment, ASCII word overlap, and character bigrams for Japanese, after
lowercasing and folding full-width ASCII. One result is returned per tool;
`action` names the best matching action when the match is meaningful.

## Declarations

`capabilities/catalog.toml` declares OSS names, domains, aliases, and
per-action aliases for a `tool` (exact name) or a `backend` (every tool of
that backend, plus its unverified placeholder). `[[internal]]` entries name
the `via` tool. The file is public: names, aliases, and summaries only.
Validation rejects URLs, loopback addresses, and machine paths.

## Manifest cache

`lomway serve` stores deferred manifests in the per-user state directory:
`LOMWAY_STATE_DIR` if set, otherwise `%LOCALAPPDATA%\Lomway\state` on
Windows, `~/Library/Application Support/Lomway/state` on macOS, and
`$XDG_STATE_HOME/lomway` (or `~/.local/state/lomway`) elsewhere. One file per
gateway identity (listener port plus deferred backend ids and prefixes).
Manifests are refreshed at startup and after explicit describe/call
activations. Library callers using `Gateway::build` keep manifests in memory
only.
