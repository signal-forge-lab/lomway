"""Synthetic-only Windows pipe protocol exercise; not a production trust test."""

import json
import os
from pathlib import Path
import subprocess
import uuid

import pytest

from test_host_sops_manager import isolated_sops

ROOT = Path(__file__).resolve().parents[1]
BROKER = ROOT / "scripts" / "Test-HostSopsPipeBroker.ps1"
CLIENT = ROOT / "scripts" / "Test-HostSopsPipeClient.ps1"


def run(*args, env=None, timeout=14):
    return subprocess.run([str(x) for x in args], capture_output=True, text=True,
                          errors="replace", timeout=timeout, env=env, check=False)


@pytest.mark.usefixtures("isolated_sops")
def test_synthetic_pipe_allows_metadata_only_and_denies_mutation(isolated_sops):
    env, canonical, _ = isolated_sops
    env = dict(env, HOST_SOPS_SYNTHETIC_TEST="1")
    sid = run("pwsh", "-NoProfile", "-Command",
              "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value")
    assert sid.returncode == 0
    name = "iw-sops-test-" + uuid.uuid4().hex
    broker = subprocess.Popen(
        ["pwsh", "-NoProfile", "-NonInteractive", "-File", str(BROKER),
         "-PipeName", name, "-ClientSid", sid.stdout.strip(),
         "-MaxRequests", "3", "-SyntheticOnly"],
        env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    requests = [
        {"version": 1, "action": "Status", "provider": "RandomHex",
         "key": "KNOWN_TEST_SECRET"},
        {"version": 1, "action": "Status", "provider": "RandomHex",
         "key": "UNKNOWN_TEST_SECRET"},
        {"version": 1, "action": "Generate", "provider": "RandomHex",
         "key": "GENERATED_TEST_SECRET"},
    ]
    responses = []
    before = canonical.read_bytes()
    try:
        for request in requests:
            result = run(
                "pwsh", "-NoProfile", "-NonInteractive", "-File", CLIENT,
                "-PipeName", name, "-RequestJson", json.dumps(request), env=env,
                timeout=15
            )
            if result.returncode != 0:
                if broker.poll() is not None:
                    _, server_err = broker.communicate(timeout=3)
                    pytest.fail(
                        "Synthetic broker exited before client: "
                        + server_err.decode("utf-8", "replace")[-1300:]
                    )
                pytest.fail("Synthetic broker/client connect failed: " + result.stderr[-250:])
            responses.append(json.loads(result.stdout.strip()))
        out, err = broker.communicate(timeout=25)
        assert broker.returncode == 0, err.decode("utf-8", "replace")
        assert responses == [
            {"ok": True, "present": True},
            {"ok": True, "present": False},
            {"ok": False, "code": "DENIED"},
        ]
        assert canonical.read_bytes() == before
        assert "synthetic-before" not in (out + err).decode("utf-8", "replace")
    finally:
        if broker.poll() is None:
            broker.terminate()
            broker.wait(timeout=8)


def test_synthetic_pipe_rejects_normal_mode():
    result = run(
        "pwsh", "-NoProfile", "-NonInteractive", "-File", BROKER,
        "-PipeName", "iw-sops-test-" + uuid.uuid4().hex,
        "-ClientSid", "S-1-5-18",
        timeout=10,
    )
    assert result.returncode != 0
    assert "Synthetic-only broker" in result.stderr or "Synthetic-only broker" in result.stdout


def test_synthetic_pipe_refuses_real_windows_userprofile(tmp_path):
    dummy = tmp_path / "dummy-age-key.txt"
    dummy.write_text("not a production identity", encoding="utf-8")
    env = dict(os.environ, HOST_SOPS_SYNTHETIC_TEST="1",
               SOPS_AGE_KEY_FILE=str(dummy))
    sid = run("pwsh", "-NoProfile", "-Command",
              "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value")
    assert sid.returncode == 0
    result = run(
        "pwsh", "-NoProfile", "-NonInteractive", "-File", BROKER,
        "-PipeName", "iw-sops-test-" + uuid.uuid4().hex,
        "-ClientSid", sid.stdout.strip(),
        "-SyntheticOnly", env=env
    )
    assert result.returncode != 0
    assert "real Windows profile" in result.stdout + result.stderr
