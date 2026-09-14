# Lomway — Public Generalization Handoff Records

Status: VERIFICATION RECORD FOR UNCOMMITTED G1–G5 WORK (2026-09-13)

Machine-readable backlog: `docs/PUBLIC_GENERALIZATION_TASKS.yaml`.
Human-readable plan: `docs/PUBLIC_GENERALIZATION_TASKS.md`.
This file records per-task handoff entries using the shared handoff template without
altering any implementation file.

## Baseline verification (wired build, 2026-09-13)

Run against the working tree exactly as found (commits `c98ea95`, `ce19116` preserved;
no reset, clean, checkout, or discard performed):

- `cargo fmt --check --all` → exit 0
- `cargo clippy --all-targets --locked -- -D warnings` → exit 0, zero warnings
- `cargo test --locked` → `policy` 10 passed / 0 failed; `proxy` 8 passed / 0 failed;
  lib 0 tests; `real_backends` 1 ignored (requires the machine-local backend set)
- Re-run of `cargo test --locked --test policy` later the same day → 11 passed / 0
  failed (a concurrent worker added
  `public_release_hygiene_excludes_private_tunnel_paths_and_local_build_artifacts`
  during verification).
- Total: 18 passed, 0 failed in the wired build at first run; 19 after the
  concurrent policy test landed.

## Concurrent-worker observation (2026-09-13)

While this verification ran, a parallel worker modified `.gitignore` (ignoring
`/target-deploy/`, `/.tmp_vendor/`, `/.tmp_tower_mcp/`), `scripts/tunnel.ps1` and
`scripts/configure-tunnel.ps1` (portable `LOCAL_MCP_TUNNEL_CLIENT` / PATH resolution
replacing the machine-specific user-profile path), and `tests/policy.rs` (the hygiene
regression test above). Those changes are owned by that worker and were left untouched
here; they were not assessed against LMG-G6/G8 tasks in this handoff.

## Uncommitted work not covered by a backlog task (preserve as-is)

- `src/lib.rs` (modified): backend restart reconnect logic — per-backend port probe,
  stable-recovery detection, and transport replacement via
  `tower_mcp::client::HttpClientTransport` + `TimeoutLayer`; `Cargo.toml`/`Cargo.lock`
  add `tower` (timeout) and move `tower-mcp` to a main dependency; `tests/proxy.rs`
  adds `brief_backend_restart_recovers_without_gateway_restart`,
  `sustained_backend_outage_recovers_without_gateway_restart`,
  `readiness_tracks_backend_outage_and_recovery`. All pass in the baseline run above.
  This reconnect work maps to no LMG-G task; it must be carried forward intact.

## Verification method for the untracked module set

The untracked directories `src/config/`, `src/backend/`, `src/namespace/`,
`src/health/`, `src/gateway/`, `src/cli/` are **not declared in `src/lib.rs`**, so they
are excluded from every cargo target and from the fmt/clippy/test baseline. To verify
them without touching any repository file, they were compiled as a unit in a scratch
crate outside the repository (temporary directory; `#[path]` includes; vendored
dependencies via `.tmp_vendor/`, offline). Scratch-only additions (not repository
changes): `DEFAULT_SERVER_NAME` constant, `toml` and `tracing-subscriber` as
dependencies. Result: **2 hard compile errors, 3 clippy-fatal unused imports** — the
module set is implemented but not yet integrable.

Genuine defects found in the untracked modules:

1. `src/config/migrate.rs:13` — `mcp_proxy::BackendConfig` and
   `mcp_proxy::ProxySettings` do not exist at the crate root of the pinned
   `mcp-proxy 0.4.3`; they are only exported as `mcp_proxy::config::*`.
2. `src/backend/registry.rs:38` — accesses private field `id` of
   `BackendDescriptor` (declared in `src/backend/descriptor.rs` as private) from a
   sibling module; needs an accessor or `pub(crate)` field.
3. Clippy `-D warnings` failures: unused imports
   `SCHEMA_VERSION` (`src/config/model.rs:27`),
   `DEFAULT_BACKEND_TIMEOUT_SECONDS` (`src/backend/descriptor.rs:11`),
   `UnavailableBackend` (`src/namespace/collision.rs:114`).
