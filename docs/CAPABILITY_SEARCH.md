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
- **`status` describes provenance, not health.** `listed` means the tool is
  in the gateway's current `tools/list` catalog (direct tools, or a deferred
  tool just after an explicit describe); it does not claim the backend is
  healthy or that a call will succeed. `cached` and `unverified` are as
  above, and `declared` marks an internal capability.
- **Results are scoped to the caller.** For PAT and restricted OAuth
  callers, the northbound authorization layer passes the caller's tool
  allowlist to search/describe (overwriting any caller-supplied value).
  Tools the caller may not use are removed before ranking, and describing
  one returns the same error as a nonexistent tool. For these callers an
  unknown `backend` filter returns the same empty result as a configured
  backend they cannot see; unrestricted callers still get an error for an
  unknown backend id.
- **Meta-tool descriptions are generic.** `tools/list` is visible to every
  caller, so the `lomway_search_tools`, `lomway_describe_tool`, and
  `lomway_call_tool` descriptions name no configured backend ids and do not
  repeat `server.instructions` (which reaches clients through the MCP
  `instructions` field). They state the routing rule instead: direct results
  are called directly, internal results through their `via` tool, and only
  deferred results use describe then call.
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
the `via` tool, or an ordered list of candidate tools. An internal
capability is indexed (and describable) only while one of its `via`
candidates is a direct tool in the gateway's current `tools/list`; the first
listed candidate is reported. A profile that exposes none of them never
offers the capability, so search cannot route a caller to a tool that does
not exist. Tool-keyed declarations likewise only annotate tools that are
listed. The file is public: names, aliases, and summaries only.
Validation covers every field (including `[[internal]]` `domain`) and
rejects URLs, loopback/private/link-local/CGNAT IPv4 and IPv6 addresses,
`host:port` bindings, absolute POSIX paths, `~/` paths, Windows and UNC
paths, and email addresses. Slashes between ordinary words (`read/write`)
are allowed.

## Manifest cache

`lomway serve` stores deferred manifests in the per-user state directory:
`LOMWAY_STATE_DIR` if set, otherwise `%LOCALAPPDATA%\Lomway\state` on
Windows, `~/Library/Application Support/Lomway/state` on macOS, and
`$XDG_STATE_HOME/lomway` (or `~/.local/state/lomway`) elsewhere. One file per
gateway identity (listener port plus deferred backend ids and prefixes).
Each backend's cached manifest is also keyed by a SHA-256 fingerprint of
the settings that decide where the gateway connects (transport and resolved
URL). When those settings change, the old manifest is ignored and the
backend is reported as `unverified` until it is observed again. Only the
hash is written to the file name and cache contents; endpoint values are
never stored or logged.
Writes stage into a temporary file unique to the process and write, are
serialized within the process, and are renamed over the cache file, so
concurrent observations cannot truncate each other or leave an older
snapshot in place.
Manifests are refreshed at startup and after explicit describe/call
activations. Library callers using `Gateway::build` keep manifests in memory
only.
