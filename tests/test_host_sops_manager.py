"""Real SOPS/age CLI end-to-end tests using only disposable synthetic secrets."""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "Invoke-HostSops.ps1"


def run(args, *, env=None, cwd=None):
    return subprocess.run(
        [str(arg) for arg in args],
        capture_output=True,
        text=True,
        errors="replace",
        timeout=35,
        env=env,
        cwd=cwd,
        check=False,
    )


@pytest.fixture()
def isolated_sops(tmp_path):
    # Substitute USERPROFILE/LOCALAPPDATA only in the child process. The
    # production script retains its fixed per-user canonical relative path.
    home = tmp_path / "testhome"
    local = tmp_path / "local"
    secrets = home / ".config" / "sops" / "secrets"
    secrets.mkdir(parents=True)
    local.mkdir()
    keyfile = tmp_path / "test-age-key.txt"
    keygen = run(["age-keygen", "-o", keyfile], cwd=tmp_path)
    assert keygen.returncode == 0, "age-keygen could not create a test key"
    pub = re.search(
        r"public key:\s*(age1[a-z0-9]+)",
        keyfile.read_text(encoding="utf-8") + keygen.stderr + keygen.stdout,
        re.IGNORECASE,
    )
    assert pub, "age-keygen did not report a synthetic public key"
    canonical = secrets / "global.sops.json"
    canonical.write_text('{"KNOWN_TEST_SECRET":"synthetic-before"}', encoding="utf-8")
    enc = run(
        ["sops", "encrypt", "--age", pub.group(1), "--in-place", canonical],
        cwd=tmp_path,
    )
    assert enc.returncode == 0, "Cannot build isolated encrypted fixture"
    env = os.environ.copy()
    env.update(
        USERPROFILE=str(home),
        LOCALAPPDATA=str(local),
        SOPS_AGE_KEY_FILE=str(keyfile),
    )
    try:
        yield env, canonical, local
    finally:
        # Synthetic age private keys must not survive the test run, either.
        shutil.rmtree(tmp_path)


def host(env, *params):
    return run(
        ["pwsh", "-NoProfile", "-NonInteractive", "-File", SCRIPT, *params],
        env=env,
        cwd=SCRIPT.parent,
    )


def extract(env, encrypted_file, key):
    proc = run(
        ["sops", "decrypt", "--extract", f'["{key}"]', encrypted_file],
        env=env,
    )
    assert proc.returncode == 0, "Cannot read isolated test key"
    val = proc.stdout.strip()
    try:
        return json.loads(val)
    except json.JSONDecodeError:
        return val.strip('"')


def test_status_only_returns_non_secret_metadata(isolated_sops):
    env, canonical, _ = isolated_sops
    present = host(env, "-Action", "Status", "-Key", "KNOWN_TEST_SECRET")
    assert present.returncode == 0, present.stderr
    assert json.loads(present.stdout) == {
        "key": "KNOWN_TEST_SECRET",
        "provider": "RandomHex",
        "present": True,
    }
    assert "synthetic-before" not in present.stdout + present.stderr
    missing = host(env, "-Action", "Status", "-Key", "ABSENT_TEST_SECRET")
    assert missing.returncode == 0
    assert json.loads(missing.stdout)["present"] is False
    assert extract(env, canonical, "KNOWN_TEST_SECRET") == "synthetic-before"