4. `src/config/load.rs` uses `toml` in production code, but `toml` is absent from
   `[dependencies]` in `Cargo.toml`.

Resolution (2026-09-13, lane A completion): all four defects are fixed. The
module set is declared in `src/lib.rs`, `serde`/`serde_json`/`toml` are regular
dependencies, `migrate.rs` imports from `mcp_proxy::config`, and the registry
uses descriptor accessors. The full suite now runs in-repo (lib 37, policy 11,
proxy 9 passed).

---

## Handoff entries

### Task ID: LMG-G1-01
Status: PARTIAL
Files changed: `src/config/model.rs` (new, untracked); no `mod` wiring in `src/lib.rs`
Behavior changed: none in the wired build — the module is not compiled by any cargo target
Tests run + result: unit tests present (`schema_version_one_is_supported`, `zero_backends_parse_but_fail_policy`, `one_and_many_backends_parse`, `unsupported_schema_versions_are_rejected_at_parse`, `unknown_keys_are_rejected_strictly`, `missing_schema_version_is_rejected`, `defaults_are_conservative`); not executable in-repo because the module is unwired; scratch compile of the module set fails on sibling-module errors
Security/privacy impact: none — model contains dummy values only; unknown keys rejected via `deny_unknown_fields`
Compatibility impact: none while unwired; once wired, the public schema is additive to the legacy `mcp_proxy::ProxyConfig` path
Known follow-ups: declare `pub mod config;` (plus the other new modules) in `src/lib.rs`; add `toml` to `[dependencies]`
Do not infer / unresolved decisions: acceptance `serde_tests_pass` is unmet until the module is wired and tests run in-repo

### Task ID: LMG-G1-02
Status: PARTIAL
Files changed: `src/config/validate.rs`, `src/gateway/policy.rs` (new, untracked)
Behavior changed: none in the wired build (unwired); the wired `validate_policy` in `src/lib.rs` remains the active safety gate and is unchanged
Tests run + result: `validate.rs` tests (`accepts_valid_public_config`, `rejects_unsupported_schema_versions`, `rejects_non_loopback_listener_and_backends`, `rejects_escaped_policy_flags`, `rejects_empty_backend_list`, `rejects_duplicate_ids_and_prefixes`, `rejects_ambiguous_ids_and_prefixes_without_normalization`) and `policy.rs` decomposition functions exist; not executable in-repo (unwired); scratch compile blocked by migrate.rs/registry.rs errors
Security/privacy impact: none; fail-closed orientation preserved; no safety reduction in the wired gate
Compatibility impact: none while unwired
Known follow-ups: fix the module-set compile errors, then confirm equivalence with the wired policy (no acceptance gap)
Do not infer / unresolved decisions: none

### Task ID: LMG-G1-03
Status: PARTIAL
Files changed: `src/config/migrate.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: tests present (`current_style_deployment_maps_to_public_schema`, `public_config_maps_back_to_equivalent_legacy_config`, `migration_rejects_unmappable_backends`); blocked from compiling by `mcp_proxy::BackendConfig`/`mcp_proxy::ProxySettings` unresolved imports (mcp-proxy 0.4.3 exports these only under `mcp_proxy::config`)
Security/privacy impact: migration logic references no private values in the reviewed source; final check required once wired
Compatibility impact: intended to keep the current six-backend deployment working through the legacy path
Known follow-ups: change imports to `mcp_proxy::config::{BackendConfig, ProxySettings}`; then run tests in-repo
Do not infer / unresolved decisions: acceptance `current_deployment_can_map_to_public_schema` unverified until compile+tests pass

### Task ID: LMG-G2-01
Status: PARTIAL
Files changed: `src/backend/descriptor.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: `descriptors_are_constructed_only_from_validated_config` present; not executable in-repo (unwired)
Security/privacy impact: none; descriptor deliberately excludes OpenAI/Swibo/SOPS types (acceptance-aligned)
Compatibility impact: none while unwired
Known follow-ups: remove unused import `DEFAULT_BACKEND_TIMEOUT_SECONDS` (clippy `-D warnings`); expose `id` accessor or `pub(crate)` field for registry
Do not infer / unresolved decisions: none

