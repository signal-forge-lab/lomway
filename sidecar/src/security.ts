import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { randomBytes, scrypt, timingSafeEqual } from "node:crypto";

function derivePassword(password: string, salt: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    scrypt(password, salt, PASSWORD_KEY_BYTES, {
      N: PASSWORD_COST,
      r: 8,
      p: 1,
      maxmem: 32 * 1024 * 1024,
    }, (error, key) => error ? reject(error) : resolve(key));
  });
}

const PASSWORD_KEY_BYTES = 32;
const PASSWORD_SALT_BYTES = 16;
const PASSWORD_COST = 16_384;

export interface PasswordRecord {
  salt: string;
  hash: string;
}

/**
 * Validate the non-network portion of a CIMD URL before the provider fetches
 * it. The provider's own dispatcher adds connection-time private-address
 * protection; this check keeps unsafe schemes, credentials, and URL forms
 * out of that fetch path entirely.
 */
export function assertSafeCimdUrl(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("unsafe CIMD URL: malformed URL");
  }

  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (
    url.protocol !== "https:" ||
    url.username !== "" ||
    url.password !== "" ||
    url.hash !== "" ||
    url.search !== "" ||
    host === "" ||
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host.endsWith(".local") ||
    host.endsWith(".internal") ||
    isIP(host) !== 0
  ) {
    throw new Error("unsafe CIMD URL: scheme, host, or URL credentials are not allowed");
  }

  return url;
}

/**
 * Resolve a CIMD host before use. The final HTTP request still uses
 * oidc-provider's SSRF-aware dispatcher, which closes the DNS-rebinding race
 * between this advisory check and the socket connection.
 */
export async function assertPublicCimdHost(url: URL): Promise<void> {
  const addresses = await lookup(url.hostname, { all: true, verbatim: true });
  if (addresses.length === 0 || addresses.some(({ address }) => !isPublicAddress(address))) {
    throw new Error("unsafe CIMD URL: host does not resolve to a public address");
  }
}

export async function safeCimdFetch(
  input: string | URL | Request,
  init?: RequestInit,
): Promise<Response> {
  const url = assertSafeCimdUrl(
    typeof input === "string" ? input : input instanceof URL ? input.href : input.url,
  );
  await assertPublicCimdHost(url);
  return fetch(input, { ...init, redirect: "error" });
}

export async function hashOwnerPassword(password: string): Promise<PasswordRecord> {
  if (password.length === 0) {
    throw new Error("owner password must not be empty");
  }
  const salt = randomBytes(PASSWORD_SALT_BYTES);
  const hash = await derivePassword(password, salt);
  return {
    salt: salt.toString("base64url"),
    hash: hash.toString("base64url"),
  };
}

export async function verifyOwnerPassword(
  password: string,
  record: PasswordRecord,
): Promise<boolean> {
  try {
    const salt = Buffer.from(record.salt, "base64url");
    const expected = Buffer.from(record.hash, "base64url");
    if (salt.length !== PASSWORD_SALT_BYTES || expected.length !== PASSWORD_KEY_BYTES) {
      return false;
    }
    const actual = await derivePassword(password, salt);
    return timingSafeEqual(actual, expected);
  } catch {
    return false;
  }
}

export function constantTimeEqual(left: string, right: string): boolean {
  const leftBytes = Buffer.from(left);
  const rightBytes = Buffer.from(right);
  const length = Math.max(leftBytes.length, rightBytes.length);
  const paddedLeft = Buffer.alloc(length);
  const paddedRight = Buffer.alloc(length);
  leftBytes.copy(paddedLeft);
  rightBytes.copy(paddedRight);
  return timingSafeEqual(paddedLeft, paddedRight) && leftBytes.length === rightBytes.length;
}

export function isPublicAddress(address: string): boolean {
  if (isIP(address) === 4) {
    const octets = address.split(".").map(Number);
    const [first, second] = octets;
    return (
      octets.length === 4 &&
      first !== undefined &&
      second !== undefined &&
      first > 0 &&
      first < 224 &&
      first !== 10 &&
      !(first === 100 && second >= 64 && second <= 127) &&
      !(first === 127) &&
      !(first === 169 && second === 254) &&
      !(first === 172 && second >= 16 && second <= 31) &&
      !(first === 192 && second === 168) &&
      !(first === 192 && second === 0) &&
      !(first === 198 && second >= 18 && second <= 19)
    );
  }

  if (isIP(address) === 6) {
    const normalized = address.toLowerCase();
    return (
      normalized !== "::" &&
      normalized !== "::1" &&
      !normalized.startsWith("::ffff:") &&
      !normalized.startsWith("fc") &&
      !normalized.startsWith("fd") &&
      !normalized.startsWith("fe8") &&
      !normalized.startsWith("fe9") &&
      !normalized.startsWith("fea") &&
      !normalized.startsWith("feb")
    );
  }

  return false;
}
