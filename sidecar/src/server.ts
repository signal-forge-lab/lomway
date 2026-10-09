import { realpathSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { fileURLToPath, pathToFileURL } from "node:url";

import type Provider from "oidc-provider";

import { loadConfig, type SidecarConfig } from "./config.js";
import { PAT_PREFIX, patStorePath, verifyPat } from "./pat.js";
import { createLomwayProvider } from "./provider.js";
import { hashOwnerPassword, verifyOwnerPassword } from "./security.js";

const MAX_FORM_BYTES = 4096;
const FAILURE_LIMIT = 5;
const FAILURE_WINDOW_MS = 60_000;

interface ConsentDetails {
  missingOIDCScope?: string[];
  missingOIDCClaims?: string[];
  missingResourceScopes?: Record<string, string[]>;
}

export function createLomwaySidecar(config: SidecarConfig): {
  provider: Provider;
  start(): Promise<void>;
  close(): Promise<void>;
} {
  const provider = createLomwayProvider(config);
  const issuer = new URL(config.issuer);
  const ownerPassword = hashOwnerPassword(config.ownerCredential);
  // One owner account: bound all login attempts together, not by attacker-supplied uid.
  let loginFailures: number[] = [];
  const providerCallback = provider.callback();
  const server = createServer((request, response) => {
    void handleRequest(request, response).catch(() => {
      if (!response.headersSent) response.statusCode = 400;
      if (!response.writableEnded) response.end();
    });
  });
  server.requestTimeout = 15_000;
  server.headersTimeout = 10_000;
  let starting: Promise<void> | undefined;
  let closing: Promise<void> | undefined;

  async function handleRequest(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (request.method === "GET" && url.pathname === "/healthz") {
      response.statusCode = 200;
      response.end();
      return;
    }

    if (url.pathname === "/oauth/introspect") {
      // The Gateway uses this endpoint directly over loopback. Cloudflare
      // public requests must never be allowed to inspect bearer credentials.
      if (request.headers["cf-connecting-ip"] !== undefined) {
        response.statusCode = 404;
        response.end();
        return;
      }
      if (request.method !== "POST") {
        response.statusCode = 405;
        response.end();
        return;
      }
      if (request.headers["content-type"]?.split(";", 1)[0]?.trim().toLowerCase() !== "application/x-www-form-urlencoded") {
        response.statusCode = 415;
        response.end();
        return;
      }

      const body = await readForm(request);
      if (body === undefined) {
        response.statusCode = 413;
        response.end();
        return;
      }

      const tokenValue = new URLSearchParams(body).get("token");
      if (tokenValue?.startsWith(PAT_PREFIX)) {
        const pat = await verifyPat(patStorePath(config.runtimeDir), tokenValue, config.resourceUrl);
        response.statusCode = 200;
        response.setHeader("content-type", "application/json; charset=utf-8");
        response.setHeader("cache-control", "no-store");
        response.end(JSON.stringify(pat ? {
          active: true,
          client_id: "pat:" + pat.id,
          iat: pat.createdAt,
          exp: pat.expiresAt,
          iss: provider.issuer,
          aud: pat.audience,
          scope: pat.scope,
          token_type: "Bearer",
          pat: true,
          allowed_tools: pat.allowedTools,
        } : { active: false }));
        return;
      }
      const token = tokenValue ? await provider.AccessToken.find(tokenValue) : undefined;
      if (!token?.isValid) {
        response.statusCode = 200;
        response.setHeader("content-type", "application/json; charset=utf-8");
        response.end(JSON.stringify({ active: false }));
        return;
      }

      if (token.grantId) {
        const grant = await provider.Grant.find(token.grantId, { ignoreExpiration: true });
        if (
          grant === undefined ||
          grant.isExpired ||
          grant.clientId !== token.clientId ||
          grant.accountId !== token.accountId
        ) {
          response.statusCode = 200;
          response.setHeader("content-type", "application/json; charset=utf-8");
          response.end(JSON.stringify({ active: false }));
          return;
        }
      }

      response.statusCode = 200;
      response.setHeader("content-type", "application/json; charset=utf-8");
      response.end(
        JSON.stringify({
          active: true,
          client_id: token.clientId,
          exp: token.exp,
          iat: token.iat,
          iss: provider.issuer,
          aud: token.aud,
          scope: token.scope,
          token_type: token.tokenType,
        }),
      );
      return;
    }

    const match = /^\/interaction\/([^/]+)$/.exec(url.pathname);
    if (!match) {
      request.headers.host = issuer.host;
      request.headers["x-forwarded-host"] = issuer.host;
      request.headers["x-forwarded-proto"] = issuer.protocol.slice(0, -1);
      providerCallback(request, response);
      return;
    }

    const uid = decodeURIComponent(match[1] ?? "");
    // Never perform expensive password verification for an unknown interaction.
    // oidc-provider validates the signed interaction cookie/uid here.
    const interaction = await provider.interactionDetails(request, response);
    if (interaction.uid !== uid) {
      response.statusCode = 400;
      response.end();
      return;
    }
    if (request.method === "GET") {
      response.setHeader("cache-control", "no-store");
      response.setHeader("x-frame-options", "DENY");
      // Chromium also applies form-action to redirects following POST. OAuth
      // authorization intentionally redirects to the registered client's
      // origin, so allow ONLY that origin in addition to our own. The OIDC
      // provider has already validated the redirect_uri for this interaction.
      let approvedRedirectOrigin = "";
      if (typeof interaction.params.redirect_uri === "string") {
        try {
          const redirect = new URL(interaction.params.redirect_uri);
          if (
            (redirect.protocol === "https:" ||
              (redirect.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(redirect.hostname))) &&
            !redirect.username && !redirect.password
          ) {
            approvedRedirectOrigin = " " + redirect.origin;
          }
        } catch {
          // Missing or malformed redirect never weakens the default CSP.
        }
      }
      response.setHeader("content-security-policy",
        "default-src 'none'; form-action 'self'" + approvedRedirectOrigin +
        "; frame-ancestors 'none'; base-uri 'none'");
      response.statusCode = 200;
      response.setHeader("content-type", "text/html; charset=utf-8");
      if (interaction.prompt.name === "login") {
        response.end('<!doctype html><html><body><h1>Lomway owner authentication</h1>' +
          '<form method="post"><label>Password <input name="password" type="password" autocomplete="current-password" required></label>' +
          '<button type="submit">Sign in</button></form></body></html>');
        return;
      }
      if (interaction.prompt.name === "consent") {
        const clientId = interaction.params.client_id;
        const client = typeof clientId === "string" ? await provider.Client.find(clientId) : undefined;
        const metadata = client?.metadata() as Record<string, unknown> | undefined;
        const displayName = typeof metadata?.client_name === "string" ? metadata.client_name : "Unnamed client";
        const redirect = typeof interaction.params.redirect_uri === "string" ? interaction.params.redirect_uri : "";
        const requestedScopes = typeof interaction.params.scope === "string" ? interaction.params.scope : "";
        response.end('<!doctype html><html><body><h1>Authorize this client?</h1>' +
          '<p>Client name (unverified): ' + escapeHtml(displayName) + '</p>' +
          '<p>Client ID: ' + escapeHtml(String(clientId ?? "")) + '</p>' +
          '<p>Redirect URI: ' + escapeHtml(redirect) + '</p>' +
          '<p>Requested scopes: ' + escapeHtml(requestedScopes) + '</p>' +
          '<p>Only approve a client and redirect destination you trust.</p>' +
          '<form method="post"><button name="decision" value="approve" type="submit">Approve access</button></form>' +
          '</body></html>');
        return;
      }
      response.statusCode = 501;
      response.end();
      return;
    }

    if (request.method !== "POST") {
      response.statusCode = 405;
      response.end();
      return;
    }
    if (request.headers["content-type"]?.split(";", 1)[0]?.trim().toLowerCase() !== "application/x-www-form-urlencoded") {
      response.statusCode = 415;
      response.end();
      return;
    }

    const origin = request.headers.origin;
    if (origin && origin !== issuer.origin) {
      response.statusCode = 403;
      response.end();
      return;
    }
    const now = Date.now();
    loginFailures = loginFailures.filter((timestamp) => now - timestamp < FAILURE_WINDOW_MS);
    if (interaction.prompt.name === "login" && loginFailures.length >= FAILURE_LIMIT) {
      response.statusCode = 429;
      response.end();
      return;
    }

    const body = await readForm(request);
    if (body === undefined) {
      response.statusCode = 413;
      response.end();
      return;
    }
    const form = new URLSearchParams(body);
    if (interaction.prompt.name === "login") {
      // Reserve a slot before asynchronous hashing; simultaneous requests
      // must not all pass the rate limit while the worker pool is busy.
      loginFailures.push(now);
      const password = form.get("password") ?? "";
      if (!(await verifyOwnerPassword(password, await ownerPassword))) {
        response.statusCode = 401;
        response.end();
        return;
      }
      loginFailures = [];
      await provider.interactionFinished(
        request,
        response,
        { login: { accountId: "owner" } },
        { mergeWithLastSubmission: false },
      );
      return;
    }
    if (interaction.prompt.name !== "consent") {
      response.statusCode = 501;
      response.end();
      return;
    }
    if (form.get("decision") !== "approve") {
      response.statusCode = 403;
      response.end();
      return;
    }

    const clientId = interaction.params.client_id;
    if (typeof clientId !== "string") {
      response.statusCode = 400;
      response.end();
      return;
    }
    const grant = interaction.grantId
      ? await provider.Grant.find(interaction.grantId)
      : new provider.Grant({
          accountId: interaction.session?.accountId ?? "owner",
          clientId,
        });
    if (!grant) {
      response.statusCode = 400;
      response.end();
      return;
    }

    const details = interaction.prompt.details as ConsentDetails;
    if (details.missingOIDCScope) grant.addOIDCScope(details.missingOIDCScope);
    if (details.missingOIDCClaims) grant.addOIDCClaims(details.missingOIDCClaims);
    if (details.missingResourceScopes) {
      for (const [resource, scopes] of Object.entries(details.missingResourceScopes)) {
        grant.addResourceScope(resource, scopes);
      }
    }
    const grantId = await grant.save();
    await provider.interactionFinished(
      request,
      response,
      { consent: { grantId } },
      { mergeWithLastSubmission: true },
    );
  }

  return {
    provider,
    async start(): Promise<void> {
      await ownerPassword;
      if (closing) await closing;
      if (server.listening) return;
      if (starting) return starting;
      starting ??= new Promise<void>((resolve, reject) => {
        const onError = (error: Error): void => {
          server.off("listening", onListening);
          reject(error);
        };
        const onListening = (): void => {
          server.off("error", onError);
          resolve();
        };
        server.once("error", onError);
        server.once("listening", onListening);
        server.listen(config.port, config.host);
      });
      try {
        await starting;
      } finally {
        starting = undefined;
      }
    },
    async close(): Promise<void> {
      if (closing) return closing;
      if (starting) await starting;
      if (closing) return closing;
      if (!server.listening) return;
      closing ??= new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      });
      try {
        await closing;
      } finally {
        closing = undefined;
      }
    },
  };
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char] ?? char);
}

