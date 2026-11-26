# Plan

- Reuse the existing startup probe (`backend::probe`) and extend
  `UpstreamTool` with a fingerprint so `serve` and `catalog` share one
  definition of the upstream catalog.
- Implement the command in `src/cli/catalog.rs`: collect, render
  (text/json/names), snapshot write (`create_new`), diff, and change gate.
- Resolve exposure/prefix from the validated public config; no new config
  keys.
- E2E with localhost mock MCP backends (`tests/catalog.rs`): exposure and
  availability reporting, no `tools/call`, snapshot redaction, added/changed
  diff, and outage-is-not-removal.
- Document in CONFIGURATION/OPERATIONS (EN/JA), README, CHANGELOG.
