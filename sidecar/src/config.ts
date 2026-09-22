import { existsSync } from "node:fs";
import { isIP } from "node:net";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { JWK, JWKS } from "oidc-provider";

/**
 * Fail-closed, environment-only configuration for the Lomway OAuth sidecar.
 *
 * Every value is required from the process environment: there is no committed
 * default, no file fallback, and no read of local machine state, SOPS, or the
 * network. Anything that violates the ADR-0005 safety boundary is a hard
 * startup error rather than a silently downgraded default.
 */
export interface SidecarConfig {
  readonly host: "127.0.0.1";
  readonly port: number;
  readonly issuer: string;
  readonly resourceUrl: string;
  readonly ownerCredential: string;
  readonly jwks: JWKS;
  readonly cookieKeys: readonly string[];
  readonly runtimeDir: string;
  readonly statePath: string;
}

/** Environment-like record so the loader is testable without a live process. */
export type SidecarEnvironment = Record<string, string | undefined>;

const LOOPBACK_HOST = "127.0.0.1";

const ENV = {
  host: "LOMWAY_OAUTH_BIND_HOST",
  port: "LOMWAY_OAUTH_PORT",
  issuer: "LOMWAY_OAUTH_ISSUER",
  resourceUrl: "LOMWAY_OAUTH_RESOURCE_URL",
  ownerCredential: "LOMWAY_OAUTH_OWNER_CREDENTIAL",
  jwks: "LOMWAY_OAUTH_JWKS_JSON",
  cookieKeys: "LOMWAY_OAUTH_COOKIE_KEYS_JSON",
  runtimeDir: "LOMWAY_OAUTH_RUNTIME_DIR",
  statePath: "LOMWAY_OAUTH_STATE_PATH",
} as const;

const packageRoot = findPackageRoot(dirname(fileURLToPath(import.meta.url)));
const sourceRoots = [join(packageRoot, "src"), join(packageRoot, "tests")];

export function loadConfig(environment: SidecarEnvironment = process.env): SidecarConfig {
  const host = required(environment, ENV.host);
  if (host !== LOOPBACK_HOST) {
    fail(ENV.host, `must be exactly ${LOOPBACK_HOST}; the sidecar listener is loopback-only`);
  }

  const port = tcpPort(required(environment, ENV.port));
  const issuer = publicHttpsUrl(required(environment, ENV.issuer), ENV.issuer);
  const resourceUrl = publicHttpsUrl(required(environment, ENV.resourceUrl), ENV.resourceUrl);
  const ownerCredential = required(environment, ENV.ownerCredential);
  const jwks = privateJwks(required(environment, ENV.jwks));
  const cookieKeys = signingCookieKeys(required(environment, ENV.cookieKeys));

  const runtimeDir = absolutePath(required(environment, ENV.runtimeDir), ENV.runtimeDir);
  const statePath = absolutePath(required(environment, ENV.statePath), ENV.statePath);

  const stateRelative = relative(runtimeDir, statePath);
  if (stateRelative === "" || stateRelative.startsWith("..") || isAbsolute(stateRelative)) {
    fail(ENV.statePath, `must resolve beneath ${ENV.runtimeDir}`);
  }
  for (const sourceRoot of sourceRoots) {
    const sourceRelative = relative(sourceRoot, statePath);
    if (sourceRelative === "" || (!sourceRelative.startsWith("..") && !isAbsolute(sourceRelative))) {
      fail(ENV.statePath, "must never resolve inside the sidecar source or test tree");
    }
  }

  return {
    host: LOOPBACK_HOST,
    port,
    issuer,
    resourceUrl,
    ownerCredential,
    jwks,
    cookieKeys,
    runtimeDir,
    statePath,
  };
}

function required(environment: SidecarEnvironment, key: string): string {
  const value = environment[key];
  if (value === undefined || value.trim() === "") {
    fail(key, "is required and must not be empty");
  }
  return value;
}

function tcpPort(raw: string): number {
  const port = /^[0-9]+$/.test(raw) ? Number(raw) : Number.NaN;
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    fail(ENV.port, "must be an integer TCP port between 1 and 65535");
  }
  return port;
}

