# Tasks: Hybrid Tool Exposure

## Phase 1 - Contract and RED tests

- [x] T001 Add config parsing/migration tests for `exposure = "deferred"`.
- [x] T002 Add E2E test proving mixed direct/deferred list and call behavior.
- [x] T003 Add security assertions that proxy admin tools and non-deferred targets cannot be invoked through the meta-tool.

## Phase 2 - Implementation

- [x] T004 Add the public exposure enum and validation.
- [x] T005 Preserve configured deferred backend ids/prefixes as the invocation allowlist.
- [x] T006 Implement live raw-catalog search/describe and safe call dispatch.
- [x] T007 Register the Lomway-owned meta backend and retain upstream admin removal.
- [x] T008 Map deferred configuration to the upstream capability filter and let the reconnect monitor adopt late-started configured backends.

## Phase 3 - Production profile and documentation

- [x] T009 Mark Browser, CUA Windows, Chrome DevTools, UFO, Jev, and XMind deferred in the local production config.
- [x] T010 Update English/Japanese configuration, architecture, testing, ADR, and changelog documentation as applicable.

## Phase 4 - Verification

- [x] T011 Focused tests pass after RED/GREEN cycle, including late-started deferred backend adoption.
- [x] T012 `scripts/check.ps1`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked --all-targets` pass.
- [x] T013 Live Lomway `tools/list` shows 110 direct/meta tools: Workbridge 16, Memory 32, Praxiom 2, CUA Windows 57, and three `lomway_*` meta-tools; deferred namespaces contribute zero direct schemas.
- [x] T014 Live search/describe/call smoke succeeds for `browser_list_instances`; the Browser namespace remains absent from normal `tools/list`. E2E also proves a deferred backend started after gateway startup is adopted without a gateway restart.
- [x] T015 Feature diff/hygiene review passes and the existing unrelated `Cargo.lock` working-tree change remains unstaged and untouched. Repository-wide release privacy/dependency gates still report pre-existing sidecar test-email/history findings and the pre-existing rustls advisory; no public push is performed by this task.