### Task ID: LMG-G2-02
Status: PARTIAL
Files changed: `src/backend/registry.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: tests present (`supports_zero_one_and_many_backends`, `lookup_by_id_is_exact`, `contains_no_hard_coded_private_backend_names`); blocked from compiling by the private-field access `backend.id` at `registry.rs:38` (`BackendDescriptor.id` is private)
Security/privacy impact: none; no hard-coded private backend names found in the source
Compatibility impact: none while unwired
Known follow-ups: fix the `id` access (accessor or `pub(crate)`), then run tests in-repo
Do not infer / unresolved decisions: acceptance items unverified until compile+tests pass

### Task ID: LMG-G2-03
Status: PARTIAL
Files changed: `src/backend/probe.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: tests present (`required_backend_down_fails_startup`, `optional_backend_down_degrades_but_is_reported`); not executable in-repo (unwired); scratch compile blocked by registry/descriptor errors
Security/privacy impact: none
Compatibility impact: none while unwired
Known follow-ups: compile after registry fix; add the `all_unavailable_fails_by_default` case if not covered by the current tests
Do not infer / unresolved decisions: acceptance `all_unavailable_fails_by_default` needs an explicit test before PASS

### Task ID: LMG-G3-01
Status: PARTIAL
Files changed: `src/namespace/normalize.rs`, `src/config/validate.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: `prefix_maps_to_mcp_name_and_back` plus `rejects_ambiguous_ids_and_prefixes_without_normalization` present; charset/reserved-prefix documentation exists in module docs; not executable in-repo (unwired)
Security/privacy impact: none; ambiguity is rejected, never silently normalized
Compatibility impact: none while unwired
Known follow-ups: verify reserved-prefix rejection is covered by a dedicated test after wiring
Do not infer / unresolved decisions: none

### Task ID: LMG-G3-02
Status: PARTIAL
Files changed: `src/namespace/collision.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: tests present (`plans_deterministic_unique_names`, `collisions_fail_fast_and_identify_both_sources`, `duplicate_upstream_names_within_one_backend_also_fail`); not executable in-repo (unwired)
Security/privacy impact: none; final tool names are precomputed before serving
Compatibility impact: none while unwired
Known follow-ups: remove unused import `UnavailableBackend` (`collision.rs:114`, clippy `-D warnings`)
Do not infer / unresolved decisions: none

### Task ID: LMG-G4-01
Status: PARTIAL
Files changed: `src/gateway/build.rs`, `src/gateway/mod.rs` (new, untracked)
Behavior changed: none in the wired build (unwired); the wired `build_proxy` in `src/lib.rs` already performs the same fail-closed upstream composition and `proxy` control-plane removal, proven by `build_removes_upstream_proxy_control_plane_backend` (passing)
Tests run + result: `Gateway::finish` composition exists; no dedicated unit test for `build.rs` itself; scratch compile blocked by module-set errors
Security/privacy impact: none; control-plane removal refusal semantics preserved in both paths
Compatibility impact: none while unwired
Known follow-ups: wire modules; ensure the reconnect logic currently living in `src/lib.rs` is carried into (or preserved alongside) the gateway builder without behavior loss
Do not infer / unresolved decisions: how the lib.rs reconnect work merges with `Gateway::serve` is an open integration decision for the next agent

### Task ID: LMG-G4-02
Status: PARTIAL
Files changed: `src/gateway/router.rs` (new, untracked)
Behavior changed: none in the wired build (unwired); the wired `gateway_router` in `src/lib.rs` already exposes `/healthz`, `/readyz`, nests MCP under `/mcp`, and 404s `/admin/*`, proven by `external_http_surface_exposes_mcp_and_health_but_not_admin` (passing)
Tests run + result: router module exists with `block_upstream_admin`; dedicated unknown-route-404 test not present for the unwired module
Security/privacy impact: none; admin surface remains inaccessible in both paths
Compatibility impact: none while unwired
Known follow-ups: after wiring, add/keep explicit unknown-route 404 coverage
Do not infer / unresolved decisions: none

