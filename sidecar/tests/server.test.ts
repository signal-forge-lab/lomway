import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, randomBytes } from "node:crypto";
import { mkdtemp, rm, symlink } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import type { SidecarConfig } from "../src/config.js";
import { issuePat, patStorePath, revokePat } from "../src/pat.js";
import { createLomwaySidecar, isMainEntrypoint } from "../src/server.js";

const ISSUER = "https://issuer.example.test";
const RESOURCE = "https://mcp.example.test/mcp";
const OWNER_PASSWORD = "synthetic-owner-password";
const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });

test("recognizes the main entrypoint through a symlink or junction alias", async () => {
  const runtimeDir = await mkdtemp(join(tmpdir(), "lomway-entrypoint-"));
  const serverPath = fileURLToPath(new URL("../src/server.js", import.meta.url));
  const aliasDir = join(runtimeDir, "alias");

  try {
    await symlink(dirname(serverPath), aliasDir, process.platform === "win32" ? "junction" : "dir");
    assert.equal(isMainEntrypoint(join(aliasDir, basename(serverPath))), true);
  } finally {
    await rm(runtimeDir, { recursive: true, force: true });
  }
});

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
    assert.equal(activeBody.oauth_unrestricted_legacy, false);
    assert.deepEqual(activeBody.allowed_tools, ["lomway_search_tools", "lomway_describe_tool"]);

    await accessToken.destroy();
    const revoked = await introspect(baseUrl, tokenValue);
    assert.equal(revoked.status, 200);
    assert.deepEqual(await revoked.json(), { active: false });
  });
});

test("public DCR enforces a server-side global registration burst limit", async () => {
  await withSidecar(async (baseUrl) => {
    const metadata = {
      token_endpoint_auth_method: "none",
      grant_types: ["authorization_code"],
      response_types: ["code"],
      redirect_uris: ["http://127.0.0.1/callback"],
    };
    for (let index = 0; index < 20; index++) {
      const result = await fetch(baseUrl + "/reg", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(metadata),
      });
      assert.equal(result.status, 201, "registration " + index);
    }
    const blocked = await fetch(baseUrl + "/reg", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(metadata),
    });
    assert.equal(blocked.status, 429);
    assert.equal(blocked.headers.get("retry-after"), "60");
  });
});

test("introspection accepts scoped PATs and immediately observes revocation", async () => {
  await withSidecar(async (baseUrl, config) => {
    const { token, record } = await issuePat(patStorePath(config.runtimeDir), {
      label: "synthetic-agent",
      days: 30,
      audience: config.resourceUrl,
      allowedTools: ["lomway_search_tools"],
    });
    const active = await introspect(baseUrl, token);
    assert.equal(active.status, 200);
    const result = await active.json() as Record<string, unknown>;
    assert.equal(result.active, true);
    assert.equal(result.pat, true);
    assert.equal(result.scope, "devspace");
    assert.equal(result.aud, config.resourceUrl);
    assert.deepEqual(result.allowed_tools, ["lomway_search_tools"]);
    assert.equal(active.headers.get("cache-control"), "no-store");

    assert.equal(await revokePat(patStorePath(config.runtimeDir), record.id), true);
    const revoked = await introspect(baseUrl, token);
    assert.deepEqual(await revoked.json(), { active: false });
  });
});

test("public proxy requests cannot reach internal bearer introspection", async () => {
  await withSidecar(async (baseUrl) => {
    const blocked = await fetch(`${baseUrl}/oauth/introspect`, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        "cf-connecting-ip": "203.0.113.11",
      },
      body: new URLSearchParams({ token: "synthetic-token" }),
    });
    assert.equal(blocked.status, 404);
    const internal = await introspect(baseUrl, "synthetic-token");
    assert.deepEqual(await internal.json(), { active: false });
  });
});

