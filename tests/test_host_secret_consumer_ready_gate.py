"""Exercise the real read-only Plan decision, not only its filtered candidates.

The PowerShell cmdlets that inspect GUI processes/TCP connections are replaced
by synthetic functions in the invoking PowerShell process. No actual viewer,
network connection or real SOPS credential is used or inspected.
"""

import json
import re
from pathlib import Path

import pytest

from test_host_sops_manager import isolated_sops, run


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "Invoke-HostSecretConsumer.ps1"
SYNTHETIC_PASSWORD = "synthetic-vnc-test-value-not-a-password"


def _install_encrypted_test_key(env, store):
    public = run(["age-keygen", "-y", env["SOPS_AGE_KEY_FILE"]])
    assert public.returncode == 0
    assert re.fullmatch(r"age1[a-z0-9]+", public.stdout.strip())
    store.write_text(
        json.dumps({"MAC_MINI_ADMIN_PASSWORD": SYNTHETIC_PASSWORD}),
        encoding="utf-8",
    )
    sealed = run(
        ["sops", "encrypt", "--age", public.stdout.strip(), "--in-place", store]
    )
    assert sealed.returncode == 0
    assert SYNTHETIC_PASSWORD.encode() not in store.read_bytes()


def _ps_literal(value):
    """Quote only controlled synthetic test values as PowerShell literals."""
    return "'" + str(value).replace("'", "''") + "'"


def _run_plan_with_fake_cmdlets(env, viewers, connections):
    """Invoke the full production Plan without changing production scripts."""
    def make_viewer(path, title, process_id=42):
        return (
            "[pscustomobject]@{"
            f"Id={process_id};Path={_ps_literal(path)};"
            f"MainWindowTitle={_ps_literal(title)}"
            "}"
        )

    def make_connection(address, port):
        return (
            "[pscustomobject]@{"
            f"RemoteAddress={_ps_literal(address)};RemotePort={port}"
            "}"
        )

    viewer_rows = ";".join(make_viewer(*item) for item in viewers)
    connection_rows = ";".join(make_connection(*item) for item in connections)
    # PowerShell function names are resolved from the script invocation's
    # dynamic parent scope. The fakes only return fixed synthetic objects.
    script = f"""
$ErrorActionPreference = 'Stop'
function Get-Process {{
    [CmdletBinding()]
    param([string]$Name)
    @({viewer_rows})
}}
function Get-NetTCPConnection {{
    [CmdletBinding()]
    param([int]$OwningProcess, [string]$State)
    if ($OwningProcess -ne 42 -or $State -ne 'Established') {{
        throw 'unexpected-test-connection-query'
    }}
    @({connection_rows})
}}
& {_ps_literal(SCRIPT)} -Action Plan -Consumer MacVncLogin
"""
    harness = Path(env["USERPROFILE"]) / "plan-ready-gate-test.ps1"
    harness.write_text(script, encoding="utf-8")
    try:
        return run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-File", harness],
            env=env,
            cwd=ROOT,
        )
    finally:
        harness.unlink()


GOOD_VIEWER = ("C:\\Program Files\\TigerVNC\\vncviewer.exe", "VNC authentication")
GOOD_CONNECTION = ("100.100.47.83", 5900)


@pytest.mark.parametrize(
    "viewers,connections,expected_ready",
    [
        ([GOOD_VIEWER], [GOOD_CONNECTION], True),
        ([], [GOOD_CONNECTION], False),
        ([GOOD_VIEWER, GOOD_VIEWER], [GOOD_CONNECTION], False),
        ([("C:\\Other\\vncviewer.exe", GOOD_VIEWER[1])], [GOOD_CONNECTION], False),
        ([(GOOD_VIEWER[0], "Other window")], [GOOD_CONNECTION], False),
        ([GOOD_VIEWER], [], False),
        ([GOOD_VIEWER], [GOOD_CONNECTION, GOOD_CONNECTION], False),
        ([GOOD_VIEWER], [("100.100.47.83", 5901)], False),
        ([GOOD_VIEWER], [("100.100.47.84", 5900)], False),
    ],
)
def test_plan_ready_requires_exactly_one_matching_viewer_and_connection(
    isolated_sops, viewers, connections, expected_ready
):
    env, canonical, _ = isolated_sops
    _install_encrypted_test_key(env, canonical)
    before = canonical.read_bytes()
    outcome = _run_plan_with_fake_cmdlets(env, viewers, connections)
    assert outcome.returncode == 0, outcome.stderr
    result = json.loads(outcome.stdout)
    assert result["viewer_ready"] is expected_ready
    assert result["secret_exposed"] is False
    assert result["credential_delivery"] == "not-implemented"
    assert SYNTHETIC_PASSWORD not in outcome.stdout + outcome.stderr
    assert canonical.read_bytes() == before