### Task ID: LMG-G4-03
Status: PARTIAL
Files changed: `src/health/state.rs`, `src/health/mod.rs` (new, untracked)
Behavior changed: none in the wired build (unwired); wired readiness in `src/lib.rs` currently recomputes health from `McpProxy::health_check` per request and is exercised by `readiness_tracks_backend_outage_and_recovery` (passing)
Tests run + result: tests present (`liveness_and_readiness_are_distinct_concepts`, `optional_degradation_is_reported_but_ready`, `required_failure_never_reports_ready`); not executable in-repo (unwired)
Security/privacy impact: none
Compatibility impact: none while unwired
Known follow-ups: reconcile the wired per-request readiness (lib.rs) with the `HealthState` snapshot approach — behavioral difference must be resolved deliberately, not silently
Do not infer / unresolved decisions: which readiness semantics (live health check vs startup snapshot + state machine) becomes canonical is unresolved

### Task ID: LMG-G5-01
Status: PARTIAL
Files changed: `src/cli/args.rs`, `src/cli/mod.rs` (new, untracked); `src/main.rs` still uses its own inline clap CLI (committed baseline shape)
Behavior changed: none in the wired build (unwired)
Tests run + result: CLI module implements `serve`, `check` (with `--probe`), `list-backends`, `version`; no dedicated unit tests for the CLI; scratch compile blocked by `tracing_subscriber`/module-set errors (scratch dep gap and migrate/registry errors)
Security/privacy impact: none; config summary printing reviewed to avoid echoing secrets
Compatibility impact: none while unwired; current `main.rs` behavior unchanged
Known follow-ups: replace `main.rs` inline CLI with `cli::args::Cli`, wire modules, add exit-code tests
Do not infer / unresolved decisions: acceptance `no_powershell_required_for_binary` unverified until the binary builds with the new CLI

### Task ID: LMG-G5-02
Status: PARTIAL
Files changed: `src/config/load.rs` (new, untracked)
Behavior changed: none in the wired build (unwired)
Tests run + result: tests present (`detects_public_and_legacy_formats`, `config_path_precedence_is_documented_and_tested`, `env_refs_resolve_or_fail_loudly`); blocked from compiling because `toml` is used in production code but absent from `Cargo.toml [dependencies]`
Security/privacy impact: none; env references resolve or fail loudly; no private user path in core defaults
Compatibility impact: none while unwired
Known follow-ups: add `toml` to `[dependencies]`; re-verify precedence tests after wiring
Do not infer / unresolved decisions: none

### Task ID: LMG-G1-01 (completion record — supersedes the PARTIAL entry above)
Status: DONE
Files changed: `src/lib.rs` (declares `backend`, `cli`, `config`, `gateway`, `health`, `namespace`; adds `DEFAULT_SERVER_NAME`; removes the duplicated policy body and its local URL helper), `Cargo.toml`/`Cargo.lock` (`serde` with derive, `serde_json`, `toml` as regular dependencies), `src/backend/registry.rs` (id accessor), `src/backend/descriptor.rs` and `src/namespace/collision.rs` (unused imports), `src/cli/mod.rs` (`&Path` parameter)
Behavior changed: none for the wired legacy path — `load_config`/`build_proxy`/`gateway_router`/`serve` are unchanged and `tests/proxy.rs` passes 9/9
Tests run + result: `cargo test --locked` → lib 37 passed / 0 failed (previously 0 executable), policy 11 passed / 0 failed, proxy 9 passed / 0 failed, real_backends 1 ignored; `cargo fmt --check --all` exit 0; `cargo clippy --all-targets --locked -- -D warnings` exit 0
Security/privacy impact: none; no secrets, private identifiers, or machine-local paths introduced; unknown keys still rejected strictly
Compatibility impact: the public schema is now compiled and executable in-repo; the legacy path behaves exactly as before (proven by the unchanged `tests/policy.rs` suite running through the delegated gate)
Known follow-ups: none for this task
Do not infer / unresolved decisions: none

