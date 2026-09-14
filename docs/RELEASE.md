# Release Metadata and Checklist

Updated: 2026-09-14

The release contract for Lomway: what identifies a release, where each piece of release metadata lives, and the exact gates that must pass before a release is cut. **This repository never publishes or pushes automatically** — tagging, artifact upload, and any external publication are explicit manual actions by the maintainer.

Japanese: [RELEASE.ja.md](RELEASE.ja.md)

## 1. Version

- Single source of truth: `version` in `[package]` of [`Cargo.toml`](../Cargo.toml). The CLI reports it via `lomway version` (together with the MCP protocol feature and the config `schema_version`), and `Cargo.lock` pins the resolved tree.
- Versioning follows [Semantic Versioning](https://semver.org/): breaking changes to the public configuration schema, the namespace contract (prefixes are part of the client-visible tool names), or the MCP surface bump the major/minor as appropriate. The current release is `0.1.0`.
- Config schema compatibility is versioned independently: `schema_version = 1` is the only supported public configuration schema in this release.

## 2. Changelog

- [`docs/CHANGELOG.md`](CHANGELOG.md) is the consumer-facing changelog, curated by impact (Added / Changed / Fixed / Security), newest first. It is written in the same change that makes the change, not reconstructed at release time.
- The changelog is maintained in English; the Japanese documentation set describes behavior, and release entries are linked from the Japanese docs where relevant.

## 3. Artifacts and checksums

Produced manually on the release machine, never by CI:

```powershell
$releaseTarget = Join-Path $env:TEMP ("lomway-release-" + [guid]::NewGuid().ToString('N'))
$env:CARGO_TARGET_DIR = $releaseTarget
cargo build --locked --release
Get-FileHash -Algorithm SHA256 (Join-Path $releaseTarget 'release/lomway.exe')
```

- The build must be `--locked`: the committed `Cargo.lock` is part of the reviewed release content.
- On Windows, if Lomway is currently running from `target/release/lomway.exe`, that executable is locked by the OS and an in-place release rebuild can fail with access denied. A fresh staging `CARGO_TARGET_DIR` is therefore the recommended release procedure; alternatively stop Lomway before rebuilding the in-place artifact.
- Record the SHA-256 of every published artifact (binary per platform and the source archive) in the release notes next to the artifacts they belong to.
- Checksums are properties of a concrete release artifact set; they are generated at release time and are not committed to the repository.

## 4. License

- `license = "MIT"` in `Cargo.toml`; the full license text is [`LICENSE`](../LICENSE).
- Dependency license review is a release gate: `python scripts/dependency_gate.py` verifies the exact committed lockfile against `cargo metadata --locked`, requires license metadata for every package, and queries OSV. The gate conservatively blocks any unresolved OSV record, therefore high/critical findings necessarily block release.

## 5. Source

- A release is an immutable tagged point in history: `git tag -a v<version> -m "Release <version>"`, created only after the gates below pass.
- The tagged tree must be the intended release tree: no local logs, runtime state, PID files, machine-local configuration, or generated tunnel profiles (all gitignored and verified absent before tagging).

## 6. Examples

Shipped with the repository and referenced by the docs:

- [`config/proxy.example.toml`](../config/proxy.example.toml) — public example configuration; environment-variable references only, no real endpoints.
- [`test-fixtures/configs/`](../test-fixtures/) — redistributable zero/one/many-backend configurations exercised by the test suite.

## 7. Pre-release gates

All of the following must pass on the intended release tree before tagging:

1. `cargo fmt --check --all` → exit 0
2. `cargo clippy --all-targets --locked -- -D warnings` → exit 0
3. `cargo test --locked` → green (mock backends only; the live six-backend regression stays `--ignored` and machine-local)
4. Locked release build → success. On an idle/clean machine `cargo build --locked --release` is sufficient; on a live Windows workstation use a fresh staging `CARGO_TARGET_DIR` so the running executable is never overwritten.
5. Docs match the actual CLI and configuration schema (commands in [QUICKSTART.md](QUICKSTART.md) and [CONFIGURATION.md](CONFIGURATION.md) run as written).
6. Privacy/hygiene scan: `python scripts/release_privacy_scan.py` must pass with no credentials, private identifiers, machine-specific paths, or secret values in the intended release tree.
7. Dependency/license/vulnerability gate: `python scripts/dependency_gate.py` must match the exact lockfile package set, find license metadata for every package, and return no unresolved OSV records.
8. Clean-machine artifact smoke: `pwsh -NoProfile -File scripts/clean-machine-smoke.ps1` copies the release binary and public one-backend fixture into a fresh temp directory, starts only the redistributable public mock backend, and proves `check`, `/healthz`, and `/readyz` without any private deployment backend.

## 8. What this repository deliberately does not do

- No automatic publish, tag push, artifact upload, or CI-triggered release. Every step above is manual.
- No retry of any release step; a failed gate fails and is investigated, not re-run blindly.
- The `repository` field in `Cargo.toml` is intentionally unset until the maintainer creates the public repository location; it must never point at a private or machine-specific location.
