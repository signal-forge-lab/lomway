# SOPS Integration (optional)

Status: OPTIONAL INTEGRATION

This directory packages the SOPS secret-resolution workflow. The core gateway
does **not** require SOPS: it reads plain process environment variables only,
so a deployment can supply secrets without SOPS installed.

## Core independence

- `src/` contains no SOPS references and `Cargo.toml` carries no SOPS
  dependency; `cargo build` / `cargo test` succeed on a machine without the
  `sops` binary.
- The canonical secret source remains the user's external global SOPS store.
  Nothing is committed to this repository; no secret values appear in any
  file under `integrations/sops/`.

## Files

| File | Purpose |
| --- | --- |
| `Import-SopsSecrets.ps1` | Decrypts the canonical store once and copies named secrets into process-level environment variables. |

## Usage

Resolve named secrets into the current process environment:

```powershell
pwsh -NoProfile -File integrations/sops/Import-SopsSecrets.ps1 -Names @('CONTROL_PLANE_API_KEY')
```

Then launch the gateway normally; environment references in the configuration
resolve from the injected variables:

```powershell
pwsh -NoProfile -File scripts/start.ps1
```

The store location defaults to `%USERPROFILE%\.config\sops\secrets\global.sops.json`
and can be overridden for the current process with `LOCAL_MCP_SOPS_STORE`.

## Guarantees

- Secret values are injected into the process environment only; they are
  never written to disk or printed.
- Each invocation decrypts and applies exactly once; nothing here retries
  mutating operations automatically.
