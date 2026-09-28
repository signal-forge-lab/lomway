import { createHash, randomBytes, randomUUID, timingSafeEqual } from "node:crypto";

import {
  AccessDeniedError,
  InvalidGrantError,
  InvalidRequestError,
  InvalidTokenError,
} from "@modelcontextprotocol/sdk/server/auth/errors.js";
import {
  checkResourceAllowed,
  resourceUrlFromServerUrl,
} from "@modelcontextprotocol/sdk/shared/auth-utils.js";

import { OAuthClientsStore, OAuthStore } from "./store.mjs";
import {
  AuthorizationAttemptLimiter,
  delay,
  requestAddress,
} from "./security.mjs";

const CODE_TTL_MS = 5 * 60 * 1000;

function randomToken() {
  return randomBytes(32).toString("base64url");
}

function hashToken(token) {
  return createHash("sha256").update(token).digest("base64url");
}

function safeEquals(left, right) {
  const leftBuffer = Buffer.from(left);
  const rightBuffer = Buffer.from(right);
  if (leftBuffer.byteLength !== rightBuffer.byteLength) return false;
  return timingSafeEqual(leftBuffer, rightBuffer);
}

function htmlEscape(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function authorizationFormFields(client, params) {
  return {
    response_type: "code",
    client_id: client.client_id,
    redirect_uri: params.redirectUri,
    code_challenge: params.codeChallenge,
    code_challenge_method: "S256",
    scope: params.scopes?.join(" "),
    state: params.state,
    resource: params.resource?.href,
  };
}

function formHtml({ error, clientName, scopes, resource, fields }) {
  const hiddenFields = Object.entries(fields)
    .filter(([, value]) => value !== undefined)
    .map(([name, value]) =>
      `<input type="hidden" name="${htmlEscape(name)}" value="${htmlEscape(value)}" />`)
    .join("\n");
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Connect Lomway</title>
  <style>
    body{font-family:system-ui,-apple-system,sans-serif;margin:0;background:#0f172a;color:#e2e8f0}
    main{max-width:440px;margin:12vh auto;padding:32px;background:#111827;border:1px solid #334155;border-radius:18px}
    h1{margin:0 0 12px;font-size:28px}p{line-height:1.5;color:#cbd5e1}
    dl{padding:16px;background:#020617;border-radius:12px}dt{color:#94a3b8;font-size:12px;text-transform:uppercase}
    dd{margin:4px 0 12px;word-break:break-word}label{display:block;margin:18px 0 8px;font-weight:600}
    input{box-sizing:border-box;width:100%;padding:12px 14px;border-radius:10px;border:1px solid #475569;background:#020617;color:#e2e8f0;font-size:16px}
    button{margin-top:18px;width:100%;border:0;border-radius:10px;padding:12px 14px;font-weight:700;background:#38bdf8;color:#020617}
    .error{color:#fecaca;background:#7f1d1d;border-radius:10px;padding:10px 12px}.warning{color:#fde68a}
  </style>
</head>
<body><main>
  <h1>Connect Lomway</h1>
  <p class="warning">Only approve this if you are intentionally connecting your own ChatGPT or MCP client to this local machine.</p>
  ${error ? `<p class="error">${htmlEscape(error)}</p>` : ""}
  <dl>
    <dt>Client</dt><dd>${htmlEscape(clientName)}</dd>
    <dt>Scope</dt><dd>${htmlEscape(scopes.join(" ") || "devspace")}</dd>
    <dt>Resource</dt><dd>${htmlEscape(resource?.href ?? "Lomway MCP endpoint")}</dd>
  </dl>
  <form method="post">
    ${hiddenFields}
    <label for="owner_token">Owner password</label>
    <input id="owner_token" name="owner_token" type="password" autocomplete="current-password" autofocus required />
    <button type="submit">Authorize Lomway</button>
  </form>
</main></body></html>`;
}

function sendAuthorizationForm(res, client, params, error, status = 200, retryAfterSeconds) {
  res.status(status).setHeader("Content-Type", "text/html; charset=utf-8");
  if (retryAfterSeconds !== undefined) res.setHeader("Retry-After", String(retryAfterSeconds));
  res.send(formHtml({
    error,
    clientName: client.client_name ?? client.client_id,
    scopes: params.scopes ?? [],
    resource: params.resource,
    fields: authorizationFormFields(client, params),
  }));
}

export class SingleUserOAuthProvider {
  #codes = new Map();

  constructor(config, resourceServerUrl, databasePath) {
    this.config = config;
    this.resourceServerUrl = resourceUrlFromServerUrl(resourceServerUrl);
    this.store = new OAuthStore(databasePath, config.inactiveClientMaxAgeSeconds);
    this.clientsStore = new OAuthClientsStore(
      this.store,
      config.allowedRedirectHosts,
      config.clientRegistrationKey,
      config.maxRegisteredClients,
    );
    this.authorizationAttempts = new AuthorizationAttemptLimiter(config.authorizationRateLimit);
  }

  close() {
    this.store.close();
  }

  async authorize(client, params, res) {
    if (
      !params.resource
      || !checkResourceAllowed({
        requestedResource: params.resource,
        configuredResource: this.resourceServerUrl,
      })
    ) throw new InvalidRequestError("Invalid or missing OAuth resource");
    if (!(params.scopes ?? []).every((scope) => this.config.scopes.includes(scope))) {
      throw new InvalidRequestError("Requested scope is not supported");
    }

    if (res.req.method !== "POST") {
      sendAuthorizationForm(res, client, params);
      return;
    }

    const attemptKey = requestAddress(res.req);
    this.authorizationAttempts.prune();
    const existingRetryAfterMs = this.authorizationAttempts.retryAfterMs(attemptKey);
    if (existingRetryAfterMs > 0) {
      sendAuthorizationForm(
        res,
        client,
        params,
        `Too many failed attempts. Try again in ${Math.max(1, Math.ceil(existingRetryAfterMs / 1000))} seconds.`,
        429,
        Math.max(1, Math.ceil(existingRetryAfterMs / 1000)),
      );
      return;
    }

    const providedToken = String(res.req.body?.owner_token ?? "");
    if (!safeEquals(providedToken, this.config.ownerCredential)) {
      await delay(this.config.authorizationRateLimit.failureDelayMs);
      const retryAfterMs = this.authorizationAttempts.recordFailure(attemptKey);
      if (retryAfterMs > 0) {
        const seconds = Math.max(1, Math.ceil(retryAfterMs / 1000));
        sendAuthorizationForm(
          res,
          client,
          params,
          `Too many failed attempts. Try again in ${seconds} seconds.`,
          429,
          seconds,
        );
        return;
      }
      sendAuthorizationForm(res, client, params, "The Owner password was not accepted.", 401);
      return;
    }

    this.authorizationAttempts.recordSuccess(attemptKey);
    this.#pruneAuthorizationCodes();
    if (!this.store.getClient(client.client_id)) this.store.saveClient(client);
    const code = `code-${randomUUID()}`;
    this.#codes.set(code, {
      clientId: client.client_id,
      params,
      expiresAtMs: Date.now() + CODE_TTL_MS,
    });
    const redirectUrl = new URL(params.redirectUri);
    redirectUrl.searchParams.set("code", code);
    if (params.state !== undefined) redirectUrl.searchParams.set("state", params.state);
    res.redirect(302, redirectUrl.href);
  }

  async challengeForAuthorizationCode(client, authorizationCode) {
    return this.#validCodeRecord(client, authorizationCode).params.codeChallenge;
  }

  async exchangeAuthorizationCode(client, authorizationCode, _codeVerifier, redirectUri, resource) {
    const record = this.#validCodeRecord(client, authorizationCode);
    if (redirectUri && redirectUri !== record.params.redirectUri) {
      throw new InvalidGrantError("redirect_uri does not match the authorization request");
    }
    if (
      resource
      && !checkResourceAllowed({
        requestedResource: resource,
        configuredResource: this.resourceServerUrl,
      })
    ) throw new InvalidGrantError("Invalid resource");
    this.#codes.delete(authorizationCode);
    return this.#issueTokens(
      client.client_id,
      record.params.scopes ?? this.config.scopes,
      record.params.resource,
    );
  }

  async exchangeRefreshToken(client, refreshToken, scopes, resource) {
    const refreshTokenHash = hashToken(refreshToken);
    const record = this.store.getRefreshToken(refreshTokenHash);
    if (
      !record
      || record.clientId !== client.client_id
      || record.expiresAt < Math.floor(Date.now() / 1000)
    ) throw new InvalidGrantError("Invalid refresh token");
    if (
      resource
      && !checkResourceAllowed({
        requestedResource: resource,
        configuredResource: this.resourceServerUrl,
      })
    ) throw new InvalidGrantError("Invalid resource");
    const requestedScopes = scopes ?? record.scopes;
    if (!requestedScopes.every((scope) => record.scopes.includes(scope))) {
      throw new AccessDeniedError("Refresh token cannot grant requested scopes");
    }
    return this.#issueTokens(
      client.client_id,
      requestedScopes,
      resource ?? (record.resource ? new URL(record.resource) : undefined),
      refreshTokenHash,
    );
  }

  async verifyAccessToken(token) {
    const record = this.store.getAccessToken(hashToken(token));
    if (!record || record.expiresAt < Math.floor(Date.now() / 1000)) {
      throw new InvalidTokenError("Invalid or expired access token");
    }
    return {
      token,
      clientId: record.clientId,
      scopes: record.scopes,
      expiresAt: record.expiresAt,
      resource: record.resource ? new URL(record.resource) : undefined,
    };
  }

  async revokeToken(_client, request) {
    const hashed = hashToken(request.token);
    this.store.deleteAccessToken(hashed);
    this.store.deleteRefreshToken(hashed);
  }

  #validCodeRecord(client, authorizationCode) {
    this.#pruneAuthorizationCodes();
    const record = this.#codes.get(authorizationCode);
    if (!record || record.clientId !== client.client_id || record.expiresAtMs < Date.now()) {
      throw new InvalidGrantError("Invalid authorization code");
    }
    return record;
  }

  #pruneAuthorizationCodes(nowMs = Date.now()) {
    for (const [code, record] of this.#codes) {
      if (record.expiresAtMs < nowMs) this.#codes.delete(code);
    }
  }

  #issueTokens(clientId, scopes, resource, consumedRefreshTokenHash) {
    const now = Math.floor(Date.now() / 1000);
    const accessToken = randomToken();
    const refreshToken = randomToken();
    const pair = {
      accessTokenHash: hashToken(accessToken),
      accessToken: {
        clientId,
        scopes,
        expiresAt: now + this.config.accessTokenTtlSeconds,
        resource: resource?.href,
      },
      refreshTokenHash: hashToken(refreshToken),
      refreshToken: {
        clientId,
        scopes,
        expiresAt: now + this.config.refreshTokenTtlSeconds,
        resource: resource?.href,
      },
    };
    if (!this.store.saveTokenPair(pair, consumedRefreshTokenHash)) {
      throw new InvalidGrantError("Invalid refresh token");
    }
    return {
      access_token: accessToken,
      token_type: "bearer",
      expires_in: this.config.accessTokenTtlSeconds,
      refresh_token: refreshToken,
      scope: scopes.join(" "),
    };
  }
}
