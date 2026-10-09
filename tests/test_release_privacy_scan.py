"""Regression cases for safe synthetic email exclusions in the release gate."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCAN_PATH = Path(__file__).resolve().parents[1] / "scripts" / "release_privacy_scan.py"
spec = importlib.util.spec_from_file_location("release_privacy_scan", SCAN_PATH)
assert spec is not None and spec.loader is not None
privacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(privacy)


def email(local: str, domain: str) -> str:
    return local + "@" + domain


class PrivacyScanTest(unittest.TestCase):
    def assert_allowed(self, value: str) -> None:
        self.assertEqual(privacy.scan_text(value, "fixture"), [])

    def assert_detected(self, value: str) -> None:
        self.assertEqual(privacy.scan_text(value, "fixture"), ["fixture:1: email address"])

    def test_reserved_domains_are_only_synthetic_email_matches(self) -> None:
        for domain in (
            "client.example.test",
            "example.com",
            "example.net",
            "example.org",
            "sub.example.com",
            "demo.test",
            "demo.example",
            "demo.invalid",
            "demo.localhost",
            "EXAMPLE.TEST",
        ):
            with self.subTest(domain=domain):
                self.assert_allowed(email("sample", domain))
        self.assert_allowed("https://user:pass" + "@" + "client.example.test/callback")

    def test_real_domains_are_never_excluded(self) -> None:
        for domain in (
            "company.com",
            "real-company.com",
            "notexample.com",
            "example.com.attacker.net",
            "sample.test.evil.org",
        ):
            with self.subTest(domain=domain):
                self.assert_detected(email("person", domain))

    def test_url_userinfo_with_real_domain_stays_detectable(self) -> None:
        address = email("secret", "real-company.com")
        self.assert_detected("https://user:" + address + "/resource")
        self.assert_detected("//" + email("person", "company.com"))

    def test_one_pinned_history_fixture_does_not_disable_url_userinfo_scan(self) -> None:
        historical = "https://" + email("user", "workbridge-mac.example-tailnet.ts.net") + "/mcp"
        source = "history:src/config/validate.rs"
        self.assertEqual(
            privacy.scan_text(historical, source, history_oid=privacy.LEGACY_FIXTURE_BLOB),
            [],
        )
        # It is still detected anywhere except the precise historical blob.
        self.assert_detected(historical)
        self.assertEqual(
            privacy.scan_text(historical, source, history_oid="0" * 40),
            [source + ":1: email address"],
        )
        self.assertEqual(
            privacy.scan_text(historical, "history:src/other.rs", history_oid=privacy.LEGACY_FIXTURE_BLOB),
            ["history:src/other.rs:1: email address"],
        )

    def test_non_email_secret_rules_remain_active(self) -> None:
        self.assertEqual(
            privacy.scan_text("-----BEGIN " + "PRIVATE KEY-----", "fixture"),
            ["fixture:1: private key"],
        )


if __name__ == "__main__":
    unittest.main()
