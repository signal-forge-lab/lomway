"""Read-only consumer contract with disposable synthetic SOPS documents."""

import json
import re
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


def test_malformed_store_is_sanitized(isolated_sops):
    env, canonical, _ = isolated_sops
    canonical.write_text('{"unterminated": ', encoding="utf-8")
    before = canonical.read_bytes()
    for action in ("Status", "Plan"):
        response = host(env, "-Action", action, "-Consumer", "MacVncLogin")
        assert response.returncode != 0
        assert "HOST_SECRET_CONSUMER_PREFLIGHT_FAILED" in response.stderr
        assert str(canonical) not in response.stdout + response.stderr
        assert "unterminated" not in response.stdout + response.stderr
    assert canonical.read_bytes() == before


@pytest.mark.parametrize(
    "extra",
    [
        ("-Command", "whoami"),
        ("-Executable", "C:\\Windows\\System32\\cmd.exe"),
        ("-Key", "OTHER_SECRET"),
        ("-Host", "example.invalid"),
        ("-EnvVar", "PATH"),
        ("-Unknown", "x"),
    ],
)
def test_unknown_parameters_are_rejected(isolated_sops, extra):
    env, canonical, _ = isolated_sops
    before = canonical.read_bytes()
    response = host(env, "-Action", "Plan", "-Consumer", "MacVncLogin", *extra)
    assert response.returncode != 0
    assert str(canonical) not in response.stdout + response.stderr
    assert canonical.read_bytes() == before


# Test-only guard harness: the production consumer table and the complete
# viewer/connection guard block, including the $ready decision, are copied from
# the script and run with Get-Process and Get-NetTCPConnection replaced by local
# fakes, so no real viewer or network is observed. Production code gains no
# injection seam from this.
#
# Functions called through a pipeline run in their own child scope, so a fake
# function's output would be lost. A scriptblock variable is cloned into that
# child scope and keeps its output instead.
PROFILE_BLOCK = re.compile(r"(?s)\$profile = @\{.*?\}\[\$Consumer\]")
GUARD_BLOCK = re.compile(r"(?s)    \$viewer = @\(\).*?(?=\n    \[pscustomobject\]@\{)")
# Both guard decisions, each on its own source line. They are pinned so a
# mutated production guard cannot pass these tests unnoticed.
VIEWER_DECISION = "if ($viewer.Count -eq 1) {"
CONNECTION_DECISION = "$ready = $connection.Count -eq 1"
VIEWER_QUERY = "Get-Process -Name vncviewer"
CONNECTION_QUERY = "Get-NetTCPConnection -OwningProcess"

PATH_OK = "C:\\Program Files\\TigerVNC\\vncviewer.exe"
TITLE_OK = "VNC authentication"
ADDRESS_OK = "100.100.47.83"
PORT_OK = "5900"


def viewer(**overrides):
    """One complete viewer object; keyword overrides change single fields."""
    fields = {"Id": "1", "Path": "'" + PATH_OK + "'", "MainWindowTitle": "'" + TITLE_OK + "'"}
    fields.update(overrides)
    return tuple(fields.items())


def connection(**overrides):
    """One complete connection object; keyword overrides change single fields."""
    fields = {"RemoteAddress": "'" + ADDRESS_OK + "'", "RemotePort": PORT_OK}
    fields.update(overrides)
    return tuple(fields.items())


def rows(objects):
    """Emit the given data objects into a fake cmdlet body."""
    lines = []
    for index, members in enumerate(objects):
        lines.append("$o{0} = New-Object -TypeName psobject".format(index))
        for prop, value in members:
            lines.append(
                "$o{0} | Add-Member -NotePropertyName {1} -NotePropertyValue {2}".format(
                    index, prop, value
                )
            )
        lines.append("$o{0}".format(index))
    return lines


def setup(processes, connections):
    """Fake data sources; an empty set emits nothing, like a real empty lookup."""
    parts = ["Set-StrictMode -Version Latest"]
    for name, objects in (
        ("fakeProcesses", processes),
        ("fakeConnections", connections),
    ):
        body = rows(objects) or ["return"]
        parts.append(
            "${0} = {{ param($Name, $OwningProcess, $State); {1} }}".format(
                name, "; ".join(body)
            )
        )
    return "\n".join(parts) + "\n"


def run_guard(env, processes, connections, *, guard=None):
    """Run the real guard block against fake processes and connections only.

    The two candidate lookups are rebound to fakes; both `.Count -eq 1`
    decisions and the `$ready` assignment stay production code. A caller may
    pass a deliberately mutated `guard` to prove the tests can detect it.
    """
    body = SCRIPT.read_text(encoding="utf-8")
    profile = PROFILE_BLOCK.search(body)
    if guard is None:
        found = GUARD_BLOCK.search(body)
        assert found, "production layout changed; update the test harness"
        guard = found.group(0)
    stage = guard
    for original, replacement in (
        (VIEWER_QUERY, "& $fakeProcesses -Name vncviewer"),
        (CONNECTION_QUERY, "& $fakeConnections -OwningProcess"),
    ):
        assert original in stage, "production guard changed; update the test harness"
        stage = stage.replace(original, replacement)
    runner = (
        setup(processes, connections)
        + "& { $Action = 'Plan'; $Consumer = 'MacVncLogin'; $present = $true\n"
        + profile.group(0)
        + "\n"
        + stage
        + "\n"
        + 'Write-Output ("GUARD_READY=$ready VIEWER=" + @($viewer).Count)\n'
        + "}\n"
    )
    path = Path(env["USERPROFILE"]) / "guard-harness.ps1"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(runner, encoding="utf-8")
    try:
        return run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-File", path],
            env=env,
            cwd=ROOT,
        )
    finally:
        path.unlink()


