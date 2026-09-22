import assert from "node:assert/strict";
import { generateKeyPairSync, randomBytes } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { SidecarConfig } from "../src/config.js";
import { createLomwaySidecar } from "../src/server.js";

const ISSUER = "https://issuer.example.test";
const RESOURCE = "https://mcp.example.test/mcp";
const OWNER_PASSWORD = "synthetic-owner-password";
const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });

async function withSidecar(
  run: (
    baseUrl: string,
    config: SidecarConfig,
    provider: ReturnType<typeof createLomwaySidecar>["provider"],
  ) => Promise<void>,
): Promise<void> {
  const runtimeDir = await mkdtemp(join(tmpdir(), "lomway-oauth-server-"));
  const port = await availablePort();
  const config: SidecarConfig = {
    host: "127.0.0.1",
    port,
    issuer: ISSUER,
    resourceUrl: RESOURCE,
    ownerCredential: OWNER_PASSWORD,
    jwks: { keys: [privateKey.export({ format: "jwk" })] },
    cookieKeys: [randomBytes(32).toString("base64url"), randomBytes(32).toString("base64url")],
    runtimeDir,
    statePath: join(runtimeDir, "oauth-state.json"),
  };
  const sidecar = createLomwaySidecar(config);

  try {
    await sidecar.start();
    await run(`http://${config.host}:${config.port}`, config, sidecar.provider);
  } finally {
    await sidecar.close();
    await rm(runtimeDir, { recursive: true, force: true });
  }
}

test("starts health and provider discovery without any Workbridge dependency", async () => {
  await withSidecar(async (baseUrl) => {
    const health = await fetch(`${baseUrl}/healthz`);
    assert.equal(health.status, 200);

    const discovery = await fetch(`${baseUrl}/.well-known/openid-configuration`);
    assert.equal(discovery.status, 200);
    const metadata = (await discovery.json()) as Record<string, unknown>;
    assert.equal(metadata.issuer, ISSUER);
    assert.equal(metadata.authorization_endpoint, `${ISSUER}/auth`);
    assert.equal(metadata.token_endpoint, `${ISSUER}/token`);
  });
});

test("serves loopback RFC 7662 introspection without client authentication", async () => {
  await withSidecar(async (baseUrl, config, provider) => {
    const inactive = await introspect(baseUrl, "missing-token");
    assert.equal(inactive.status, 200);
    assert.deepEqual(await inactive.json(), { active: false });

    const registration = await fetch(`${baseUrl}/reg`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        token_endpoint_auth_method: "none",
        grant_types: ["authorization_code"],
        response_types: ["code"],
        redirect_uris: ["http://127.0.0.1/callback"],
      }),
    });
    assert.equal(registration.status, 201);
    const { client_id: clientId } = (await registration.json()) as { client_id: string };
    const client = await provider.Client.find(clientId);
    assert.ok(client);

    const grant = new provider.Grant({ accountId: "owner", clientId });
    await grant.save();
    const accessToken = new provider.AccessToken({
      client,
      accountId: "owner",
      aud: config.resourceUrl,
      scope: "devspace",
      grantId: grant.jti,
      gty: "authorization_code",
    });
    const tokenValue = await accessToken.save();

    const active = await introspect(baseUrl, tokenValue);
    assert.equal(active.status, 200);
    const activeBody = (await active.json()) as Record<string, unknown>;
    assert.equal(activeBody.active, true);
    assert.equal(activeBody.aud, config.resourceUrl);
    assert.equal(activeBody.client_id, clientId);
    assert.equal(activeBody.scope, "devspace");

    await accessToken.destroy();
    const revoked = await introspect(baseUrl, tokenValue);
    assert.equal(revoked.status, 200);
    assert.deepEqual(await revoked.json(), { active: false });
  });
});

test("limits oversized and repeated failed owner-password submissions", async () => {
  await withSidecar(async (baseUrl) => {
    const oversized = await fetch(`${baseUrl}/interaction/test-uid`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: `password=${"x".repeat(5000)}`,
    });
    assert.equal(oversized.status, 413);

    for (let attempt = 0; attempt < 5; attempt += 1) {
      const rejected = await submitPassword(baseUrl, "rate-limit-uid", "wrong-password");
      assert.equal(rejected.status, 401);
    }
    const limited = await submitPassword(baseUrl, "rate-limit-uid", "wrong-password");
    assert.equal(limited.status, 429);
  });
});

test("authenticates the owner and resumes the provider-managed interaction", async () => {
  await withSidecar(async (baseUrl) => {
    const registration = await fetch(`${baseUrl}/reg`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        token_endpoint_auth_method: "none",
        grant_types: ["authorization_code"],
        response_types: ["code"],
        redirect_uris: ["http://127.0.0.1/callback"],
      }),
    });
    assert.equal(registration.status, 201);
    const client = (await registration.json()) as { client_id: string };

    const authorization = new URL(`${baseUrl}/auth`);
    authorization.searchParams.set("client_id", client.client_id);
    authorization.searchParams.set("redirect_uri", "http://127.0.0.1/callback");
    authorization.searchParams.set("response_type", "code");
    authorization.searchParams.set("scope", "openid devspace");
    authorization.searchParams.set("resource", RESOURCE);
    authorization.searchParams.set("code_challenge", "A".repeat(43));
    authorization.searchParams.set("code_challenge_method", "S256");

    const authResponse = await fetch(authorization, { redirect: "manual" });
    assert.equal(authResponse.status, 303);
    const interactionUrl = localUrl(baseUrl, requiredLocation(authResponse));
    const cookies = responseCookies(authResponse);

    const form = await fetch(interactionUrl, { headers: { cookie: cookies } });
    assert.equal(form.status, 200);
    const html = await form.text();
    assert.match(html, /type="password"/);
    assert.doesNotMatch(html, new RegExp(OWNER_PASSWORD));

    const accepted = await fetch(interactionUrl, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        cookie: cookies,
      },
      body: new URLSearchParams({ password: OWNER_PASSWORD }),
      redirect: "manual",
    });
    assert.equal(accepted.status, 303);
    assert.match(requiredLocation(accepted), /^https:\/\/issuer\.example\.test\/auth\//);
  });
});

async function submitPassword(
  baseUrl: string,
  uid: string,
  password: string,
): Promise<Response> {
  return fetch(`${baseUrl}/interaction/${uid}`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ password }),
    redirect: "manual",
  });
}

async function introspect(baseUrl: string, token: string): Promise<Response> {
  return fetch(`${baseUrl}/oauth/introspect`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ token }),
  });
}

function requiredLocation(response: Response): string {
  const location = response.headers.get("location");
  assert.ok(location);
  return location;
}

function localUrl(baseUrl: string, publicUrl: string): string {
  const url = new URL(publicUrl, ISSUER);
  return `${baseUrl}${url.pathname}${url.search}`;
}

function responseCookies(response: Response): string {
  return response.headers
    .getSetCookie()
    .map((cookie) => cookie.split(";", 1)[0])
    .join("; ");
}

async function availablePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return address.port;
}