def test_random_hex_generation_and_rotation_no_plaintext(isolated_sops):
    env, canonical, _ = isolated_sops
    before = canonical.read_bytes()
    first = host(env, "-Action", "Generate", "-Key", "GENERATED_TEST_SECRET")
    assert first.returncode == 0, first.stderr
    assert json.loads(first.stdout)["updated"] is True
    secret = extract(env, canonical, "GENERATED_TEST_SECRET")
    assert re.fullmatch("[0-9a-f]{64}", secret)
    assert secret not in first.stdout + first.stderr
    assert secret.encode() not in canonical.read_bytes()

    denied = host(env, "-Action", "Generate", "-Key", "GENERATED_TEST_SECRET")
    assert denied.returncode != 0
    assert extract(env, canonical, "GENERATED_TEST_SECRET") == secret

    rotated = host(
        env, "-Action", "Generate", "-Key", "GENERATED_TEST_SECRET", "-Rotate"
    )
    assert rotated.returncode == 0, rotated.stderr
    updated = extract(env, canonical, "GENERATED_TEST_SECRET")
    assert updated != secret
    assert re.fullmatch("[0-9a-f]{64}", updated)
    assert updated not in rotated.stdout + rotated.stderr
    assert extract(env, canonical, "KNOWN_TEST_SECRET") == "synthetic-before"
    assert canonical.read_bytes() != before
    assert not list(canonical.parent.glob(".host-sops-*"))


def test_scoped_pat_issue_status_and_rotation(isolated_sops):
    env, canonical, local = isolated_sops
    params = [
        "-Key", "SYNTHETIC_PAT",
        "-Provider", "LomwayPAT",
        "-Label", "sops-test",
        "-AllowedTools", "praxiom_praxiom_status,praxiom_praxiom_execute",
        "-Days", "1",
    ]
    issue = host(env, "-Action", "Generate", *params)
    assert issue.returncode == 0, issue.stderr
    token = extract(env, canonical, "SYNTHETIC_PAT")
    assert token.startswith("lompat_")
    assert token not in issue.stdout + issue.stderr
    status = host(env, "-Action", "Status", "-Key", "SYNTHETIC_PAT",
                  "-Provider", "LomwayPAT")
    assert status.returncode == 0, status.stderr
    payload = json.loads(status.stdout)
    assert payload["active_pat_matches"] == 1
    assert payload["allowed_count"] == 2
    assert payload["praxiom_count"] == 2
    assert payload["execute_allowed"] is True
    assert payload["coding_tools_count"] == 0

    rotate = host(env, "-Action", "Generate", *params, "-Rotate")
    assert rotate.returncode == 0, rotate.stderr
    storefile = local / "Lomway" / "oauth-sidecar" / "runtime" / "pat-tokens.json"
    records = json.loads(storefile.read_text(encoding="utf-8"))["tokens"]
    assert len(records) == 2
    assert len([x for x in records if x.get("revokedAt") is None]) == 1
    assert len([x for x in records if x.get("revokedAt") is not None]) == 1
    updated = extract(env, canonical, "SYNTHETIC_PAT")
    assert updated != token
    assert updated not in rotate.stdout + rotate.stderr
    assert extract(env, canonical, "KNOWN_TEST_SECRET") == "synthetic-before"


def test_rejects_invalid_key_names(isolated_sops):
    env, canonical, _ = isolated_sops
    original = canonical.read_bytes()
    invalid = host(env, "-Action", "Generate", "-Key", "../escape")
    assert invalid.returncode != 0
    assert canonical.read_bytes() == original


def test_forbidden_pat_tools_fail_without_mutation(isolated_sops):
    env, canonical, local = isolated_sops
    original = canonical.read_bytes()
    invalid = host(
        env, "-Action", "Generate", "-Key", "FORBIDDEN_PAT",
        "-Provider", "LomwayPAT", "-Label", "sops-test",
        "-AllowedTools", "praxiom_praxiom_status,proxy/config",
    )
    assert invalid.returncode != 0
    assert canonical.read_bytes() == original
    store = local / "Lomway" / "oauth-sidecar" / "runtime" / "pat-tokens.json"
    assert not store.exists()
    assert "synthetic-before" not in invalid.stdout + invalid.stderr


def test_random_hex_status_does_not_require_decryption(isolated_sops):
    env, canonical, _ = isolated_sops
    env = dict(env)
    env["SOPS_AGE_KEY_FILE"] = str(canonical.parent / "missing-age-test-key.txt")
    status = host(env, "-Action", "Status", "-Key", "KNOWN_TEST_SECRET")
    assert status.returncode == 0, status.stderr
    assert json.loads(status.stdout)["present"] is True
    assert "synthetic-before" not in status.stdout + status.stderr
