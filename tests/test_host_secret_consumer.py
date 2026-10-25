"""Read-only consumer contract with disposable synthetic SOPS documents."""

import json
import re
import subprocess
from pathlib import Path

import pytest

from test_host_sops_manager import isolated_sops, run

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "Invoke-HostSecretConsumer.ps1"
SYNTHETIC_VALUE = "synthetic-not-a-real-mac-password-5410"


def host(env, *args):
    return run(
        ["pwsh", "-NoProfile", "-NonInteractive", "-File", SCRIPT, *args],
        env=env,
        cwd=ROOT,
    )


def install_synthetic_mac_key(env, canonical):
    age = Path(env["SOPS_AGE_KEY_FILE"])
    pub = run(["age-keygen", "-y", age])
    assert pub.returncode == 0
    assert re.fullmatch(r"age1[a-z0-9]+", pub.stdout.strip())
    canonical.write_text(
        json.dumps({"MAC_MINI_ADMIN_PASSWORD": SYNTHETIC_VALUE}),
        encoding="utf-8",
    )
    sealed = run([
        "sops", "encrypt", "--age", pub.stdout.strip(), "--in-place", canonical,
    ])
    assert sealed.returncode == 0
    assert SYNTHETIC_VALUE.encode() not in canonical.read_bytes()


def test_missing_key_is_visible_only_as_metadata(isolated_sops):
    env, canonical, _ = isolated_sops
    before = canonical.read_bytes()
    response = host(env, "-Action", "Status", "-Consumer", "MacVncLogin")
    assert response.returncode == 0, response.stderr
    parsed = json.loads(response.stdout)
    assert parsed["present"] is False
    assert parsed["secret_exposed"] is False
    assert parsed["credential_delivery"] == "not-implemented"
    assert canonical.read_bytes() == before


def test_synthetic_secret_never_decrypted_for_status_or_plan(isolated_sops):
    env, canonical, _ = isolated_sops
    install_synthetic_mac_key(env, canonical)
    before = canonical.read_bytes()
    # Deliberately break the AGE decryption path. A preflight must still work.
    env = dict(env)
    env["SOPS_AGE_KEY_FILE"] = str(canonical.parent / "no-such-age-key.txt")
    for action in ("Status", "Plan"):
        response = host(env, "-Action", action, "-Consumer", "MacVncLogin")
        assert response.returncode == 0, response.stderr
        payload = json.loads(response.stdout)
        assert payload["present"] is True
        assert payload["secret_exposed"] is False
        assert payload["credential_delivery"] == "not-implemented"
        assert SYNTHETIC_VALUE not in response.stdout + response.stderr
        if action == "Plan":
            assert payload["destination"] == "100.100.47.83:5900"
            assert isinstance(payload["viewer_ready"], bool)
    assert canonical.read_bytes() == before


@pytest.mark.parametrize(
    "action,consumer",
    [
        ("Invoke", "MacVncLogin"),
        ("Status", "ShellCommand"),
        ("Status", "MacVncLogin;Get-ChildItem"),
        ("ReadPlaintext", "MacVncLogin"),
    ],
)
def test_no_generic_secret_or_command_execution(isolated_sops, action, consumer):
    env, canonical, _ = isolated_sops
    before = canonical.read_bytes()
    response = host(env, "-Action", action, "-Consumer", consumer)
    assert response.returncode != 0
    assert canonical.read_bytes() == before
    assert "synthetic-before" not in response.stdout + response.stderr


def test_store_error_is_sanitized(isolated_sops):
    env, canonical, _ = isolated_sops
    canonical.unlink()
    response = host(env, "-Action", "Plan", "-Consumer", "MacVncLogin")
    assert response.returncode != 0
    assert "HOST_SECRET_CONSUMER_PREFLIGHT_FAILED" in response.stderr
    assert str(canonical) not in response.stdout + response.stderr


def test_no_broad_secret_dispatch_capabilities():
    code = SCRIPT.read_text(encoding="utf-8")
    assert "ValidateSet('MacVncLogin')" in code
    assert "ValidateSet('Status', 'Plan')" in code
    assert "sops --decrypt" not in code
    assert "MAC_MINI_ADMIN_PASSWORD" in code
    assert "100.100.47.83" in code
    assert "Invoke-Expression" not in code
    assert "Start-Process" not in code