test("limits oversized and repeated failed owner-password submissions", async () => {
  await withSidecar(async (baseUrl) => {
    // Unknown UIDs must be rejected BEFORE the expensive password check.
    const unknown = await submitPassword(baseUrl, "unknown-uid", OWNER_PASSWORD);
    assert.equal(unknown.status, 400);

    const first = await newLoginInteraction(baseUrl);
    const oversized = await fetch(first.url, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        cookie: first.cookies,
      },
      body: `password=${"x".repeat(5000)}`,
    });
    assert.equal(oversized.status, 413);

    for (let attempt = 0; attempt < 5; attempt += 1) {
      const rejected = await fetch(first.url, {
        method: "POST",
        headers: {
          "content-type": "application/x-www-form-urlencoded",
          cookie: first.cookies,
        },
        body: new URLSearchParams({ password: "wrong-password" }),
        redirect: "manual",
      });
      assert.equal(rejected.status, 401);
    }
    // A fresh, valid interaction does not reset the owner-wide failure budget.
    const second = await newLoginInteraction(baseUrl);
    const limited = await fetch(second.url, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        cookie: second.cookies,
      },
      body: new URLSearchParams({ password: "wrong-password" }),
    });
    assert.equal(limited.status, 429);
  });
});

async function newLoginInteraction(
  baseUrl: string,
  opts: { challenge?: string; refresh?: boolean } = {},
): Promise<{ url: string; cookies: string; clientId: string }> {
  const registration = await fetch(`${baseUrl}/reg`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      token_endpoint_auth_method: "none",
      grant_types: opts.refresh ? ["authorization_code", "refresh_token"] : ["authorization_code"],
      response_types: ["code"],
      redirect_uris: ["http://127.0.0.1/callback"],
    }),
  });
  assert.equal(registration.status, 201);
  const client = await registration.json() as { client_id: string };
  const url = new URL(`${baseUrl}/auth`);
  url.searchParams.set("client_id", client.client_id);
  url.searchParams.set("redirect_uri", "http://127.0.0.1/callback");
  url.searchParams.set("response_type", "code");
  url.searchParams.set("scope", "openid devspace");
  url.searchParams.set("resource", RESOURCE);
  url.searchParams.set("code_challenge", opts.challenge ?? "A".repeat(43));
  url.searchParams.set("code_challenge_method", "S256");
  const auth = await fetch(url, { redirect: "manual" });
  assert.equal(auth.status, 303);
  return { url: localUrl(baseUrl, requiredLocation(auth)), cookies: responseCookies(auth), clientId: client.client_id };
}

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

test("requires explicit informed consent after owner login", async () => {
  await withSidecar(async (baseUrl) => {
    const first = await newLoginInteraction(baseUrl);
    const signedIn = await fetch(first.url, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        cookie: first.cookies,
      },
      body: new URLSearchParams({ password: OWNER_PASSWORD }),
      redirect: "manual",
    });
    assert.equal(signedIn.status, 303);
    const jar = new Map<string, string>();
    function mergeCookies(header: string): void {
      for (const part of header.split("; ").filter(Boolean)) {
        const name = part.split("=", 1)[0];
        if (name) jar.set(name, part);
      }
    }
    mergeCookies(first.cookies);
    mergeCookies(responseCookies(signedIn));
    const currentCookie = (): string => [...jar.values()].join("; ");
    let next = localUrl(baseUrl, requiredLocation(signedIn));
    let consentUrl: string | undefined;
    let consentBody = "";
    let consentCsp = "";
    for (let hop = 0; hop < 8; hop++) {
      const response = await fetch(next, { headers: { cookie: currentCookie() }, redirect: "manual" });
      mergeCookies(responseCookies(response));
      if (response.status === 200 && next.includes("/interaction/")) {
        consentUrl = next;
        consentBody = await response.text();
        consentCsp = response.headers.get("content-security-policy") ?? "";
        break;
      }
      assert.equal(response.status, 303, "expected authorization redirect at " + new URL(next).pathname);
      next = localUrl(baseUrl, requiredLocation(response));
    }
    assert.ok(consentUrl, "OAuth must pause at a separate consent screen");
    assert.match(consentBody, /Authorize this client/);
    assert.match(consentBody, /Client ID:/);
    assert.match(consentBody, /Redirect URI:/);
    assert.match(consentBody, /Requested scopes:/);
    assert.match(consentBody, /Approve access/);
    // OAuth form POST may redirect through the provider to the client's
    // registered redirect origin. Chromium enforces CSP on that chain.
    assert.match(consentCsp, /form-action 'self' http:\/\/127\.0\.0\.1/);
    assert.match(consentCsp, /frame-ancestors 'none'/);
    const notApproved = await fetch(consentUrl, {
      method: "POST",
      headers: { cookie: currentCookie(), "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ decision: "invalid" }),
      redirect: "manual",
    });
    assert.equal(notApproved.status, 403);
    const approved = await fetch(consentUrl, {
      method: "POST",
      headers: { cookie: currentCookie(), "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ decision: "approve" }),
      redirect: "manual",
    });
    assert.equal(approved.status, 303);
  });
});

