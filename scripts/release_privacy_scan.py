#!/usr/bin/env python3
"""Fail the public-release candidate on machine/private credential material.

The gate covers both the candidate worktree/index and every blob reachable
from HEAD. The history pass is intentionally limited to HEAD so local tooling
refs (for example review snapshots) are never treated as public push content.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKIP_TOP = {".git", ".runtime", ".tmp_vendor", ".tmp_tower_mcp", "target", "target-deploy"}
MAX_TEXT_BYTES = 2 * 1024 * 1024
HISTORY_FORBIDDEN_TOP = {".runtime", ".tmp_vendor", ".tmp_tower_mcp", "target", "target-deploy"}
HISTORY_FORBIDDEN_PATHS = {"config/proxy.local.toml"}
HISTORY_FORBIDDEN_PARTS = {"__pycache__"}
HISTORY_FORBIDDEN_SUFFIXES = {".pyc", ".pyo"}

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


def scan_text(text: str, source: str) -> list[str]:
    findings: list[str] = []
    for label, pattern in CHECKS:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append(f"{source}:{line}: {label}")
    return findings


def history_objects(ref: str = "HEAD") -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    """Return all blob paths and unique blobs reachable from *ref*.

    Every path is retained for generated/private-path validation. Blob content
    is de-duplicated separately so identical file contents are scanned once.
    """

    raw = subprocess.check_output(["git", "rev-list", "--objects", ref], cwd=ROOT)
    raw_entries: list[tuple[str, str]] = []
    for line in raw.decode("utf-8", "surrogateescape").splitlines():
        if " " not in line:
            continue
        oid, path = line.split(" ", 1)
        raw_entries.append((oid, path.replace("\\", "/")))

    if not raw_entries:
        return [], []

    unique_oids = list(dict.fromkeys(oid for oid, _ in raw_entries))
    type_input = "".join(f"{oid}\n" for oid in unique_oids)
    type_output = subprocess.check_output(
        ["git", "cat-file", "--batch-check=%(objectname) %(objecttype)"],
        cwd=ROOT,
        input=type_input,
        text=True,
    )
    object_types = {
        oid: obj_type
        for oid, obj_type in (line.split(" ", 1) for line in type_output.splitlines())
    }

    paths: list[tuple[str, str]] = []
    unique_blobs: list[tuple[str, str]] = []
    seen_blobs: set[str] = set()
    for oid, normalized in raw_entries:
        if object_types.get(oid) != "blob":
            continue
        paths.append((oid, normalized))
        if oid in seen_blobs:
            continue
        seen_blobs.add(oid)
        unique_blobs.append((oid, normalized))
    return paths, unique_blobs


def forbidden_history_path(rel: str) -> bool:
    path = Path(rel)
    parts = path.parts
    return (
        bool(parts and parts[0] in HISTORY_FORBIDDEN_TOP)
        or rel in HISTORY_FORBIDDEN_PATHS
        or any(part in HISTORY_FORBIDDEN_PARTS for part in parts)
        or path.suffix.lower() in HISTORY_FORBIDDEN_SUFFIXES
    )


def history_blob_contents(
    blobs: list[tuple[str, str]],
) -> list[tuple[str, str, bytes]]:
    """Read blob contents through one persistent `git cat-file --batch` process."""

    if not blobs:
        return []

    proc = subprocess.Popen(
        ["git", "cat-file", "--batch"],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert proc.stdin is not None
    assert proc.stdout is not None

    results: list[tuple[str, str, bytes]] = []
    try:
        for oid, rel in blobs:
            proc.stdin.write(f"{oid}\n".encode("ascii"))
            proc.stdin.flush()
            header = proc.stdout.readline().decode("ascii", "replace").strip()
            fields = header.split()
            if len(fields) != 3 or fields[1] != "blob":
                raise RuntimeError(f"unexpected git cat-file header for {oid}: {header}")
            size = int(fields[2])
            data = proc.stdout.read(size)
            terminator = proc.stdout.read(1)
            if terminator != b"\n":
                raise RuntimeError(f"missing git cat-file terminator for {oid}")
            results.append((oid, rel, data))
    finally:
        if proc.stdin:
            proc.stdin.close()
        proc.wait()
    if proc.returncode != 0:
        stderr = proc.stderr.read().decode("utf-8", "replace") if proc.stderr else ""
        raise RuntimeError(f"git cat-file --batch failed: {stderr.strip()}")
    return results


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
        findings.extend(scan_text(text, rel))

    history_paths, unique_history_blobs = history_objects()
    for _, rel in history_paths:
        if forbidden_history_path(rel):
            findings.append(f"history:{rel}: forbidden public-history path")

    history_scanned = 0
    for _, rel, data in history_blob_contents(unique_history_blobs):
        if len(data) > MAX_TEXT_BYTES:
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            continue
        history_scanned += 1
        findings.extend(scan_text(text, f"history:{rel}"))

    if findings:
        print("release privacy scan: FAIL")
        for finding in findings:
            print(f"- {finding}")
        return 1

    print(
        "release privacy scan: PASS "
        f"({scanned} UTF-8 candidate files; {history_scanned} UTF-8 HEAD-history blobs)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
