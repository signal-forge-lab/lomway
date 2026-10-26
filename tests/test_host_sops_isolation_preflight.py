"""Read-only preflight must not claim that ACL inspection proves isolation."""

import json
import os
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "Test-HostSopsIsolation.ps1"


def test_preflight_reports_metadata_only_and_never_claims_isolation(tmp_path):
    profile = tmp_path / "profile"
    appdata = tmp_path / "appdata"
    local = tmp_path / "local"
    sops = profile / ".config" / "sops" / "secrets"
    age = appdata / "sops" / "age"
    pats = local / "Lomway" / "oauth-sidecar" / "runtime"
    for folder in (sops, age, pats):
        folder.mkdir(parents=True)
    markers = {
        sops / "global.sops.json": "SYNTHETIC_ENCRYPTED_METADATA_ONLY",
        age / "keys.txt": "SYNTHETIC_AGE_TEST_IDENTITY_NOT_REAL",
        pats / "pat-tokens.json": "SYNTHETIC_PAT_TEST_MARKER",
    }
    for filename, content in markers.items():
        filename.write_text(content, encoding="utf-8")

    env = dict(os.environ, USERPROFILE=str(profile),
               APPDATA=str(appdata), LOCALAPPDATA=str(local))
    run = subprocess.run(
        ["pwsh", "-NoProfile", "-NonInteractive", "-File", str(SCRIPT)],
        capture_output=True, text=True, errors="replace", timeout=15, env=env,
        check=False,
    )
    assert run.returncode == 0, run.stderr
    result = json.loads(run.stdout)
    assert result["verdict"] == "NOT_VERIFIED"
    assert result["separate_account_activation_tested"] is False
    assert {entry["name"] for entry in result["targets"]} == {
        "canonical_sops", "age_identity", "lomway_pat_store"
    }
    assert all(entry["exists"] for entry in result["targets"])
    assert all(marker not in run.stdout + run.stderr for marker in markers.values())
    assert str(tmp_path) not in run.stdout + run.stderr


def test_isolation_required_mode_fails_closed(tmp_path):
    env = dict(os.environ, USERPROFILE=str(tmp_path),
               APPDATA=str(tmp_path), LOCALAPPDATA=str(tmp_path))
    run = subprocess.run(
        ["pwsh", "-NoProfile", "-NonInteractive", "-File", str(SCRIPT),
         "-RequireIsolated"],
        capture_output=True, text=True, errors="replace", timeout=15, env=env,
        check=False,
    )
    assert run.returncode == 2
    assert json.loads(run.stdout)["verdict"] == "NOT_VERIFIED"
