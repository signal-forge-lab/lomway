import { scryptSync } from "node:crypto";
import { join } from "node:path";

import express from "express";
import {
  getOAuthProtectedResourceMetadataUrl,
  mcpAuthRouter,
} from "@modelcontextprotocol/sdk/server/auth/router.js";
import { resourceUrlFromServerUrl } from "@modelcontextprotocol/sdk/shared/auth-utils.js";

import { SingleUserOAuthProvider } from "./provider.mjs";
import { ownerAuthorizedIntrospection } from "./introspection.mjs";

function requiredEnv(name, minimumLength = 1) {
  const value = process.env[name]?.trim();
  if (!value || value.length < minimumLength) throw new Error(`${name} is required`);
  return value;
}

function integerEnv(name, fallback) {
  const raw = process.env[name];
  if (!raw) return fallback;
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${name} must be a positive integer`);
  return value;
}

const host = process.env.LOMWAY_OAUTH_BIND_HOST?.trim() || "127.0.0.1";
if (!["127.0.0.1", "::1", "localhost"].includes(host)) {
  throw new Error("LOMWAY_OAUTH_BIND_HOST must remain loopback-only");
}
const port = integerEnv("LOMWAY_OAUTH_PORT", 7677);
const issuer = new URL(requiredEnv("LOMWAY_OAUTH_ISSUER"));
const resourceUrl = new URL(requiredEnv("LOMWAY_OAUTH_RESOURCE_URL"));
const ownerCredential = requiredEnv("LOMWAY_OAUTH_OWNER_CREDENTIAL", 16);
const clientRegistrationKey = process.env.LOMWAY_OAUTH_CLIENT_REGISTRATION_KEY?.trim()
  || scryptSync(
    ownerCredential,
    "devspace-oauth-client-registration-v1",
    32,
    { N: 16_384, r: 8, p: 1 },
  ).toString("base64url");
const runtimeDir = requiredEnv("LOMWAY_OAUTH_RUNTIME_DIR");
const databasePath = process.env.LOMWAY_OAUTH_DATABASE_PATH?.trim()
  || join(runtimeDir, "oauth.sqlite");
const scopes = (process.env.LOMWAY_OAUTH_SCOPES || "devspace")
  .split(/\s+/)
  .filter(Boolean);
const allowedRedirectHosts = (process.env.LOMWAY_OAUTH_ALLOWED_REDIRECT_HOSTS
  || "chatgpt.com,localhost,127.0.0.1")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);

const config = {
  ownerCredential,
  clientRegistrationKey,
  accessTokenTtlSeconds: integerEnv("LOMWAY_OAUTH_ACCESS_TOKEN_TTL_SECONDS", 3600),
  refreshTokenTtlSeconds: integerEnv("LOMWAY_OAUTH_REFRESH_TOKEN_TTL_SECONDS", 30 * 24 * 60 * 60),
  scopes,
  allowedRedirectHosts,
  maxRegisteredClients: integerEnv("LOMWAY_OAUTH_MAX_REGISTERED_CLIENTS", 50),
  inactiveClientMaxAgeSeconds: integerEnv(
    "LOMWAY_OAUTH_INACTIVE_CLIENT_MAX_AGE_SECONDS",
    90 * 24 * 60 * 60,
  ),
  authorizationRateLimit: {
    maxFailures: integerEnv("LOMWAY_OAUTH_FAILURE_LIMIT", 5),
    failureWindowMs: integerEnv("LOMWAY_OAUTH_FAILURE_WINDOW_SECONDS", 300) * 1000,
    blockDurationMs: integerEnv("LOMWAY_OAUTH_BLOCK_SECONDS", 900) * 1000,
    failureDelayMs: integerEnv("LOMWAY_OAUTH_FAILURE_DELAY_MS", 250),
  },
};

const provider = new SingleUserOAuthProvider(config, resourceUrl, databasePath);
const app = express();
app.disable("x-powered-by");
app.set("trust proxy", 1);
app.use(express.urlencoded({ extended: false, limit: "4kb" }));

app.get("/healthz", (_req, res) => {
  res.json({ ok: true, name: "lomway-oauth-sidecar" });
});

app.post("/oauth/introspect", async (req, res) => {
  const token = typeof req.body?.token === "string" ? req.body.token : "";
  if (!token) {
    res.json({ active: false });
    return;
  }
  try {
    const info = await provider.verifyAccessToken(token);
    res.json(ownerAuthorizedIntrospection(info));
  } catch {
    res.json({ active: false });
  }
});

const resourceServerUrl = resourceUrlFromServerUrl(resourceUrl);
app.use(mcpAuthRouter({
  provider,
  issuerUrl: issuer,
  baseUrl: issuer,
  resourceServerUrl,
  scopesSupported: scopes,
  resourceName: "Lomway",
}));

app.get("/.well-known/oauth-protected-resource/mcp", (_req, res) => {
  res.json({
    resource: resourceUrl.href,
    authorization_servers: [issuer.href],
    scopes_supported: scopes,
    bearer_methods_supported: ["header"],
  });
});

const server = app.listen(port, host, () => {
  console.log(`Lomway OAuth sidecar listening on http://${host}:${port}`);
  console.log(`issuer: ${issuer.href}`);
  console.log(`resource: ${resourceUrl.href}`);
  console.log(`resource metadata: ${getOAuthProtectedResourceMetadataUrl(resourceServerUrl).href}`);
});

let closing = false;
function close(signal) {
  if (closing) return;
  closing = true;
  server.close(() => {
    provider.close();
    console.log(`Lomway OAuth sidecar stopped (${signal})`);
    process.exit(0);
  });
  setTimeout(() => process.exit(1), 5000).unref();
}
process.on("SIGINT", () => close("SIGINT"));
process.on("SIGTERM", () => close("SIGTERM"));