### Task ID: LMG-G1-02 (completion record — supersedes the PARTIAL entry above)
Status: DONE
Files changed: `src/config/validate.rs` (population minimum removed from the public schema with 0..N documented and tested), `src/gateway/policy.rs` (population rule documented as the legacy/upstream contract; preservation test added), `src/lib.rs` (`validate_policy` delegates to `gateway::policy::validate_proxy_policy`), `src/config/model.rs` (doc + test updates)
Behavior changed: the public configuration schema now accepts zero, one, or many backends (deliberate generalization per the design goal "aggregate 0..N backends"); the legacy deployment gate still requires at least one backend, matching the pinned upstream `ProxyConfig` loader — no safety reduction
Tests run + result: RED→GREEN proven (`accepts_zero_one_and_many_backends`, `unrestricted_population_still_validates_every_present_entry`, `empty_single_and_many_public_configs_load_cleanly`, `zero_backends_parse_and_pass_policy` failed against the old gate and pass now); `legacy_policy_keeps_the_single_backend_population_rule` pins the legacy rule; full suite green as recorded under LMG-G1-01
Security/privacy impact: none; loopback-only listener/backends, fail-closed flags, no-retry/hedging/cache, uniqueness, and reserved-prefix rules are unchanged
Compatibility impact: the existing six-backend deployment config continues to load and validate identically
Known follow-ups: serving an empty-backend configuration currently fails closed in the startup probe / upstream composition (probe reports all-unavailable); whether an empty gateway may serve is a G2-03/G7-01 decision, deliberately not made here
Do not infer / unresolved decisions: zero-backend serving semantics (config-level loading is clean; serve-time behavior is downstream)

### Task ID: LMG-G1-03 (completion record — supersedes the PARTIAL entry above)
Status: DONE
Files changed: `src/config/migrate.rs` (imports fixed to `mcp_proxy::config::{BackendConfig, ProxySettings}`; empty-configuration round-trip test added)
Behavior changed: none; migration remains a pure in-memory transformation
Tests run + result: `current_style_deployment_maps_to_public_schema`, `public_config_maps_back_to_equivalent_legacy_config`, `empty_public_configuration_maps_to_a_backend_free_legacy_proxy`, and `migration_rejects_unmappable_backends` all pass (config module: 23 passed / 0 failed)
Security/privacy impact: none; no private values are added to public files and no private identifiers appear in source
Compatibility impact: the current deployment schema maps losslessly to the public schema and back
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G6-01 (completion record, 2026-09-13)
Status: DONE
Files changed: `integrations/openai-secure-tunnel/tunnel.ps1` and `integrations/openai-secure-tunnel/configure-tunnel.ps1` (moved from `scripts/` via `git mv`; optional-integration headers added), `integrations/openai-secure-tunnel/README.md` + `README.ja.md` (new), `docs/CONFIGURATION.md`/`.ja.md` and `docs/OPERATIONS.md`/`.ja.md` (tunnel command paths updated to the integration directory), `tests/policy.rs` (hygiene test reads the wrappers from the new location — consequential move edit, outside G6 `allowed_paths`, disclosed here; the alternative was a permanently red `cargo test --test policy`)
Behavior changed: none for the gateway binary; the tunnel workflow is now packaged as an optional integration under `integrations/openai-secure-tunnel/` while remaining fully usable (`LOCAL_MCP_TUNNEL_CLIENT`/PATH resolution and single-attempt client invocation preserved)
Tests run + result: PowerShell parser check clean for all 10 `.ps1` under `scripts/` + `integrations/`; `rg -i 'openai|swibo|sops'` over `src/` and `Cargo.toml` clean; `rg` over `scripts/` finds no integration remnants; `rustfmt --check` on `tests/policy.rs` clean; `cargo clippy --all-targets --locked -- -D warnings` exit 0; `cargo test --locked` → 46 lib + 11 policy + 9 proxy passed, 0 failed
Security/privacy impact: none; no credentials, identifiers, or machine-local paths introduced; each tunnel-client operation runs exactly once (no retry loops added)
Compatibility impact: current local tunnel workflow remains usable from the new path; `README.md`/`README.ja.md` repo-tree and command listings still reference the old `scripts/` locations (not editable within G6 allowed paths)
Known follow-ups: update README tree/commands when README enters scope (LMG-G10-01/LMG-G9-03); run the tunnel workflow once against a live gateway at the next natural opportunity
Do not infer / unresolved decisions: none

