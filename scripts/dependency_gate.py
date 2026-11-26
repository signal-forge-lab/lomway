#!/usr/bin/env python3
"""Review exact Cargo.lock packages for license metadata and OSV findings."""

from __future__ import annotations

import json
import subprocess
import tomllib
import urllib.request
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OSV_URL = "https://api.osv.dev/v1/querybatch"
EXCEPTIONS_PATH = ROOT / "scripts" / "dependency_gate_exceptions.toml"
EXCEPTION_FIELDS = ("package", "version", "advisory", "reason", "reviewed")


def cargo_metadata() -> dict:
    raw = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT,
    )
    return json.loads(raw)


def lock_packages() -> list[dict]:
    with (ROOT / "Cargo.lock").open("rb") as handle:
        return tomllib.load(handle)["package"]


def load_exceptions() -> dict[tuple[str, str, str], dict]:
    if not EXCEPTIONS_PATH.exists():
        return {}
    with EXCEPTIONS_PATH.open("rb") as handle:
        entries = tomllib.load(handle).get("exception", [])
    exceptions: dict[tuple[str, str, str], dict] = {}
    for entry in entries:
        missing = [f for f in EXCEPTION_FIELDS if not str(entry.get(f, "")).strip()]
        key = (entry.get("package", "?"), entry.get("version", "?"), entry.get("advisory", "?"))
        if missing:
            raise ValueError(f"exception {' '.join(key)} is missing {', '.join(missing)}")
        if key in exceptions:
            raise ValueError(f"duplicate exception for {' '.join(key)}")
        exceptions[key] = entry
    return exceptions


def osv_query(packages: list[dict]) -> list[tuple[str, str, list[str]]]:
    findings: list[tuple[str, str, list[str]]] = []
    registry = [p for p in packages if str(p.get("source", "")).startswith("registry+")]
    for start in range(0, len(registry), 100):
        chunk = registry[start : start + 100]
        body = {
            "queries": [
                {
                    "package": {"ecosystem": "crates.io", "name": p["name"]},
                    "version": p["version"],
                }
                for p in chunk
            ]
        }
        request = urllib.request.Request(
            OSV_URL,
            data=json.dumps(body).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            results = json.loads(response.read().decode("utf-8"))["results"]
        for package, result in zip(chunk, results, strict=True):
            ids = [vuln["id"] for vuln in result.get("vulns", []) if vuln.get("id")]
            if ids:
                findings.append((package["name"], package["version"], ids))
    return findings


def main() -> int:
    locked = lock_packages()
    metadata = cargo_metadata()["packages"]

    locked_pairs = Counter((p["name"], p["version"]) for p in locked)
    metadata_pairs = Counter((p["name"], p["version"]) for p in metadata)
    if locked_pairs != metadata_pairs:
        print("dependency gate: FAIL (Cargo.lock and cargo metadata package sets differ)")
        return 1

    missing = sorted((p["name"], p["version"]) for p in metadata if not p.get("license"))
    if missing:
        print("dependency gate: FAIL (missing license metadata)")
        for name, version in missing:
            print(f"- {name} {version}")
        return 1

    try:
        exceptions = load_exceptions()
    except (ValueError, tomllib.TOMLDecodeError) as error:
        print(f"dependency gate: FAIL (invalid {EXCEPTIONS_PATH.name}: {error})")
        return 1

    license_expressions = sorted({p["license"] for p in metadata})
    findings = []
    accepted = []
    for name, version, ids in osv_query(locked):
        unresolved = [i for i in ids if (name, version, i) not in exceptions]
        accepted.extend((name, version, i) for i in ids if i not in unresolved)
        if unresolved:
            findings.append((name, version, unresolved))
    for key in sorted(set(exceptions) - set(accepted)):
        print(f"dependency gate: note: stale exception no longer matches: {' '.join(key)}")
    if findings:
        print("dependency gate: FAIL (unresolved OSV records; release blocks conservatively)")
        for name, version, ids in findings:
            print(f"- {name} {version}: {', '.join(ids)}")
        return 1

    print(f"dependency gate: PASS ({len(locked)} exact locked packages)")
    print(f"license expressions reviewed: {len(license_expressions)}; missing: 0")
    print("OSV unresolved vulnerability records: 0")
    print(f"OSV records accepted by reviewed exception: {len(accepted)}")
    for name, version, advisory in sorted(accepted):
        print(f"- {name} {version}: {advisory} ({exceptions[(name, version, advisory)]['reviewed']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