def real_guard():
    body = SCRIPT.read_text(encoding="utf-8")
    return GUARD_BLOCK.search(body).group(0)


VIEWER_CASES = [
    # No candidate: not ready, and the connection stage is never reached.
    ([], "GUARD_READY=False VIEWER=0"),
    # Exactly one exact match, with one matching connection.
    ([viewer()], "GUARD_READY=True VIEWER=1"),
    # Two exact candidates: ambiguous, not ready, connection stage skipped.
    ([viewer(), viewer()], "GUARD_READY=False VIEWER=2"),
    # One object with the wrong executable path.
    ([viewer(Path="'C:\\\\tmp\\\\vncviewer.exe'")], "GUARD_READY=False VIEWER=0"),
    # One object with another window title.
    ([viewer(MainWindowTitle="'Other window'")], "GUARD_READY=False VIEWER=0"),
]

CONNECTION_CASES = [
    # No established connection for the viewer's process.
    ([], "GUARD_READY=False VIEWER=1"),
    # Exactly one connection to the fixed endpoint.
    ([connection()], "GUARD_READY=True VIEWER=1"),
    # Two established connections: ambiguous, not ready.
    ([connection(), connection()], "GUARD_READY=False VIEWER=1"),
    # Same address, wrong port: no exact match, so not ready.
    ([connection(RemotePort="5901")], "GUARD_READY=False VIEWER=1"),
    # Neighbouring address, right port: no exact match, so not ready.
    ([connection(RemoteAddress="'100.100.47.84'")], "GUARD_READY=False VIEWER=1"),
]


@pytest.mark.parametrize("processes,expected", VIEWER_CASES)
def test_viewer_guard_sets_ready_only_for_one_exact_viewer(
    isolated_sops, processes, expected
):
    env, canonical, _ = isolated_sops
    install_synthetic_mac_key(env, canonical)
    before = canonical.read_bytes()
    response = run_guard(env, processes, [connection()])
    assert response.returncode == 0, response.stderr
    assert response.stdout.strip() == expected
    assert SYNTHETIC_VALUE not in response.stdout + response.stderr
    assert canonical.read_bytes() == before


@pytest.mark.parametrize("connections,expected", CONNECTION_CASES)
def test_connection_guard_sets_ready_only_for_one_exact_endpoint(
    isolated_sops, connections, expected
):
    env, canonical, _ = isolated_sops
    install_synthetic_mac_key(env, canonical)
    before = canonical.read_bytes()
    # One exact viewer is assumed so only the connection decision varies.
    response = run_guard(env, [viewer()], connections)
    assert response.returncode == 0, response.stderr
    assert response.stdout.strip() == expected
    assert SYNTHETIC_VALUE not in response.stdout + response.stderr
    assert canonical.read_bytes() == before


def test_harness_detects_mutated_guard_decisions(isolated_sops):
    """A weakened production decision must change the harness result.

    This protects the tests above: if the harness silently ignored the guard
    decisions, these planted defects would pass and the check would fail.
    """
    env, canonical, _ = isolated_sops
    install_synthetic_mac_key(env, canonical)
    guard = real_guard()
    assert VIEWER_DECISION in guard
    assert CONNECTION_DECISION in guard
    # Reference result: two exact viewers must stay not ready.
    real = run_guard(env, [viewer(), viewer()], [connection()], guard=guard)
    assert real.stdout.strip() == "GUARD_READY=False VIEWER=2", real.stderr
    weakened_viewer = run_guard(
        env,
        [viewer(), viewer()],
        [connection()],
        guard=guard.replace(VIEWER_DECISION, "if ($viewer.Count -ge 1) {"),
    )
    assert weakened_viewer.stdout.strip() == "GUARD_READY=True VIEWER=2", (
        weakened_viewer.stderr
    )
    weakened_ready = run_guard(
        env,
        [viewer()],
        [connection(), connection()],
        guard=guard.replace(
            CONNECTION_DECISION, "$ready = $connection.Count -ge 1"
        ),
    )
    assert weakened_ready.stdout.strip() == "GUARD_READY=True VIEWER=1", (
        weakened_ready.stderr
    )


def test_no_broad_secret_dispatch_capabilities():
    code = SCRIPT.read_text(encoding="utf-8")
    assert "ValidateSet('MacVncLogin')" in code
    assert "ValidateSet('Status', 'Plan')" in code
    assert "sops --decrypt" not in code
    assert "MAC_MINI_ADMIN_PASSWORD" in code
    assert "100.100.47.83" in code
    assert "Invoke-Expression" not in code
    assert "Start-Process" not in code