function publicHttpsUrl(raw: string, key: string): string {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return fail(key, "must be an absolute URL");
  }
  if (url.protocol !== "https:") {
    return fail(key, "must be an absolute https URL");
  }
  if (url.username !== "" || url.password !== "") {
    return fail(key, "must not embed credentials");
  }
  if (url.hash !== "") {
    return fail(key, "must not include a fragment");
  }
  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (host === "" || !isPublicHost(host)) {
    return fail(key, "must use a public host, not a loopback, private, or local address");
  }
  return raw;
}

function privateJwks(raw: string): JWKS {
  const value = parseJson(raw, ENV.jwks);
  if (!isRecord(value)) {
    return fail(ENV.jwks, "must be a JSON object containing private signing keys");
  }

  const keys = value.keys;
  if (!Array.isArray(keys) || keys.length === 0 || !keys.every(isPrivateJwk)) {
    return fail(
      ENV.jwks,
      "must contain a non-empty keys array whose entries have non-empty kty and private d fields",
    );
  }
  return { keys };
}

function signingCookieKeys(raw: string): readonly string[] {
  const value = parseJson(raw, ENV.cookieKeys);
  if (
    !Array.isArray(value) ||
    value.length < 2 ||
    !value.every(isStrongCookieKey) ||
    new Set(value).size !== value.length
  ) {
    return fail(
      ENV.cookieKeys,
      "must be a JSON array of at least two distinct non-empty strings of 32 or more characters",
    );
  }
  return value;
}

function parseJson(raw: string, key: string): unknown {
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return fail(key, "must contain valid JSON");
  }
}

function isPrivateJwk(value: unknown): value is JWK & { kty: string; d: string } {
  return isRecord(value) && nonEmptyString(value.kty) && nonEmptyString(value.d);
}

function isStrongCookieKey(value: unknown): value is string {
  return typeof value === "string" && value.trim() !== "" && value.length >= 32;
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim() !== "";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function absolutePath(raw: string, key: string): string {
  if (!isAbsolute(raw)) {
    return fail(key, "must be an absolute path");
  }
  return resolve(raw);
}

function fail(key: string, reason: string): never {
  throw new Error(`${key}: ${reason}`);
}

function isPublicHost(host: string): boolean {
  if (host === "localhost" || host.endsWith(".localhost")) return false;
  if (host.endsWith(".local") || host.endsWith(".internal") || host.endsWith(".home.arpa")) {
    return false;
  }
  const version = isIP(host);
  if (version === 4) return isPublicIpv4(host);
  if (version === 6) return isPublicIpv6(host);
  return true;
}

function isPublicIpv4(address: string): boolean {
  const octets = address.split(".").map(Number);
  const [first, second] = octets;
  if (octets.length !== 4 || first === undefined || second === undefined) return false;
  return (
    first > 0 &&
    first < 224 &&
    first !== 10 &&
    first !== 127 &&
    !(first === 100 && second >= 64 && second <= 127) &&
    !(first === 169 && second === 254) &&
    !(first === 172 && second >= 16 && second <= 31) &&
    !(first === 192 && second === 168) &&
    !(first === 192 && second === 0) &&
    !(first === 198 && (second === 18 || second === 19))
  );
}

function isPublicIpv6(address: string): boolean {
  const normalized = address.toLowerCase();
  return (
    normalized !== "::" &&
    normalized !== "::1" &&
    !normalized.startsWith("fc") &&
    !normalized.startsWith("fd") &&
    !normalized.startsWith("fe8") &&
    !normalized.startsWith("fe9") &&
    !normalized.startsWith("fea") &&
    !normalized.startsWith("feb")
  );
}

/** Locate the sidecar package root so runtime state can never land in `src`/`tests`. */
function findPackageRoot(start: string): string {
  let directory = start;
  for (;;) {
    if (existsSync(join(directory, "package.json"))) return directory;
    const parent = dirname(directory);
    if (parent === directory) return start;
    directory = parent;
  }
}
