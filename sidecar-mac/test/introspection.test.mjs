import assert from "node:assert/strict";
import { test } from "node:test";

import { ownerAuthorizedIntrospection } from "../src/introspection.mjs";

test("verified owner-authorized OAuth token carries an explicit legacy authorization marker", () => {
  const response = ownerAuthorizedIntrospection({
    clientId: "test-public-client",
    scopes: ["devspace"],
    expiresAt: 1791860000,
    resource: new URL("https://mcp-mac.maiteneru.com/mcp"),
    token: "secret-not-for-output",
  });
  assert.deepEqual(response, {
    active: true,
    client_id: "test-public-client",
    scope: "devspace",
    exp: 1791860000,
    aud: "https://mcp-mac.maiteneru.com/mcp",
    oauth_unrestricted_legacy: true,
  });
  assert.equal(JSON.stringify(response).includes("secret-not-for-output"), false);
});

test("introspection marker does not fabricate an audience", () => {
  const response = ownerAuthorizedIntrospection({
    clientId: "test-public-client",
    scopes: ["devspace"],
    expiresAt: 1791860000,
  });
  assert.equal(Object.hasOwn(response, "aud"), false);
});
