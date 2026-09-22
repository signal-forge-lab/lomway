import assert from "node:assert/strict";
import test from "node:test";

import {
  assertSafeCimdUrl,
  constantTimeEqual,
  hashOwnerPassword,
  verifyOwnerPassword,
} from "../src/security.js";

test("CIMD retrieval accepts HTTPS without credentials and rejects SSRF targets", () => {
  assert.equal(
    assertSafeCimdUrl("https://client.example.test/metadata.json").href,
    "https://client.example.test/metadata.json",
  );
  for (const value of [
    "http://client.example.test/metadata.json",
    "https://user:pass@client.example.test/metadata.json",
    "https://127.0.0.1/metadata.json",
    "https://10.0.0.4/metadata.json",
    "https://[::1]/metadata.json",
    "https://client.local/metadata.json",
  ]) {
    assert.throws(() => assertSafeCimdUrl(value), /unsafe CIMD URL/);
  }
});

test("owner password hashing does not retain or compare plaintext state", async () => {
  const record = await hashOwnerPassword("synthetic-owner-password");
  assert.equal(await verifyOwnerPassword("synthetic-owner-password", record), true);
  assert.equal(await verifyOwnerPassword("wrong-password", record), false);
  assert.equal("password" in record, false);
});

test("constant-time comparison rejects different lengths", () => {
  assert.equal(constantTimeEqual("same", "same"), true);
  assert.equal(constantTimeEqual("same", "different"), false);
});
