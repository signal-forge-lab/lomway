import { createHmac, randomUUID, timingSafeEqual } from "node:crypto";
import { chmodSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

import Database from "better-sqlite3";
import { InvalidRequestError } from "@modelcontextprotocol/sdk/server/auth/errors.js";
import { OAuthClientInformationFullSchema } from "@modelcontextprotocol/sdk/shared/auth.js";

import { isAllowedOAuthRedirectUri } from "./security.mjs";

const CLIENT_ID_PREFIX = "lomway-v1";
const LEGACY_CLIENT_ID_PREFIX = "devspace-v1";
const MAX_CLIENT_ID_LENGTH = 4096;

function safeEquals(left, right) {
  const leftBuffer = Buffer.from(left);
  const rightBuffer = Buffer.from(right);
  if (leftBuffer.byteLength !== rightBuffer.byteLength) return false;
  return timingSafeEqual(leftBuffer, rightBuffer);
}

function sign(value, signingKey) {
  return createHmac("sha256", signingKey).update(value).digest("base64url");
}

function isPublicClient(client) {
  return (
    client.token_endpoint_auth_method === "none"
    && client.client_secret === undefined
    && client.client_secret_expires_at === undefined
  );
}

function createRecoverableClientId(client, signingKey) {
  const parsed = OAuthClientInformationFullSchema.safeParse({ ...client, client_id: "pending" });
  if (!parsed.success || !isPublicClient(parsed.data)) return undefined;
  const { client_id: _clientId, ...registration } = parsed.data;
  const payload = Buffer.from(JSON.stringify(registration)).toString("base64url");
  const signedValue = `${CLIENT_ID_PREFIX}.${payload}`;
  const clientId = `${signedValue}.${sign(signedValue, signingKey)}`;
  if (clientId.length > MAX_CLIENT_ID_LENGTH) return undefined;
  return { ...registration, client_id: clientId };
}

function recoverClientRegistration(clientId, signingKey) {
  if (clientId.length > MAX_CLIENT_ID_LENGTH) return undefined;
  const [prefix, payload, signature, extra] = clientId.split(".");
  if (
    (prefix !== CLIENT_ID_PREFIX && prefix !== LEGACY_CLIENT_ID_PREFIX)
    || !payload
    || !signature
    || extra !== undefined
  ) return undefined;
  const signedValue = `${prefix}.${payload}`;
  if (!safeEquals(signature, sign(signedValue, signingKey))) return undefined;

  let decoded;
  try {
    decoded = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
  } catch {
    return undefined;
  }
  const parsed = OAuthClientInformationFullSchema.safeParse({ ...decoded, client_id: clientId });
  return parsed.success && isPublicClient(parsed.data) ? parsed.data : undefined;
}

export function openOAuthDatabase(databasePath) {
  mkdirSync(dirname(databasePath), { recursive: true, mode: 0o700 });
  chmodSync(dirname(databasePath), 0o700);
  const database = new Database(databasePath);
  chmodSync(databasePath, 0o600);
  database.pragma("journal_mode = WAL");
  database.pragma("synchronous = NORMAL");
  database.pragma("busy_timeout = 5000");
  database.pragma("foreign_keys = ON");
  database.exec(`
    create table if not exists oauth_clients (
      client_id text primary key,
      client_json text not null,
      issued_at integer not null
    );
    create index if not exists oauth_clients_issued_at_idx
      on oauth_clients(issued_at desc);
    create table if not exists oauth_access_tokens (
      token_hash text primary key,
      client_id text not null,
      scopes_json text not null,
      expires_at integer not null,
      resource text,
      foreign key (client_id) references oauth_clients(client_id) on delete cascade
    );
    create index if not exists oauth_access_tokens_client_id_idx
      on oauth_access_tokens(client_id);
    create index if not exists oauth_access_tokens_expires_at_idx
      on oauth_access_tokens(expires_at);
    create table if not exists oauth_refresh_tokens (
      token_hash text primary key,
      client_id text not null,
      scopes_json text not null,
      expires_at integer not null,
      resource text,
      foreign key (client_id) references oauth_clients(client_id) on delete cascade
    );
    create index if not exists oauth_refresh_tokens_client_id_idx
      on oauth_refresh_tokens(client_id);
    create index if not exists oauth_refresh_tokens_expires_at_idx
      on oauth_refresh_tokens(expires_at);
  `);
  return database;
}

export class OAuthStore {
  constructor(databasePath, inactiveClientMaxAgeSeconds) {
    this.database = openOAuthDatabase(databasePath);
    const nowSeconds = Math.floor(Date.now() / 1000);
    this.database.prepare("delete from oauth_access_tokens where expires_at < ?").run(nowSeconds);
    this.database.prepare("delete from oauth_refresh_tokens where expires_at < ?").run(nowSeconds);
    if (inactiveClientMaxAgeSeconds !== undefined) {
      this.database.prepare(`
        delete from oauth_clients
        where issued_at < ?
          and not exists (
            select 1 from oauth_access_tokens
            where oauth_access_tokens.client_id = oauth_clients.client_id
          )
          and not exists (
            select 1 from oauth_refresh_tokens
            where oauth_refresh_tokens.client_id = oauth_clients.client_id
          )
      `).run(nowSeconds - inactiveClientMaxAgeSeconds);
    }
  }

  close() {
    this.database.close();
  }

  getClient(clientId) {
    const row = this.database
      .prepare("select client_json from oauth_clients where client_id = ?")
      .get(clientId);
    return row ? JSON.parse(row.client_json) : undefined;
  }

  saveClient(client) {
    this.database.prepare(`
      insert into oauth_clients (client_id, client_json, issued_at)
      values (?, ?, ?)
      on conflict(client_id) do nothing
    `).run(client.client_id, JSON.stringify(client), client.client_id_issued_at ?? 0);
  }

  getAccessToken(tokenHash) {
    const row = this.database.prepare(`
      select client_id, scopes_json, expires_at, resource
      from oauth_access_tokens where token_hash = ?
    `).get(tokenHash);
    if (!row) return undefined;
    return {
      clientId: row.client_id,
      scopes: JSON.parse(row.scopes_json),
      expiresAt: row.expires_at,
      resource: row.resource ?? undefined,
    };
  }

  getRefreshToken(tokenHash) {
    const row = this.database.prepare(`
      select client_id, scopes_json, expires_at, resource
      from oauth_refresh_tokens where token_hash = ?
    `).get(tokenHash);
    if (!row) return undefined;
    return {
      clientId: row.client_id,
      scopes: JSON.parse(row.scopes_json),
      expiresAt: row.expires_at,
      resource: row.resource ?? undefined,
    };
  }

  deleteAccessToken(tokenHash) {
    this.database.prepare("delete from oauth_access_tokens where token_hash = ?").run(tokenHash);
  }

  deleteRefreshToken(tokenHash) {
    this.database.prepare("delete from oauth_refresh_tokens where token_hash = ?").run(tokenHash);
  }

  saveTokenPair(pair, consumedRefreshTokenHash) {
    const save = this.database.transaction(() => {
      if (consumedRefreshTokenHash) {
        const deleted = this.database
          .prepare("delete from oauth_refresh_tokens where token_hash = ?")
          .run(consumedRefreshTokenHash);
        if (deleted.changes !== 1) return false;
      }
      this.database.prepare(`
        insert into oauth_access_tokens (token_hash, client_id, scopes_json, expires_at, resource)
        values (?, ?, ?, ?, ?)
        on conflict(token_hash) do update set
          client_id = excluded.client_id,
          scopes_json = excluded.scopes_json,
          expires_at = excluded.expires_at,
          resource = excluded.resource
      `).run(
        pair.accessTokenHash,
        pair.accessToken.clientId,
        JSON.stringify(pair.accessToken.scopes),
        pair.accessToken.expiresAt,
        pair.accessToken.resource ?? null,
      );
      this.database.prepare(`
        insert into oauth_refresh_tokens (token_hash, client_id, scopes_json, expires_at, resource)
        values (?, ?, ?, ?, ?)
        on conflict(token_hash) do update set
          client_id = excluded.client_id,
          scopes_json = excluded.scopes_json,
          expires_at = excluded.expires_at,
          resource = excluded.resource
      `).run(
        pair.refreshTokenHash,
        pair.refreshToken.clientId,
        JSON.stringify(pair.refreshToken.scopes),
        pair.refreshToken.expiresAt,
        pair.refreshToken.resource ?? null,
      );
      return true;
    });
    return save.immediate();
  }
}

export class OAuthClientsStore {
  constructor(store, allowedRedirectHosts, signingKey, maxRegisteredClients = 50) {
    this.store = store;
    this.allowedRedirectHosts = allowedRedirectHosts;
    this.signingKey = signingKey;
    this.maxRegisteredClients = maxRegisteredClients;
  }

  getClient(clientId) {
    const stored = this.store.getClient(clientId);
    if (stored) return stored;
    const recovered = recoverClientRegistration(clientId, this.signingKey);
    if (!recovered) return undefined;
    if (!recovered.redirect_uris.every((uri) =>
      isAllowedOAuthRedirectUri(String(uri), this.allowedRedirectHosts))) return undefined;
    return recovered;
  }

  registerClient(client) {
    if (!client.redirect_uris.every((uri) =>
      isAllowedOAuthRedirectUri(String(uri), this.allowedRedirectHosts))) {
      throw new InvalidRequestError("Client redirect_uri is not allowed for this Lomway server");
    }
    const count = this.store.database.prepare("select count(*) from oauth_clients").pluck().get();
    if (count >= this.maxRegisteredClients) {
      throw new InvalidRequestError(`OAuth client registration limit reached (${this.maxRegisteredClients})`);
    }
    const now = Math.floor(Date.now() / 1000);
    const registration = {
      ...client,
      client_id_issued_at: now,
      token_endpoint_auth_method: client.token_endpoint_auth_method ?? "none",
      grant_types: client.grant_types ?? ["authorization_code", "refresh_token"],
      response_types: client.response_types ?? ["code"],
    };
    const registered = createRecoverableClientId(registration, this.signingKey) ?? {
      ...registration,
      client_id: `lomway-${randomUUID()}`,
    };
    this.store.saveClient(registered);
    return registered;
  }
}
