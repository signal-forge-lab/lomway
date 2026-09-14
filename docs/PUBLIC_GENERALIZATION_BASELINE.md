# Public Generalization — G0 Preflight Baseline

Status: CAPTURED

Task: LMG-G0-01 (`docs/PUBLIC_GENERALIZATION_TASKS.yaml`).

This document records the verified pre-generalization baseline of Lomway (formerly Local MCP Gateway).
It contains no secret values, no machine-local URLs, and no user-specific absolute paths.

## Environment

| Item | Value |
| --- | --- |
| Date (UTC) | 2026-09-13 |
| rustc | 1.98.0 |
| cargo | 1.98.0 |
| Locked dependency set | `Cargo.lock` present, all runs used `--locked` |
| mcp-proxy | pinned `=0.4.3`, feature `protocol-2026-07-28` |

## Git state at capture time

- Branch `main` exists only as `refs/heads/main`; the repository has **zero commits** (unborn branch).
- Working tree status: **54 entries, none reset or discarded during capture**.
  - 43 staged new files (`A`)
  - 4 staged files with additional unstaged modifications (`AM`): `README.md`, `README.ja.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/IMPLEMENTATION_PLAN.ja.md`
  - 7 untracked files (`??`): the `docs/PUBLIC_GENERALIZATION_*` design/task set including `PUBLIC_GENERALIZATION_TASKS.yaml`
- `target/` and `config/proxy.local.toml` are gitignored; build and test runs did not dirty the tree.
- `git status --porcelain` was identical (54 entries, no stashes) immediately before and after all
  verification runs. The only later mutations are the G0 evidence/policy documents themselves
  (see `docs/PUBLIC_GENERALIZATION_ROLLBACK.md`); no source, config, or test code was modified.

## Namespace baseline

- Separator policy: `_` (enforced by `validate_policy`).
- Production namespace count: **6** (verified by `--check` against the machine-local config).

| # | Namespace (backend name) |
| --- | --- |
| 1 | `workbridge` |
| 2 | `memory` |
| 3 | `ufo` |
| 4 | `browser` |
| 5 | `xmind` |
| 6 | `praxiom` |

Tool names are served as `<namespace>_<tool>` (for example `browser_navigate`). The upstream
`proxy` control-plane namespace is removed at build time and is not client-visible.
Namespace names above are already public in `config/proxy.example.toml`; no URLs are recorded here.

## Test baseline

| Suite | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --locked -- -D warnings` | exit 0, no warnings |
| Unit (library) | `cargo test --locked --lib` | 0 tests, 0 failed (no `#[cfg(test)]` modules exist in `src/` yet) |
| Policy validation | `cargo test --locked --test policy` | **10 passed, 0 failed** |
| Mock backend E2E | `cargo test --locked --test proxy` | **5 passed, 0 failed** |
| Real backend regression | `cargo test --locked --test real_backends -- --ignored` | **1 passed, 0 failed** (all 6 local backends reachable; `tools/list` counts: workbridge 12, memory 32, ufo 19, browser 97, xmind 21, praxiom 2 — 183 tools total) |
| Production config check | `cargo run --locked -- --config config/proxy.local.toml --check` | exit 0 |

Totals: **16 passing test functions** executed (15 non-ignored across `policy` + `proxy`, plus the 1
ignored real-backend test run explicitly). **Zero baseline failures.**

Notes:

- The library currently exposes no unit tests; `tests/policy.rs` (10 tests) is the config/policy
  validation suite and `tests/proxy.rs` (5 tests) is the mock-backend E2E suite using `tower-mcp`.
- The real-backend test is `#[ignore]`d by default because it requires the machine-local backend
  set to be running; it was run explicitly and passed against all six namespaces.
- Mock E2E already proves: upstream `proxy` backend removal, `/healthz` present + `/admin/*` 404,
  namespaced disambiguation without collision, degraded startup with one offline backend, and
  exactly-once dispatch of a mutation tool on timeout.

## Secret/privacy review of this artifact

- No bearer tokens, API keys, private keys, emails, user paths, or live identifiers.
- Backend namespace names are public (they appear in `config/proxy.example.toml`).
- The machine-local configuration file itself is gitignored and is not reproduced here.

## Reproduction

All commands are run from the repository root on Windows with the toolchain versions listed above.
The real-backend suite additionally requires the six supervised local backends to be up.

## Verification provenance

- Captured on 2026-09-13 and independently re-verified the same day with fresh runs of every
  command above; all results were identical (fmt/clippy exit 0; 0 + 10 + 5 + 1 tests passing;
  six namespaces reported by `--check`).
- Rollback policy binding this baseline: `docs/PUBLIC_GENERALIZATION_ROLLBACK.md`
  (`docs/PUBLIC_GENERALIZATION_ROLLBACK.ja.md`).
