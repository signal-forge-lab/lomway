"""Non-secret DSH permission draft must not expand the reviewed tool scope."""

import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DRAFT = ROOT / "docs" / "HOST_SOPS_POLICY_DRAFT_DSH.json"
DSH_SCRIPT = ROOT.parent.parent / "tools" / "dsh-quota-policy" / "Register-DSH-Lomway-PAT.ps1"


def test_draft_is_non_active_and_matches_dsh_tool_scope():
    candidate = json.loads(DRAFT.read_text(encoding="utf-8"))
    pat = candidate["keys"]["DSH_LOMWAY_PAT"]
    assert candidate["version"] == 1
    assert pat["provider"] == "LomwayPAT"
    assert pat["label"] == "dsh-coding"
    assert pat["allowRotate"] is False
    assert pat["maxDays"] == 60
    requested = pat["allowedTools"]
    assert len(requested) == len(set(requested)) == 16
    assert all(not item.startswith("coding_tools_") for item in requested)
    assert "praxiom_praxiom_execute" in requested
    assert list(candidate["keys"]) == ["DSH_LOMWAY_PAT"]

    source = DSH_SCRIPT.read_text(encoding="utf-8")
    listed = re.search(r"\$tools\s*=\s*@\((.*?)\)", source, flags=re.S)
    assert listed is not None
    names = re.findall(r"'([a-z0-9_]+)'", listed.group(1))
    assert set(names) == set(requested)
