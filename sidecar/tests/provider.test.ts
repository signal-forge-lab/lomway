import assert from "node:assert/strict";
import { generateKeyPairSync, randomBytes } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { errors } from "oidc-provider";

import type { SidecarConfig } from "../src/config.js";
import {
  buildProviderConfiguration,
  createLomwayProvider,
} from "../src/provider.js";

const ISSUER = "https://issuer.example.test";
const RESOURCE = "https://mcp.example.test/mcp";
const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });

async function withConfig(run: (config: SidecarConfig) => Promise<void>): Promise<void> {
  const runtimeDir = await mkdtemp(join(tmpdir(), "lomway-oauth-provider-"));
  const config: SidecarConfig = {
    host: "127.0.0.1",
    port: 7677,
    issuer: ISSUER,
    resourceUrl: RESOURCE,
    ownerCredential: "synthetic-owner-password",
    jwks: { keys: [privateKey.export({ format: "jwk" })] },
    cookieKeys: [randomBytes(32).toString("base64url"), randomBytes(32).toString("base64url")],
    runtimeDir,
    statePath: join(runtimeDir, "oauth-state.json"),
  };

  try {
    await run(config);
  } finally {
    await rm(runtimeDir, { recursive: true, force: true });
  }
}

test("publishes the required OAuth and OpenID discovery metadata offline", async () => {
  await withConfig(async (config) => {
    const provider = createLomwayProvider(config);
    const server = createServer(provider.callback());
    await new Promise<void>((resolve) => server.listen(0, config.host, resolve));

    try {
      const address = server.address();
      assert.ok(address && typeof address === "object");
      const response = await fetch(
        `http://${config.host}:${address.port}/.well-known/openid-configuration`,
        {
          headers: {
            "x-forwarded-host": new URL(config.issuer).host,
            "x-forwarded-proto": "https",
          },
        },
      );
      assert.equal(response.status, 200);

      const metadata = (await response.json()) as Record<string, unknown>;
      assert.equal(metadata.issuer, ISSUER);
      assert.equal(metadata.authorization_endpoint, `${ISSUER}/auth`);
      assert.equal(metadata.token_endpoint, `${ISSUER}/token`);
      assert.equal(metadata.revocation_endpoint, `${ISSUER}/token/revocation`);
      assert.equal(metadata.introspection_endpoint, undefined);
      assert.equal(metadata.registration_endpoint, `${ISSUER}/reg`);
      assert.equal(metadata.authorization_response_iss_parameter_supported, true);
      assert.ok(
        Array.isArray(metadata.code_challenge_methods_supported) &&
          metadata.code_challenge_methods_supported.includes("S256"),
      );
      assert.equal(metadata.client_id_metadata_document_supported, true);
      assert.deepEqual(
        metadata.token_endpoint_auth_methods_supported,
        ["none", "private_key_jwt"],
      );
    } finally {
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    }
  });
});

test("requires PKCE and rejects resources outside the configured audience", async () => {
  await withConfig(async (config) => {
    const providerConfig = buildProviderConfiguration(config);

    assert.equal(providerConfig.pkce?.required?.({} as never, {} as never), true);
    assert.throws(
      () =>
        providerConfig.features?.resourceIndicators?.getResourceServerInfo?.(
          {} as never,
          "https://wrong.example.test/mcp",
          {} as never,
        ),
      (error: unknown) => error instanceof errors.InvalidTarget,
    );
  });
});

test("matches the legacy Workbridge token lifetime contract explicitly", async () => {
  await withConfig(async (config) => {
    const providerConfig = buildProviderConfiguration(config);

    assert.equal(providerConfig.ttl?.Grant, 30 * 24 * 60 * 60);
    assert.equal(providerConfig.ttl?.RefreshToken, 30 * 24 * 60 * 60);
    assert.equal(providerConfig.ttl?.Session, 14 * 24 * 60 * 60);
    assert.equal(providerConfig.expiresWithSession?.({} as never, {} as never), false);
    assert.equal(providerConfig.rotateRefreshToken, true);
  });
});