async function readForm(request: IncomingMessage): Promise<string | undefined> {
  const declaredLength = Number(request.headers["content-length"]);
  if (Number.isFinite(declaredLength) && declaredLength > MAX_FORM_BYTES) {
    request.resume();
    return undefined;
  }

  return new Promise<string | undefined>((resolve, reject) => {
    const chunks: Buffer[] = [];
    let length = 0;
    let resolved = false;

    request.on("data", (chunk: Buffer) => {
      if (resolved) return;
      length += chunk.length;
      if (length > MAX_FORM_BYTES) {
        resolved = true;
        resolve(undefined);
        return;
      }
      chunks.push(chunk);
    });
    request.on("end", () => {
      if (!resolved) resolve(Buffer.concat(chunks, length).toString("utf8"));
    });
    request.on("error", reject);
    request.on("aborted", () => reject(new Error("request aborted")));
  });
}

export function isMainEntrypoint(entry = process.argv[1]): boolean {
  if (!entry) return false;
  try {
    return realpathSync(fileURLToPath(import.meta.url)) === realpathSync(entry);
  } catch {
    return import.meta.url === pathToFileURL(entry).href;
  }
}

if (isMainEntrypoint()) {
  await createLomwaySidecar(loadConfig(process.env)).start();
}
// Internal resource-server introspection is handled here before oidc-provider delegation.
