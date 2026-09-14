# Core portability audit

The public Lomway core is platform-neutral Rust. The audit covers `src/` and `Cargo.toml`; optional PowerShell and workstation integrations remain outside Core under `scripts/` and `integrations/`.

## Findings

- `src/` contains no `cfg(windows)`, `cmd.exe`, PowerShell, `.exe`, `USERPROFILE`, `APPDATA`, or `LOCALAPPDATA` dependency.
- Configuration paths use `std::path::{Path, PathBuf}` and repository/current-working-directory relative defaults. No Windows path separator or user-home layout is required.
- Runtime networking uses loopback TCP/HTTP through Tokio/Axum and the pinned MCP libraries. Backend process lifecycle is external to Core.
- The CLI is a Rust binary for `serve`, `check`, `list-backends`, `migrate`, and `version`; no shell is required by those modes.
- Windows, Linux, and macOS run the same locked fmt/clippy/test/release-build CI gates. With this audit complete, all three matrix jobs are release-blocking.

Optional Windows PowerShell wrappers are convenience/integration surfaces, not Core runtime dependencies. The Windows clean-machine artifact smoke is separately enforced by `scripts/clean-machine-smoke.ps1`.

## Verification

```text
rg "cfg\(windows\)|cfg!\(windows\)|powershell|pwsh|cmd\.exe|USERPROFILE|APPDATA|LOCALAPPDATA|\.exe" src Cargo.toml
```

Expected result: zero matches except platform-neutral `std::process::ExitCode` / `PathBuf` usages when broader process/path searches are used.
