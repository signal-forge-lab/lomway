import { realpathSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { fileURLToPath, pathToFileURL } from "node:url";

import type Provider from "oidc-provider";

import { loadConfig, type SidecarConfig } from "./config.js";
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
  const failures = new Map<string, number[]>();
  const providerCallback = provider.callback();
  const server = createServer((request, response) => {
    void handleRequest(request, response).catch(() => {
      if (!response.headersSent) response.statusCode = 400;
      if (!response.writableEnded) response.end();
    });
  });
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
    if (request.method === "GET") {
      response.statusCode = 200;
      response.setHeader("content-type", "text/html; charset=utf-8");
      response.end(
        '<!doctype html><html><body><form method="post">' +
          '<label>Password <input name="password" type="password" autocomplete="current-password"></label>' +
          '<button type="submit">Continue</button></form></body></html>',
      );
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

    const key = `${request.socket.remoteAddress ?? ""}\n${uid}`;
    const now = Date.now();
    const recentFailures = (failures.get(key) ?? []).filter(
      (timestamp) => now - timestamp < FAILURE_WINDOW_MS,
    );
    if (recentFailures.length >= FAILURE_LIMIT) {
      failures.set(key, recentFailures);
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
    const password = new URLSearchParams(body).get("password") ?? "";
    if (!(await verifyOwnerPassword(password, await ownerPassword))) {
      recentFailures.push(now);
      failures.set(key, recentFailures);
      response.statusCode = 401;
      response.end();
      return;
    }
    failures.delete(key);

    const interaction = await provider.interactionDetails(request, response);
    if (interaction.prompt.name === "login") {
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
