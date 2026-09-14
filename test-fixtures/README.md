# Portable test fixtures

Redistributable public configuration fixtures for the Lomway regression suite
(`tests/fixtures.rs`). They stand in for the machine-local, gitignored
`config/proxy.local.toml` so `cargo test --locked` reproduces the regression
on a clean machine with no private deployment.

## Inventory

| Fixture | Backends | What it proves |
| --- | --- | --- |
| `configs/zero-backends.toml` | 0 | an empty backend list is valid in the public schema, while the deployment gate still requires at least one backend (no safety reduction) |
| `configs/one-backend.toml` | 1 | a single required backend loads, validates, migrates, and serves |
| `configs/many-backends.toml` | 3 (2 required + 1 optional) | mixed populations; an unreachable optional backend degrades startup instead of failing it |

## Guarantees

- Loopback-only sample endpoints (`http://127.0.0.1:<port>/mcp`); the sample
  backend ports 18701-18703 and listener ports 18790-18792 are documentation
  ports that avoid the production default 17777. Replace them with your own
  endpoints or `${VAR}` environment references.
- No private service names, no credentials, no machine-specific paths, so the
  files are safe to redistribute in the public repository.
- Every fixture parses under the strict public schema (unknown keys are
  rejected) and passes `lomway::config::load_gateway_config` end to end.

## Usage

```text
cargo test --locked
cargo run --locked -- --config test-fixtures/configs/many-backends.toml --check
```

The live six-backend private deployment regression stays separate and is
skipped by default CI:

```text
cargo test --locked --test real_backends -- --ignored
```

That suite requires the machine-local backend set to be running and keeps
loading only the gitignored `config/proxy.local.toml`.