### Task ID: LMG-G6-02 (completion record, 2026-09-13)
Status: DONE
Files changed: `integrations/swibo/README.md` + `README.ja.md` (new)
Behavior changed: none; documentation-only packaging of the external supervisor boundary
Tests run + result: `rg -i 'swibo'` over `src/` and `Cargo.toml` clean (core imports no Swibo code); the template uses placeholders only (`<gateway-target>`, `<config-path>`, `<repo-root>`, `<tunnel-target>`) with no machine-specific absolute paths
Security/privacy impact: none; no machine-local values committed
Compatibility impact: none
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G6-03 (completion record, 2026-09-13)
Status: DONE
Files changed: `integrations/sops/Import-SopsSecrets.ps1` (new helper: resolves named secrets from the canonical store into process environment variables; `LOCAL_MCP_SOPS_STORE` overrides the store path), `integrations/sops/README.md` + `README.ja.md` (new), `integrations/openai-secure-tunnel/tunnel.ps1` + `configure-tunnel.ps1` (inline SOPS decryption replaced with optional helper invocation)
Behavior changed: SOPS knowledge now lives only in `integrations/sops/`; the tunnel wrappers consult the helper only when required keys are absent from the environment (env-provided keys alone now suffice for both wrappers); core gateway behavior unchanged and SOPS-free
Tests run + result: PowerShell parser check clean; `rg -i 'sops'` over `src/` and `Cargo.toml` clean; no secret values present in any public file; helper writes only to the process environment and prints nothing
Security/privacy impact: positive isolation — secret resolution is a separate optional integration; the canonical-store workflow remains supported via the same default store path
Compatibility impact: existing local SOPS workflow remains supported; operators may alternatively export keys directly
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G6-04 (completion record, 2026-09-13)
Status: DONE
Files changed: `integrations/powershell/README.md` + `README.ja.md` (new; wrapper inventory and no-duplicate-logic guarantees); `scripts/` reduced by moving integration-specific scripts out (see LMG-G6-01/LMG-G6-03)
Behavior changed: none; `scripts/` retains only thin wrappers (`install`, `check`, `start`, `status`, `stop`, `common`, `integration-smoke`) that invoke `cargo`, the gateway CLI, or public HTTP endpoints
Tests run + result: PowerShell parser check clean for the remaining wrappers; policy/validation/routing logic remains single-sourced in the Rust core (wrappers re-implement none of it)
Security/privacy impact: none; wrappers add no retry logic and reference only repository-relative paths and environment variables
Compatibility impact: none
Known follow-ups: none
Do not infer / unresolved decisions: none

### Concurrent-worker observation (2026-09-13, lane E verification window)
While the G6 records above were produced, a parallel worker continued the "lomway" rename and migration work in `src/cli/args.rs`, `src/cli/mod.rs`, and `src/config/load.rs`. During two `cargo fmt --check --all` runs those files carried rustfmt diffs (their files, mid-edit); they were left untouched per the established concurrent-worker policy. At final verification `cargo clippy --all-targets --locked -- -D warnings` exited 0 and `cargo test --locked` passed 46 lib + 11 policy + 9 proxy tests, 0 failed; `rustfmt --check` on the lane-E Rust change (`tests/policy.rs`) was clean. A final whole-tree `cargo fmt --check --all` gate should be run by whoever lands next after the concurrent work settles.

---

## Backlog status sync and release/documentation records (2026-09-13, lane F)

