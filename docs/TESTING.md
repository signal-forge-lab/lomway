# Testing

Updated: 2026-09-14

## 1. Release test layers

### Static / compiler gate

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

### Production-policy tests

11 policy tests cover loopback-only networking, HTTP-only local backends, stable namespace, hot-reload rejection, direct tool exposure, no auth-forwarding/schema rewriting, no retry/hedge/cache, no fan-out/failover/coalescing, public-release hygiene, and bounded argument size.

### Mock MCP E2E

13 E2E/fault tests cover control-plane suppression, HTTP-surface restriction, namespace/schema preservation, startup failure/collision behavior, runtime recovery/readiness, session failure recovery, and exactly-once mutation behavior. Key cases include:

- control-plane `proxy` backend removal;
- `/mcp` + `/healthz` + `/readyz` external HTTP surface and `/admin/*` 404 behavior;
- same-name tool namespace collision;
- failed startup backend isolation;
- timed-out mutation dispatched exactly once.

### Portable fixture regression

`tests/fixtures.rs` proves the regression suite reproduces on a clean machine with zero private deployment. Redistributable public configuration fixtures under `test-fixtures/configs/` cover the zero/one/many backend populations:

- `zero-backends.toml` — an empty backend list is valid in the public schema, while the deployment gate still requires at least one backend (no safety reduction);
- `one-backend.toml` — one required backend;
- `many-backends.toml` — required + optional mix; an unreachable optional backend degrades startup.

Coverage: strict public-schema loading and validation across all three populations, public↔legacy migration with the preserved ≥1-backend deployment gate, a pinned redistributable-inventory scan (no private service names, credentials, or machine paths), and live gateway E2E on the one/many fixtures — namespaced `tools/list` plus tool calls against mock MCP backends, including degrade-on-outage for the unreachable optional backend. Usage and guarantees are documented in `test-fixtures/README.md`.

### Real backend compatibility

`tests/real_backends.rs` is intentionally ignored in the default portable test suite and is explicitly run on the configured workstation. Default `cargo test --locked` therefore reports it as `1 ignored` instead of failing, which is what keeps CI green without the private deployment; `tests/fixtures.rs` guards that `--ignored` gating and its machine-local config binding. Validated result:

```text
workbridge 12
memory     32
ufo        19
browser    97
xmind      21
praxiom     2
```

All six initialized and listed tools successfully through the same pinned `tower-mcp` HTTP client stack.

### Aggregate integration smoke

`scripts/integration-smoke.ps1` verifies the running gateway:

At the 2026-09-14 re-review the live smoke reported **213 tools**, `proxy_tool_count = 0`, and **7/7 representative calls PASS**. The six original baseline namespaces remain mandatory for the private regression; an additional 30-tool Chrome DevTools namespace is allowed and was present in the reviewed workstation.

### Secure Tunnel

The native OpenAI Secure MCP Tunnel runtime is required to report READY. Its MCP probe must initialize the gateway successfully. The validated runtime negotiated MCP `2025-11-25` with the gateway built with upstream `protocol-2026-07-28` support.

This proves the secure tunnel path to the aggregate MCP endpoint. Attaching/removing ChatGPT UI connectors is a separate user migration action and is not performed as an automated test.

### Swibo

The live target must pass:

```text
READY -> restart -> READY -> stop -> STOPPED -> start -> READY
```

Swibo's own regression gate must also remain green.

## 2. Dependent-backend regression

Compatibility changes made only to support aggregation are tested in their owning projects:

- UFO wrapper Python compile + HTTP lifecycle/status;
- Stealth Browser Python compile/section inventory and HTTP tool discovery;
- XMind Workboard full unit suite, typecheck and build after capability-drift review;
- Swibo Node/Tauri regression gate after live target registration.

## 3. Required negative coverage

- non-loopback gateway listener;
- non-loopback/non-HTTP backend URL;
- wrong namespace separator;
- `hot_reload = true`;
- search/discovery exposure;
- retry/hedging/cache/fan-out/failover/coalescing configuration;
- schema/argument/visibility rewriting;
- upstream control-plane MCP tool exposure;
- top-level and nested admin HTTP path access;
- startup backend failure;
- backend timeout with no mutation replay.

There are no “missing/wrong gateway admin token” tests because v1 serves no admin plane and requires no admin token.

## 4. Current release evidence

- Rust fmt: PASS
- Rust clippy `-D warnings`: PASS
- lib tests: 50/50 PASS
- fixture tests: 5/5 PASS
- policy tests: 11/11 PASS
- proxy E2E/fault tests: 13/13 PASS
- real backend integration: 1/1 PASS, six backend inventories present
- real backend observation: baseline six namespaces remain present; extra Chrome tools are allowed and not baseline-required
- isolated release artifact build + clean-machine smoke: PASS
- independent 2026-09-14 release-build recheck: PASS in a fresh staging `CARGO_TARGET_DIR` while the live Windows Lomway process remained running; direct overwrite of the live `target/release/lomway.exe` was identified as an OS file-lock conflict, not a compile failure
- XMind Workboard: 14 files / 72 tests PASS; typecheck/build PASS
- Swibo: Node 38/38 PASS; Tauri 2/2 PASS; clippy PASS
- Secure Tunnel runtime/MCP probe: PASS

Final release additionally requires the public-repository secret/privacy/local-path scan and final code/security review; both are now automated/repeatable through the commands below.

Release-gate commands:

```powershell
python scripts/release_privacy_scan.py
python scripts/dependency_gate.py
pwsh -NoProfile -File scripts/clean-machine-smoke.ps1
```

The clean-machine smoke copies the compiled release artifact plus `test-fixtures/configs/one-backend.toml` into a fresh system temp directory and uses only the redistributable `public_mock_backend` example. It does not depend on the workstation's private MCP deployment.
