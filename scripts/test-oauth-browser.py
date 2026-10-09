#!/usr/bin/env python3
"""Exercise the real built OAuth sidecar in isolated Chromium, no live secrets.

Requires installed Python Playwright/Chromium. Starts a disposable sidecar
with synthetic credentials, a temporary OAuth store, and a local callback.
No production OAuth registration, login, grant or browser profile is touched.
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
import subprocess
import sys
import threading
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.sync_api import TimeoutError as BrowserTimeout, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "sidecar" / "tests" / "oauth-browser-fixture.mjs"
PASSWORD = "synthetic-browser-test-password"
RESOURCE = "https://mcp.example.test/mcp"


def post_json(url: str, payload: dict) -> dict:
    request = urllib.request.Request(
        url, json.dumps(payload).encode("utf-8"),
        {"content-type": "application/json"}, method="POST",
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def post_form(url: str, payload: dict) -> dict:
    request = urllib.request.Request(
        url, urllib.parse.urlencode(payload).encode("utf-8"),
        {"content-type": "application/x-www-form-urlencoded"}, method="POST",
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


class Callback(BaseHTTPRequestHandler):
    code: str | None = None

    def do_GET(self) -> None:
        location = urllib.parse.urlsplit(self.path)
        if location.path != "/callback":
            self.send_error(404)
            return
        self.server.authorization_code = urllib.parse.parse_qs(location.query).get("code", [None])[0]
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"Authorization callback received")

    def log_message(self, format: str, *args: object) -> None:
        return


def main() -> None:
    sidecar = subprocess.Popen(
        ["node.exe" if sys.platform == "win32" else "node", str(FIXTURE)],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
    )
    callback = ThreadingHTTPServer(("127.0.0.1", 0), Callback)
    callback.authorization_code = None
    callback_thread = threading.Thread(target=callback.serve_forever, daemon=True)
    callback_thread.start()

    try:
        assert sidecar.stdout is not None
        line = sidecar.stdout.readline().strip()
        if not line.startswith("BROWSER_FIXTURE_READY:"):
            raise AssertionError("Disposable OAuth fixture did not start")
        base = "http://127.0.0.1:" + line.split(":", 1)[1]
        redirect_uri = f"http://127.0.0.1:{callback.server_port}/callback"
        name = "Synthetic Browser <NoScript>"

        client = post_json(base + "/reg", {
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "redirect_uris": [redirect_uri],
            "client_name": name,
        })
        verifier = "z" * 64
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode("ascii")).digest()
        ).decode("ascii").rstrip("=")
        query = urllib.parse.urlencode({
            "client_id": client["client_id"],
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "scope": "openid devspace",
            "resource": RESOURCE,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        })

        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            try:
                context = browser.new_context()
                page = context.new_page()
                login_response = page.goto(base + "/auth?" + query, wait_until="domcontentloaded")
                assert login_response is not None
                assert login_response.status == 200
                assert page.get_by_role("heading", name="Lomway owner authentication").count() == 1
                assert login_response.headers.get("x-frame-options") == "DENY"
                assert login_response.headers.get("cache-control") == "no-store"
                print("CHROMIUM_OAUTH_LOGIN_RENDER_PASS")

                page.locator('input[type="password"]').fill(PASSWORD)
                page.get_by_role("button", name="Sign in").click()
                page.get_by_role("heading", name="Authorize this client?").wait_for(timeout=10000)
                assert page.get_by_text(name, exact=False).count() >= 1
                assert page.get_by_text(client["client_id"], exact=False).count() >= 1
                assert page.get_by_text(redirect_uri, exact=False).count() >= 1
                assert page.locator("script").count() == 0
                print("CHROMIUM_OAUTH_INFORMED_CONSENT_RENDER_PASS")

                observed = []
                page.on("response", lambda response: observed.append((
                    response.request.method,
                    response.status,
                    urllib.parse.urlsplit(response.url).path,
                )))
                page.get_by_role("button", name="Approve access").click()
                try:
                    page.wait_for_url(
                        re.compile(r"^http://127\.0\.0\.1:\d+/callback\?"),
                        timeout=10000,
                    )
                except BrowserTimeout:
                    current = urllib.parse.urlsplit(page.url)
                    parameters = urllib.parse.parse_qs(current.query)
                    print(
                        "BROWSER_OAUTH_REDIRECT_DEBUG"
                        + " path=" + current.path
                        + " query_keys=" + ",".join(sorted(parameters))
                        + " callback=" + str(callback.authorization_code is not None)
                        + " responses=" + repr(observed[-9:])
                        + " body_prefix=" + repr(page.locator("body").inner_text()[:200]),
                    )
                    raise
                assert callback.authorization_code is not None
                tokens = post_form(base + "/token", {
                    "grant_type": "authorization_code",
                    "client_id": client["client_id"],
                    "code": callback.authorization_code,
                    "redirect_uri": redirect_uri,
                    "code_verifier": verifier,
                    "resource": RESOURCE,
                })
                assert tokens.get("access_token")
                result = post_form(base + "/oauth/introspect", {
                    "token": tokens["access_token"],
                })
                assert result.get("active") is True
                print("CHROMIUM_OAUTH_CODE_EXCHANGE_AND_INTROSPECTION_PASS")
            finally:
                browser.close()
    finally:
        callback.shutdown()
        callback.server_close()
        if sidecar.poll() is None:
            sidecar.terminate()
        try:
            sidecar.wait(timeout=8)
        except subprocess.TimeoutExpired:
            sidecar.kill()
            sidecar.wait(timeout=5)


if __name__ == "__main__":
    main()
