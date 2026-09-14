#!/usr/bin/env python3
"""Fail the public-release candidate on machine/private credential material."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKIP_TOP = {".git", ".runtime", ".tmp_vendor", ".tmp_tower_mcp", "target", "target-deploy"}
MAX_TEXT_BYTES = 2 * 1024 * 1024

CHECKS = (
    ("windows user path", re.compile(r"(?i)\b[A-Z]:\\Users\\[^\\\r\n]+")),
    ("unix user path", re.compile(r"/(?:Users|home)/[^/\s]+/")),
    ("private workspace path", re.compile(r"(?i)Documents[\\/]+Intelligence Works")),
    ("email address", re.compile(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", re.I)),
    ("private key", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")),
    ("AWS access key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("GitHub token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b")),
    ("OpenAI-style secret", re.compile(r"\bsk-[A-Za-z0-9_-]{20,}\b")),
    ("Google API key", re.compile(r"\bAIza[0-9A-Za-z_-]{20,}\b")),
    ("Slack token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b")),
    ("live tunnel id", re.compile(r"\btunnel_[0-9a-f]{32}\b", re.I)),
    ("live workspace id", re.compile(r"\bws_[0-9a-f]{8,}\b", re.I)),
)


def candidate_files() -> list[Path]:
    raw = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
    )
    paths = []
    for entry in raw.decode("utf-8", "surrogateescape").split("\0"):
        if not entry:
            continue
        rel = Path(entry)
        if rel.parts and rel.parts[0] in SKIP_TOP:
            continue
        full = (ROOT / rel).resolve()
        if not full.is_file() or full.stat().st_size > MAX_TEXT_BYTES:
            continue
        paths.append(full)
    return paths


def main() -> int:
    findings: list[str] = []
    scanned = 0
    for path in candidate_files():
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        scanned += 1
        rel = path.relative_to(ROOT).as_posix()
        for label, pattern in CHECKS:
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                findings.append(f"{rel}:{line}: {label}")

    if findings:
        print("release privacy scan: FAIL")
        for finding in findings:
            print(f"- {finding}")
        return 1

    print(f"release privacy scan: PASS ({scanned} UTF-8 candidate files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
