import assert from "node:assert/strict";
import test from "node:test";

import { errors } from "oidc-provider";

import {
  assertAllowedResource,
  cimdAllowFetch,
  cimdPreflight,
  defaultResource,
  validateCimdClientMetadata,
  validateDynamicClientMetadata,
} from "../src/policy.js";

const RESOURCE = "https://mcp.example.test/mcp";

test("allows exactly the configured resource with an audience-bound server definition", () => {
  const resourceServer = assertAllowedResource(RESOURCE, RESOURCE);

  assert.equal(resourceServer.audience, RESOURCE);
  assert.ok(resourceServer.scope.split(" ").includes("devspace"));
  assert.ok(
    typeof resourceServer.accessTokenTTL === "number" &&
      Number.isSafeInteger(resourceServer.accessTokenTTL) &&
      resourceServer.accessTokenTTL > 0,
  );
});

test("rejects a different resource with oidc-provider InvalidTarget", () => {
  assert.throws(
    () => assertAllowedResource("https://other.example.test/mcp", RESOURCE),
    (error: unknown) => error instanceof errors.InvalidTarget,
  );
  assert.throws(
    () => assertAllowedResource(undefined, RESOURCE),
    (error: unknown) => error instanceof errors.InvalidTarget,
  );
  assert.throws(
    () => assertAllowedResource("", RESOURCE),
    (error: unknown) => error instanceof errors.InvalidTarget,
  );
});

test("selects the configured resource as the default resource", () => {
  assert.equal(defaultResource(RESOURCE), RESOURCE);
});

