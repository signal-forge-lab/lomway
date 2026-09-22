import assert from "node:assert/strict";
import test from "node:test";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { generateKeyPairSync, randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

import { loadConfig } from "../src/config.js";

const { privateKey: testPrivateKey, publicKey: testPublicKey } = generateKeyPairSync("rsa", {
  modulusLength: 2048,
});
const TEST_ONLY_JWKS_JSON = JSON.stringify({
  keys: [testPrivateKey.export({ format: "jwk" })],
});

function validEnvironment(): NodeJS.ProcessEnv {
  const runtimeDir = join(tmpdir(), `lomway-oauth-sidecar-${randomUUID()}`);
  return {
    LOMWAY_OAUTH_BIND_HOST: "127.0.0.1",
    LOMWAY_OAUTH_PORT: "7677",
    LOMWAY_OAUTH_ISSUER: "https://issuer.example.test",
    LOMWAY_OAUTH_RESOURCE_URL: "https://mcp.example.test/mcp",
    LOMWAY_OAUTH_OWNER_CREDENTIAL: "synthetic-owner-credential",
    LOMWAY_OAUTH_JWKS_JSON: TEST_ONLY_JWKS_JSON,
    LOMWAY_OAUTH_COOKIE_KEYS_JSON: JSON.stringify([randomUUID(), randomUUID()]),
    LOMWAY_OAUTH_RUNTIME_DIR: runtimeDir,
    LOMWAY_OAUTH_STATE_PATH: join(runtimeDir, "oauth-state.json"),
  };
}

test("loads the complete environment-only sidecar configuration", () => {
  const environment = validEnvironment();

  const config = loadConfig(environment);

  assert.equal(config.host, "127.0.0.1");
  assert.equal(config.port, 7677);
  assert.equal(config.issuer, environment.LOMWAY_OAUTH_ISSUER);
  assert.equal(config.resourceUrl, environment.LOMWAY_OAUTH_RESOURCE_URL);
  assert.equal(config.ownerCredential, environment.LOMWAY_OAUTH_OWNER_CREDENTIAL);
  assert.deepEqual(config.jwks, JSON.parse(environment.LOMWAY_OAUTH_JWKS_JSON!));
  assert.deepEqual(config.cookieKeys, JSON.parse(environment.LOMWAY_OAUTH_COOKIE_KEYS_JSON!));
  assert.equal(config.runtimeDir, environment.LOMWAY_OAUTH_RUNTIME_DIR);
  assert.equal(config.statePath, environment.LOMWAY_OAUTH_STATE_PATH);
});

test("rejects missing, malformed, empty, or public-only signing JWKS", () => {
  const invalidValues = [
    undefined,
    "not-json",
    "null",
    "[]",
    "{}",
    JSON.stringify({ keys: [] }),
    JSON.stringify({ keys: [testPublicKey.export({ format: "jwk" })] }),
    JSON.stringify({ keys: [{ kty: "", d: "test-only-private-material" }] }),
    JSON.stringify({ keys: [{ kty: "RSA", d: "" }] }),
  ];

  for (const value of invalidValues) {
    const environment = validEnvironment();
    if (value === undefined) {
      delete environment.LOMWAY_OAUTH_JWKS_JSON;
    } else {
      environment.LOMWAY_OAUTH_JWKS_JSON = value;
    }

    assert.throws(() => loadConfig(environment), /LOMWAY_OAUTH_JWKS_JSON/);
  }
});

test("rejects missing, malformed, or weak cookie signing key arrays", () => {
  const strongKey = "a".repeat(32);
  const invalidValues = [
    undefined,
    "not-json",
    "{}",
    "[]",
    JSON.stringify([strongKey]),
    JSON.stringify([strongKey, ""]),
    JSON.stringify([strongKey, "b".repeat(31)]),
    JSON.stringify([strongKey, strongKey]),
  ];

  for (const value of invalidValues) {
    const environment = validEnvironment();
    if (value === undefined) {
      delete environment.LOMWAY_OAUTH_COOKIE_KEYS_JSON;
    } else {
      environment.LOMWAY_OAUTH_COOKIE_KEYS_JSON = value;
    }

    assert.throws(() => loadConfig(environment), /LOMWAY_OAUTH_COOKIE_KEYS_JSON/);
  }
});

test("does not echo rejected signing material in configuration errors", () => {
  const secretMarker = `test-only-secret-${randomUUID()}`;
  const invalidValues = [
    [
      "LOMWAY_OAUTH_JWKS_JSON",
      JSON.stringify({ keys: [{ kty: "", d: secretMarker }] }),
    ],
    ["LOMWAY_OAUTH_COOKIE_KEYS_JSON", JSON.stringify([secretMarker])],
  ] as const;

  for (const [key, value] of invalidValues) {
    const environment = validEnvironment();
    environment[key] = value;

    assert.throws(() => loadConfig(environment), (error: unknown) => {
      assert.ok(error instanceof Error);
      assert.match(error.message, new RegExp(key));
      assert.doesNotMatch(error.message, new RegExp(secretMarker));
      return true;
    });
  }
});

test("rejects a missing owner credential instead of applying a default", () => {
  const environment = validEnvironment();
  delete environment.LOMWAY_OAUTH_OWNER_CREDENTIAL;

  assert.throws(() => loadConfig(environment), /LOMWAY_OAUTH_OWNER_CREDENTIAL/);
});

test("rejects non-loopback hosts and invalid TCP ports", () => {
  const invalidValues = [
    ["LOMWAY_OAUTH_BIND_HOST", "localhost"],
    ["LOMWAY_OAUTH_BIND_HOST", "0.0.0.0"],
    ["LOMWAY_OAUTH_PORT", "0"],
    ["LOMWAY_OAUTH_PORT", "65536"],
    ["LOMWAY_OAUTH_PORT", "not-a-port"],
  ] as const;

  for (const [key, value] of invalidValues) {
    const environment = validEnvironment();
    environment[key] = value;
    assert.throws(() => loadConfig(environment), new RegExp(key));
  }
});

test("rejects non-public or non-HTTPS issuer and resource URLs", () => {
  const invalidValues = [
    ["LOMWAY_OAUTH_ISSUER", "http://issuer.example.test"],
    ["LOMWAY_OAUTH_ISSUER", "https://127.0.0.1/issuer"],
    ["LOMWAY_OAUTH_RESOURCE_URL", "/mcp"],
    ["LOMWAY_OAUTH_RESOURCE_URL", "https://localhost/mcp"],
    ["LOMWAY_OAUTH_RESOURCE_URL", "https://user:pass@mcp.example.test/mcp"],
  ] as const;

  for (const [key, value] of invalidValues) {
    const environment = validEnvironment();
    environment[key] = value;
    assert.throws(() => loadConfig(environment), new RegExp(key));
  }
});

test("requires state to be an absolute path below an external runtime directory", () => {
  const invalidValues = [
    { LOMWAY_OAUTH_RUNTIME_DIR: "runtime" },
    { LOMWAY_OAUTH_STATE_PATH: "oauth-state.json" },
    { LOMWAY_OAUTH_STATE_PATH: join(tmpdir(), "outside-runtime.json") },
  ] as const;

  for (const overrides of invalidValues) {
    const environment = { ...validEnvironment(), ...overrides };
    assert.throws(() => loadConfig(environment), /LOMWAY_OAUTH_(RUNTIME_DIR|STATE_PATH)/);
  }
});

test("rejects runtime state that would land inside the sidecar source or test tree", () => {
  for (const segment of ["src", "tests"] as const) {
    const sourceDirectory = join(packageRoot(), segment);
    const environment = {
      ...validEnvironment(),
      LOMWAY_OAUTH_RUNTIME_DIR: sourceDirectory,
      LOMWAY_OAUTH_STATE_PATH: join(sourceDirectory, "oauth-state.json"),
    };

    assert.throws(() => loadConfig(environment), /LOMWAY_OAUTH_(RUNTIME_DIR|STATE_PATH)/);
  }
});

function packageRoot(): string {
  let directory = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    if (existsSync(join(directory, "package.json"))) return directory;
    const parent = dirname(directory);
    if (parent === directory) return directory;
    directory = parent;
  }
}
