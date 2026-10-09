import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { mkdir, open, readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

export const PAT_PREFIX = "lompat_";
const MAX_DAYS = 90;
const MAX_RECORDS = 128;
const NAME_PATTERN = /^[A-Za-z0-9_.:-]{1,128}$/;
const FORBIDDEN_TOOLS = new Set(["proxy/config", "proxy/add_backend"]);

export interface PatRecord {
  id: string;
  label: string;
  digest: string;
  createdAt: number;
  expiresAt: number;
  revokedAt?: number;
  scope: "devspace";
  audience: string;
  allowedTools: string[];
}

interface PatStore {
  version: 1;
  tokens: PatRecord[];
}

export function patStorePath(runtimeDir: string): string {
  return join(runtimeDir, "pat-tokens.json");
}

function hashToken(token: string): Buffer {
  return createHash("sha256").update(token, "utf8").digest();
}

function validateStore(value: unknown): PatStore {
  if (
    typeof value !== "object" || value === null ||
    (value as PatStore).version !== 1 ||
    !Array.isArray((value as PatStore).tokens) ||
    (value as PatStore).tokens.length > MAX_RECORDS
  ) {
    throw new Error("invalid PAT store");
  }
  const store = value as PatStore;
  for (const record of store.tokens) {
    if (
      typeof record?.id !== "string" ||
      typeof record.label !== "string" ||
      typeof record.digest !== "string" ||
      !/^[a-f0-9]{64}$/.test(record.digest) ||
      !Number.isSafeInteger(record.createdAt) ||
      !Number.isSafeInteger(record.expiresAt) ||
      (record.revokedAt !== undefined && !Number.isSafeInteger(record.revokedAt)) ||
      record.scope !== "devspace" ||
      typeof record.audience !== "string" ||
      !Array.isArray(record.allowedTools) ||
      record.allowedTools.some((name) => typeof name !== "string" ||
        (name !== "*" && !NAME_PATTERN.test(name))) ||
      (record.allowedTools.includes("*") && record.allowedTools.length !== 1)
    ) {
      throw new Error("invalid PAT record");
    }
  }
  return store;
}

async function readStore(path: string): Promise<PatStore> {
  try {
    const data = await readFile(path, "utf8");
    return validateStore(JSON.parse(data) as unknown);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return { version: 1, tokens: [] };
    }
    throw error;
  }
}

async function writeStore(path: string, store: PatStore): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temp = path + "." + process.pid + "." + randomBytes(6).toString("hex") + ".tmp";
  await writeFile(temp, JSON.stringify(store), { encoding: "utf8", flag: "wx", mode: 0o600 });
  try {
    await rename(temp, path);
  } catch (error) {
    await unlink(temp).catch(() => undefined);
    throw error;
  }
}

async function lockedUpdate<T>(path: string, update: (store: PatStore) => Promise<T>): Promise<T> {
  const lock = path + ".lock";
  await mkdir(dirname(path), { recursive: true });
  let handle: Awaited<ReturnType<typeof open>> | undefined;
  for (let attempt = 0; attempt < 30; attempt++) {
    try {
      handle = await open(lock, "wx", 0o600);
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      // A killed administrative CLI must not block credential revocation forever.
      const age = await stat(lock).then((x) => Date.now() - x.mtimeMs).catch(() => 0);
      if (age > 30_000) await unlink(lock).catch(() => undefined);
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  if (!handle) throw new Error("PAT store is locked");
  try {
    const store = await readStore(path);
    return await update(store);
  } finally {
    await handle.close();
    await unlink(lock).catch(() => undefined);
  }
}

export async function issuePat(
  path: string,
  options: { label: string; days: number; audience: string; allowedTools: string[] },
): Promise<{ token: string; record: Omit<PatRecord, "digest"> }> {
  const { label, days, audience, allowedTools } = options;
  if (label.trim().length < 1 || label.length > 80) throw new Error("label must be 1..80 characters");
  if (!Number.isInteger(days) || days < 1 || days > MAX_DAYS) throw new Error("days must be 1..90");
  if (audience.length < 1 || !audience.startsWith("https://") || !audience.endsWith("/mcp")) {
    throw new Error("audience must be an HTTPS MCP URL");
  }
  if (
    allowedTools.length < 1 ||
    allowedTools.length > 100 ||
    new Set(allowedTools).size !== allowedTools.length ||
    (allowedTools.includes("*") && allowedTools.length !== 1) ||
    allowedTools.some((name) => name !== "*" &&
      (!NAME_PATTERN.test(name) || FORBIDDEN_TOOLS.has(name)))
  ) {
    throw new Error("explicit, unique permitted tool names are required");
  }
  return lockedUpdate(path, async (store) => {
    // Revoke/expiry makes a credential unusable. Reclaim capacity on issue,
    // rather than letting repeated one-time probes exhaust the store.
    const current = Math.floor(Date.now() / 1000);
    store.tokens = store.tokens.filter(
      (record) => record.revokedAt === undefined && record.expiresAt > current,
    );
    if (store.tokens.length >= MAX_RECORDS) throw new Error("PAT store is full");
    const token = PAT_PREFIX + randomBytes(32).toString("base64url");
    const now = Math.floor(Date.now() / 1000);
    const record: PatRecord = {
      id: randomBytes(12).toString("hex"),
      label: label.trim(),
      digest: hashToken(token).toString("hex"),
      createdAt: now,
      expiresAt: now + days * 86400,
      scope: "devspace",
      audience,
      allowedTools: [...allowedTools],
    };
    store.tokens.push(record);
    await writeStore(path, store);
    const { digest: _digest, ...publicRecord } = record;
    return { token, record: publicRecord };
  });
}

export async function listPats(path: string): Promise<Array<Omit<PatRecord, "digest">>> {
  return (await readStore(path)).tokens.map(({ digest: _digest, ...record }) => record);
}

export async function revokePat(path: string, id: string): Promise<boolean> {
  return lockedUpdate(path, async (store) => {
    const record = store.tokens.find((item) => item.id === id);
    if (!record || record.revokedAt !== undefined) return false;
    record.revokedAt = Math.floor(Date.now() / 1000);
    await writeStore(path, store);
    return true;
  });
}

export async function verifyPat(path: string, token: string, audience: string): Promise<PatRecord | undefined> {
  if (!token.startsWith(PAT_PREFIX) || !/^lompat_[A-Za-z0-9_-]{43}$/.test(token)) return undefined;
  const candidate = hashToken(token);
  const now = Math.floor(Date.now() / 1000);
  const store = await readStore(path);
  let match: PatRecord | undefined;
  for (const record of store.tokens) {
    const equal = timingSafeEqual(candidate, Buffer.from(record.digest, "hex"));
    if (equal && record.revokedAt === undefined && record.expiresAt > now && record.audience === audience) {
      match = record;
    }
  }
  return match;
}