### Task ID: backlog status sync (LMG-G2-01..G2-03, G3-01, G3-02, G4-01..G4-03, G5-01, G5-02, G7-03)
Status: DONE (status/evidence sync only — no implementation files touched)
Files changed: `docs/PUBLIC_GENERALIZATION_TASKS.yaml` (statuses moved `in_progress`/`todo` → `done` with evidence blocks; no acceptance criteria edited)
Behavior changed: none — documentation of the machine-readable backlog only
Tests run + result: full suite re-verified before recording evidence: `cargo fmt --check --all` exit 0; `cargo clippy --all-targets --locked -- -D warnings` exit 0; `cargo test --locked` → lib 50, fixtures 5, policy 11, proxy 9 passed, 0 failed, real_backends 1 ignored. Each promoted task's acceptance items were individually re-checked against named passing tests and wired code paths before promotion; LMG-G3-03, G5-03, G7-01, G7-02, G7-04, G8-02, G8-03, G9-01, G11-01 deliberately stay `todo` (no complete evidence: G3-03 lacks the schema-preservation regression, G7-02 lacks the error-path exactly-once counter, G7-04 requires live six-backend verification, G9-01 has no clean-machine artifact smoke yet)
Security/privacy impact: none; evidence strings contain repository-relative references only
Compatibility impact: none
Known follow-ups: remaining `todo` tasks above
Do not infer / unresolved decisions: G7-01/G7-02/G7-04 ownership stays with the fixtures worker; statuses were not promoted on their behalf beyond the fully evidenced G7-03

