# Changelog

All notable changes to Lomway are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project follows [Semantic Versioning](https://semver.org/). This changelog is curated for consumers and is intentionally not a commit log. The changelog is maintained in English; behavior documentation exists in English and Japanese under `docs/`.

## [0.1.0] - 2026-09-13

Initial public release of the local MCP aggregation gateway.

### Added

- Thin loopback aggregation boundary for independently managed local MCP backends, built as a library host over pinned `mcp-proxy` 0.4.3 with the explicit `protocol-2026-07-28` feature and a committed, reviewed `Cargo.lock`.
- Versioned public configuration schema (`schema_version = 1`) accepting zero, one, or many backends with strict unknown-key rejection; legacy `[proxy]` deployment configurations remain first-class and are detected and migrated in memory at load time.
- `lomway migrate`: one-time conversion of a legacy configuration to the public schema as a new file — never overwrites input or output, preserves `${VAR}` references verbatim, and keeps migrated backends optional to preserve degrade-on-outage behavior.
- Deterministic backend registry and namespace contract: `<backend>_<tool>` naming with the `_` separator, allowed prefix charset, reserved prefixes (`proxy_`, `lomway_`, legacy `lmg_`) rejected, ambiguous inputs rejected without normalization.
- Startup semantics: per-backend startup probe (required backends fail startup when unreachable; optional backends degrade; all-unavailable fails by default), tool collision preflight that fails fast while naming both sources, and live `/readyz` recomputation from backend health after startup.
- Public CLI implementing every core mode in the binary itself — `serve`, `check` (`--probe` doctor diagnostic), `list-backends`, `migrate`, `version` — with actionable exit codes and no shell or PowerShell requirement; configuration path precedence `--config` > `LOMWAY_CONFIG` > `config/proxy.local.toml` > `gateway.toml`.
- External HTTP surface limited to `/mcp`, `/healthz`, and `/readyz` on the loopback listener; the upstream `/admin/*` management plane is unreachable at both the top level and under `/mcp`, and the upstream `proxy` control-plane backend is removed before serving with proof-or-refuse semantics.
- Backend restart recovery: a backend that comes back after a brief or sustained outage is re-adopted automatically via transport replacement, without a gateway restart; readiness tracks outage and recovery.
- Portable test suites: mock-backend proxy regressions (namespacing, exactly-once timeout behavior for mutating tools, backend failure isolation and recovery, admin-surface isolation), policy/validator regressions, and redistributable zero/one/many-backend fixtures under `test-fixtures/configs/` with a pinned-inventory privacy scan; the live six-backend regression stays `--ignored` and machine-local.
- Documentation set in English and Japanese: requirements, architecture, threat model (13 threats mapped to mitigations), configuration, testing, operations, security, quickstart, optional-integration index, migration/rollback guide, and release checklist.
- Optional integrations packaged separately under `integrations/`: OpenAI Secure MCP Tunnel, Swibo, SOPS, and PowerShell wrappers; the core builds, tests, and serves with none of them.
- CI matrix running locked fmt/clippy/test/build gates with Windows, Linux, and macOS all release-blocking; no retry steps anywhere.

### Security

- Loopback-only listener and backend policy enforced at the defaults, schema, and policy layers; non-loopback escape hatches fail closed.
- No automatic retry, hedging, fan-out/failover, or tool-call response caching anywhere in the gateway; a timed-out mutation is dispatched exactly once and surfaces as an error.
- No gateway-side secret: the served surface has no admin plane, so the core needs no credential; tunnel credentials stay in the operator's environment/optional SOPS integration and never enter the repository.
- Upstream tool descriptions and input schemas are preserved verbatim; the gateway performs no schema rewriting, argument reshaping, or semantic collapsing.