test("authorization code exchange, refresh rotation and replay revocation work end to end", async () => {
  await withSidecar(async (baseUrl) => {
    const verifier = "x".repeat(64);
    const challenge = createHash("sha256").update(verifier).digest("base64url");
    const first = await newLoginInteraction(baseUrl, { challenge, refresh: true });
    const cookies = new Map<string, string>();
    const merge = (header: string): void => {
      for (const part of header.split("; ").filter(Boolean)) {
        const name = part.split("=", 1)[0];
        if (name) cookies.set(name, part);
      }
    };
    const cookie = (): string => [...cookies.values()].join("; ");
    merge(first.cookies);
    const signedIn = await fetch(first.url, {
      method: "POST",
      headers: { cookie: cookie(), "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ password: OWNER_PASSWORD }),
      redirect: "manual",
    });
    assert.equal(signedIn.status, 303);
    merge(responseCookies(signedIn));
    let next = localUrl(baseUrl, requiredLocation(signedIn));
    let consentUrl: string | undefined;
    for (let hop = 0; hop < 8; hop++) {
      const response = await fetch(next, { headers: { cookie: cookie() }, redirect: "manual" });
      merge(responseCookies(response));
      if (response.status === 200 && next.includes("/interaction/")) {
        consentUrl = next;
        break;
      }
      assert.equal(response.status, 303);
      next = localUrl(baseUrl, requiredLocation(response));
    }
    assert.ok(consentUrl);
    const approved = await fetch(consentUrl, {
      method: "POST",
      headers: { cookie: cookie(), "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ decision: "approve" }),
      redirect: "manual",
    });
    assert.equal(approved.status, 303);
    merge(responseCookies(approved));
    next = localUrl(baseUrl, requiredLocation(approved));
    let finalRedirect: URL | undefined;
    for (let hop = 0; hop < 8; hop++) {
      const response = await fetch(next, { headers: { cookie: cookie() }, redirect: "manual" });
      merge(responseCookies(response));
      assert.equal(response.status, 303, "expected authorization completion redirect");
      const destination = new URL(requiredLocation(response), ISSUER);
      if (destination.pathname === "/callback") {
        finalRedirect = destination;
        break;
      }
      next = localUrl(baseUrl, destination.href);
    }
    assert.ok(finalRedirect, "authorization code must reach registered redirect URI");
    const code = finalRedirect.searchParams.get("code");
    assert.ok(code, "authorization code must be present");
    const exchange = await fetch(`${baseUrl}/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "authorization_code",
        code,
        redirect_uri: "http://127.0.0.1/callback",
        client_id: first.clientId,
        code_verifier: verifier,
        resource: RESOURCE,
      }),
    });
    assert.equal(exchange.status, 200, "code exchange must succeed");
    const issued = await exchange.json() as { refresh_token?: string; access_token?: string };
    assert.ok(issued.refresh_token && issued.access_token);
    const rotate = async (refreshToken: string): Promise<Response> =>
      fetch(`${baseUrl}/token`, {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "refresh_token",
          refresh_token: refreshToken,
          client_id: first.clientId,
          resource: RESOURCE,
        }),
      });
    const rotated = await rotate(issued.refresh_token);
    assert.equal(rotated.status, 200, "refresh rotation must succeed");
    const replacement = await rotated.json() as { refresh_token?: string };
    assert.ok(replacement.refresh_token && replacement.refresh_token !== issued.refresh_token);
    const replay = await rotate(issued.refresh_token);
    assert.equal(replay.status, 400, "reuse must fail");
    const revoked = await rotate(replacement.refresh_token);
    assert.equal(revoked.status, 400, "replay must revoke the whole grant family");
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