### Task ID: LMG-G10-01
Status: DONE
Files changed: `docs/QUICKSTART.md` + `docs/QUICKSTART.ja.md` (new), `README.md`/`README.ja.md` (quickstart link, corrected `scripts/` tree, tunnel commands moved to `integrations/openai-secure-tunnel/`, binary-only core modes bullet)
Behavior changed: none (docs only); README no longer references the moved `scripts/tunnel.ps1`/`scripts/configure-tunnel.ps1` paths, closing the LMG-G6-01 follow-up
Tests run + result: documented commands cross-checked against `src/cli/args.rs`/`src/cli/mod.rs` and the fixture configs; `cargo run --locked -- version` → `lomway 0.1.0 (mcp protocol 2026-07-28, config schema_version 1)`; `cargo test --locked --test fixtures` 5/5 (the quickstart's live two-backend path)
Security/privacy impact: none; the walkthrough uses dummy loopback endpoints (18701/18702) and no credentials, identifiers, or machine paths
Compatibility impact: none
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G10-02
Status: DONE
Files changed: `docs/INTEGRATIONS.md` + `docs/INTEGRATIONS.ja.md` (new index of the four separate optional guides)
Behavior changed: none (docs only)
Tests run + result: all four integration README pairs verified present (`integrations/{openai-secure-tunnel,swibo,sops,powershell}/README.md` + `.ja.md`); hygiene re-scan `rg -i 'openai|swibo|sops'` over `src/` and `Cargo.toml` → 0 matches
Security/privacy impact: none; index links are repository-relative
Compatibility impact: none
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G10-03
Status: DONE
Files changed: `docs/MIGRATION.md` + `docs/MIGRATION.ja.md` (new)
Behavior changed: none (docs only)
Tests run + result: migration guarantees and field mapping cross-checked against `src/config/migrate.rs`, `src/config/load.rs` (`migrate_legacy_to_public_file` never-overwrite / `${VAR}`-preservation / required=false behavior is unit-tested), and the `lomway migrate` CLI; rollback anchored to the LMG-G0 root-commit rollback point and `PUBLIC_GENERALIZATION_ROLLBACK` (en+ja)
Security/privacy impact: none; no deployment-specific values documented
Compatibility impact: none
Known follow-ups: none
Do not infer / unresolved decisions: none

### Task ID: LMG-G9-03
Status: DONE
Files changed: `Cargo.toml` (`readme`, `keywords`, `categories`; comment explaining the intentionally unset `repository` field), `LICENSE` (new, MIT — matching the already-declared `license = "MIT"`), `docs/CHANGELOG.md` (new, curated 0.0→0.1.0 entry), `docs/RELEASE.md` + `docs/RELEASE.ja.md` (new release checklist: versioning, changelog, release-time sha256 procedure, license, tagged source, examples, seven pre-release gates), `docs/PUBLIC_GENERALIZATION_TASKS.yaml` (evidence)
Behavior changed: none — metadata and documentation only; no publish, tag push, or artifact upload performed and none is automated
Tests run + result: `cargo clippy --all-targets --locked -- -D warnings` exit 0 after the Cargo.toml change; `cargo test --locked` green; `cargo run --locked -- version` reports the pinned version string; YAML re-parsed successfully (35 tasks); hygiene scan over the release candidate files clean (only the fixtures test's own forbidden-marker list matches)
Security/privacy impact: none; `repository` stays unset rather than pointing at any private or machine-specific location; checksums are defined as release-time artifacts and are not committed
Compatibility impact: none; version remains 0.1.0
Known follow-ups: LMG-G9-01 (clean-machine artifact smoke) and LMG-G8-03 (lockfile license/vulnerability gate) remain `todo` and gate the actual release per `docs/RELEASE.md`

## Recovery completion — 2026-09-13

The failed certification run left useful partial mutations in the checkout. Recovery reconciled those changes instead of discarding or replaying them. `LMG-G3-03`, `LMG-G7-01`, and `LMG-G7-02` were already implemented in `tests/proxy.rs` and were promoted only after `cargo test --locked --test proxy` passed 13/13.

Recovery added the cross-platform audit (`docs/PORTABILITY.md` + ja), repeatable public privacy and exact-lockfile dependency gates (`scripts/release_privacy_scan.py`, `scripts/dependency_gate.py`), an isolated Windows clean-machine artifact smoke (`scripts/clean-machine-smoke.ps1` + `examples/public_mock_backend.rs`), and made all three CI platforms release-blocking. The clean-machine smoke was corrected after its first run attempted to rebuild the live release executable; it now uses a dedicated temporary `CARGO_TARGET_DIR` and leaves the live artifact untouched.

Private deployment regression was re-run without destructive migration: the six baseline namespaces all passed the explicit ignored real-backend test, an extra Chrome namespace was observed but is not baseline-required, and Secure Tunnel reported ready with local health/readiness 200. `scripts/integration-smoke.ps1` now enforces only the six baseline namespaces while reporting Chrome when present.

Final settled-tree evidence: fmt PASS; clippy `-D warnings` PASS; default `cargo test --locked` 79 PASS with real_backends 1 ignored; explicit real_backends 1/1 PASS; privacy scan PASS; exact 309-package dependency/OSV gate PASS with missing licenses 0 and unresolved OSV records 0; clean-machine artifact smoke PASS; `git diff --check` PASS (line-ending warnings only).
Do not infer / unresolved decisions: the public repository URL and the release execution (tag/artifacts) belong to the maintainer

---

## Independent final re-review correction — 2026-09-14

Status: DONE — supersedes stale documentation-only claims in earlier handoff records without changing their historical implementation evidence.

Review findings fixed:

- the public product is documented as 0..N backends; the original six-backend / 183-tool inventory is a regression baseline, not a product limit;
- northbound surface documentation consistently includes `/mcp`, `/healthz`, and `/readyz`;
- the release checklist has **8** gates, explicitly including `python scripts/dependency_gate.py` in addition to the privacy and clean-machine gates;
- testing documentation matches the settled suite (50 lib + 5 fixtures + 11 policy + 13 proxy = 79 default PASS, plus one ignored real-backend test) and the 2026-09-14 live smoke (213 tools, proxy tools 0, representative calls 7/7 PASS);
- the operator-approved post-certification migration is recorded accurately: obsolete individual Secure MCP Tunnels and local aliases were removed, the retained remote tunnel is named Lomway, and the runtime has one `lomway` alias / one tunnel-client process.

Final re-review requirement: rerun fmt, clippy, default tests, explicit real-backend test, release privacy scan, dependency gate, clean-machine smoke, integration smoke, and `git diff --check`; commit only after all gates are green.