test("accepts public and standards-style loopback dynamic client metadata", () => {
  const accepted = [
    { redirect_uris: ["https://client.example.test/callback"] },
    {
      token_endpoint_auth_method: "none",
      grant_types: ["authorization_code"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback?return=1"],
    },
    {
      token_endpoint_auth_method: "none",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      redirect_uris: [
        "http://127.0.0.1/callback",
        "http://127.0.0.1:4567/callback",
        "http://[::1]/callback",
        "http://[::1]:4567/callback?return=1",
      ],
    },
    {
      redirect_uris: ["http://127.0.0.1/"],
      client_name: "Local Dev Client",
    },
    {
      redirect_uris: ["http://[::1]/path"],
    },
  ];

  for (const metadata of accepted) {
    assert.doesNotThrow(() => validateDynamicClientMetadata(metadata));
  }
});

test("rejects incompatible dynamic client metadata and unsafe redirect URIs", () => {
  const rejected = [
    null,
    undefined,
    "not-an-object",
    [],
    {},
    { redirect_uris: "not-an-array" },
    { redirect_uris: [] },
    { redirect_uris: [""] },
    { redirect_uris: ["not a url"] },
    { redirect_uris: ["/relative/path"] },
    { redirect_uris: ["https://client.example.test/callback#fragment"] },
    { redirect_uris: ["https://user:pass@client.example.test/callback"] },
    { redirect_uris: ["https://localhost/callback"] },
    { redirect_uris: ["https://127.0.0.1/callback"] },
    { redirect_uris: ["https://[::1]/callback"] },
    { redirect_uris: ["https://192.168.1.10/callback"] },
    { redirect_uris: ["https://client.local/callback"] },
    { redirect_uris: ["https://client.internal/callback"] },
    { redirect_uris: ["http://10.0.0.10/callback"] },
    { redirect_uris: ["http://localhost/callback"] },
    { redirect_uris: ["http://localhost:8080/callback"] },
    { redirect_uris: ["http://client.example.test/callback"] },
    { redirect_uris: ["http://127.0.0.2/callback"] },
    { redirect_uris: ["http://[::2]/callback"] },
    { redirect_uris: ["com.example.app:/oauth/callback"] },
    { redirect_uris: ["http://127.0.0.1.evil.example/callback"] },
    { redirect_uris: ["http://127.0.0.1/callback#fragment"] },
    { redirect_uris: ["http://user:pass@127.0.0.1/callback"] },
    {
      token_endpoint_auth_method: "client_secret_basic",
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      token_endpoint_auth_method: "client_secret_post",
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      grant_types: [],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      grant_types: ["client_credentials"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      grant_types: ["refresh_token"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      grant_types: ["authorization_code", "authorization_code"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      grant_types: ["authorization_code", "refresh_token", "client_credentials"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      response_types: [],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      response_types: ["code", "token"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      response_types: ["token"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      response_types: ["id_token"],
      redirect_uris: ["https://client.example.test/callback"],
    },
  ];

  for (const metadata of rejected) {
    assert.throws(
      () => validateDynamicClientMetadata(metadata),
      (error: unknown) => error instanceof errors.InvalidClientMetadata,
    );
  }
});

test("accepts valid HTTPS CIMD identifiers in preflight without network operations", () => {
  const url = cimdPreflight("https://client.example.test/.well-known/oauth-client");
  assert.equal(url.href, "https://client.example.test/.well-known/oauth-client");
  assert.equal(url.hostname, "client.example.test");
  assert.equal(url.protocol, "https:");
});

test("rejects malformed and non-HTTPS CIMD identifiers before any lookup", async () => {
  const rejected = [
    "not a URL",
    "http://client.example.test/.well-known/oauth-client",
    "https://user:pass@client.example.test/.well-known/oauth-client",
    "https://client.example.test/.well-known/oauth-client#fragment",
    "https://client.example.test/.well-known/oauth-client?param=1",
    "https://127.0.0.1/.well-known/oauth-client",
    "https://10.0.0.1/.well-known/oauth-client",
    "https://[::1]/.well-known/oauth-client",
    "https://localhost/.well-known/oauth-client",
    "https://client.local/.well-known/oauth-client",
    "https://client.internal/.well-known/oauth-client",
  ];

  for (const clientId of rejected) {
    assert.throws(() => cimdPreflight(clientId), /unsafe CIMD URL/);
    assert.equal(await cimdAllowFetch(clientId), false);
  }
});

test("accepts current ChatGPT-style CIMD metadata without a singular auth preference", () => {
  assert.doesNotThrow(() =>
    validateCimdClientMetadata({
      client_id: "https://chatgpt.com/oauth/client.json",
      client_name: "ChatGPT",
      redirect_uris: ["https://chatgpt.com/connector_platform_oauth_redirect"],
      token_endpoint_auth_methods_supported: ["none", "private_key_jwt"],
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      token_endpoint_auth_signing_alg: "RS256",
      jwks_uri: "https://chatgpt.com/oauth/jwks.json",
    }),
  );
});

test("accepts private_key_jwt CIMD only with same-origin safe JWKS metadata", () => {
  assert.doesNotThrow(() =>
    validateCimdClientMetadata({
      client_id: "https://client.example.test/oauth/client.json",
      token_endpoint_auth_method: "private_key_jwt",
      token_endpoint_auth_methods_supported: ["none", "private_key_jwt"],
      jwks_uri: "https://client.example.test/oauth/jwks.json",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback"],
    }),
  );

  const rejected = [
    {
      client_id: "https://client.example.test/oauth/client.json",
      token_endpoint_auth_method: "private_key_jwt",
      token_endpoint_auth_methods_supported: ["none", "private_key_jwt"],
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      client_id: "https://client.example.test/oauth/client.json",
      token_endpoint_auth_method: "private_key_jwt",
      token_endpoint_auth_methods_supported: ["none", "private_key_jwt"],
      jwks_uri: "https://keys.example.test/oauth/jwks.json",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback"],
    },
    {
      client_id: "https://client.example.test/oauth/client.json",
      token_endpoint_auth_method: "private_key_jwt",
      token_endpoint_auth_methods_supported: ["none"],
      jwks_uri: "https://client.example.test/oauth/jwks.json",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      redirect_uris: ["https://client.example.test/callback"],
    },
  ];

  for (const metadata of rejected) {
    assert.throws(
      () => validateCimdClientMetadata(metadata),
      (error: unknown) => error instanceof errors.InvalidClientMetadata,
    );
  }
});
