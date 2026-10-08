import { createHash, randomBytes } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

import type { Adapter, AdapterPayload } from "oidc-provider";

/**
 * On-disk shape for the ADR-0005 sidecar state file.
 *
 * Records are namespaced by oidc-provider model name first and record id
 * second, so the same id used by two different models can never collide. An
 * empty store is `{ "records": {} }`.
 */
interface StateFile {
  records: Record<string, Record<string, AdapterPayload>>;
}

/**
 * Serialize read/modify/write cycles per state file. Adapter instances for
 * different model names share one file, so the queue is keyed by the resolved
 * path rather than by instance to avoid lost updates within the process.
 */
const operationQueues = new Map<string, Promise<void>>();
// The opaque jti is a bearer credential for these models. Never persist it
// as a JSON key or inside a JSON value, including migrated legacy records.
const sensitiveModels = new Set(["AccessToken", "RefreshToken", "AuthorizationCode"]);

function storedKey(model: string, id: string): string {
  return sensitiveModels.has(model)
    ? "sha256:" + createHash("sha256").update(model + ":" + id).digest("hex")
    : id;
}

function persistedPayload(model: string, id: string, record: AdapterPayload): AdapterPayload {
  if (!sensitiveModels.has(model)) return record;
  const copy = jsonSafeCopy(record);
  if (copy.jti !== undefined && copy.jti !== id) {
    throw new Error("credential id mismatch");
  }
  if (copy.jti !== undefined) {
    (copy as AdapterPayload & { _restoreOpaqueJti?: boolean })._restoreOpaqueJti = true;
    delete copy.jti;
  }
  return copy;
}

function restoredPayload(model: string, id: string, record: AdapterPayload): AdapterPayload {
  const copy = jsonSafeCopy(record);
  const marker = copy as AdapterPayload & { _restoreOpaqueJti?: boolean };
  if (sensitiveModels.has(model) && marker._restoreOpaqueJti) {
    copy.jti = id;
    delete marker._restoreOpaqueJti;
  }
  return copy;
}

function enqueue<T>(statePath: string, task: () => Promise<T>): Promise<T> {
  const key = resolve(statePath);
  const previous = operationQueues.get(key) ?? Promise.resolve();
  const result = previous.then(task, task);
  operationQueues.set(
    key,
    result.then(
      () => undefined,
      () => undefined,
    ),
  );
  return result;
}

/**
 * Create a factory for repository-owned persistent adapters backed by a single
 * JSON state file. The returned factory matches oidc-provider's `AdapterFactory`
 * shape and is instantiated once per model name by the provider.
 */
export function createPersistentAdapter(statePath: string): (modelName: string) => Adapter {
  return (modelName: string): Adapter => ({
    async upsert(id, payload, expiresIn) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        // A racing refresh must not resurrect a revoked grant family.
        const grantId = payload.grantId;
        if (
          typeof grantId === "string" &&
          state.records.RevokedGrant?.[grantId] !== undefined
        ) {
          throw new Error("grant family is revoked");
        }
        const record = persistedPayload(modelName, id, payload);
        if (typeof expiresIn === "number" && Number.isFinite(expiresIn)) {
          record.exp = nowSeconds() + expiresIn;
        }
        modelBucket(state, modelName)[storedKey(modelName, id)] = record;
        extendGrantThroughRefreshToken(state, modelName, record);
        await writeState(statePath, state);
      });
    },

    async find(id) {
      return enqueue(statePath, async () => {
        const state = await readState(statePath);
        const record = state.records[modelName]?.[storedKey(modelName, id)];
        if (record === undefined) {
          return undefined;
        }
        if (isExpired(record)) {
          deleteRecord(state, modelName, storedKey(modelName, id));
          await writeState(statePath, state);
          return undefined;
        }
        // A legacy token issued before expiresWithSession=false must first
        // prove it still belongs to a live owner session and its own grant.
        // Only then may it adopt the new sliding-refresh policy.
        if (modelName === "RefreshToken" && record.expiresWithSession === true &&
          record.consumed === undefined && legacySessionStillAuthorized(state, record)) {
          record.expiresWithSession = false;
          await writeState(statePath, state);
        }
        return restoredPayload(modelName, id, record);
      });
    },

    async consume(id) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        const record = state.records[modelName]?.[storedKey(modelName, id)];
        if (record === undefined) {
          throw new Error("one-time credential is missing");
        }
        if (record.consumed !== undefined) {
          const grantId = record.grantId;
          if (typeof grantId === "string" && grantId.length > 0) {
            markGrantRevoked(state, grantId);
            await writeState(statePath, state);
          }
          throw new Error("one-time credential reuse detected");
        }
        record.consumed = nowSeconds();
        await writeState(statePath, state);
      });
    },

    async destroy(id) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        if (deleteRecord(state, modelName, storedKey(modelName, id))) {
          await writeState(statePath, state);
        }
      });
    },

    async revokeByGrantId(grantId) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        markGrantRevoked(state, grantId);
        await writeState(statePath, state);
      });
    },

    async findByUserCode(userCode) {
      return findFirstInModel(statePath, modelName, (record) => record.userCode === userCode);
    },

    async findByUid(uid) {
      return findFirstInModel(statePath, modelName, (record) => record.uid === uid);
    },
  });
}

function legacySessionStillAuthorized(state: StateFile, token: AdapterPayload): boolean {
  if (
    typeof token.sessionUid !== "string" ||
    typeof token.clientId !== "string" ||
    typeof token.grantId !== "string" ||
    typeof token.accountId !== "string"
  ) {
    return false;
  }
  const clientId = token.clientId;
  const grantId = token.grantId;
  const now = nowSeconds();
  const grant = state.records.Grant?.[grantId];
  if (!grant || typeof grant.exp !== "number" || grant.exp <= now ||
    grant.clientId !== token.clientId || grant.accountId !== token.accountId) {
    return false;
  }
  return Object.values(state.records.Session ?? {}).some((session) => {
    if (session.uid !== token.sessionUid ||
      session.accountId !== token.accountId ||
      typeof session.exp !== "number" || session.exp <= now) return false;
    const permissions = session.authorizations as
      Record<string, { grantId?: string }> | undefined;
    return permissions?.[clientId]?.grantId === grantId;
  });
}

function markGrantRevoked(state: StateFile, grantId: string): void {
  for (const [model, bucket] of Object.entries(state.records)) {
    if (model === "RevokedGrant") continue;
    for (const [id, record] of Object.entries(bucket)) {
      if (record.grantId === grantId || (model === "Grant" && id === grantId)) {
        delete bucket[id];
      }
    }
    if (Object.keys(bucket).length === 0) {
      delete state.records[model];
    }
  }
  modelBucket(state, "RevokedGrant")[grantId] = {
    exp: nowSeconds() + 31 * 24 * 3600,
  };
}

function extendGrantThroughRefreshToken(
  state: StateFile,
  modelName: string,
  record: AdapterPayload,
): void {
  if (modelName !== "RefreshToken") {
    return;
  }

  const grantId = record.grantId;
  const refreshExpiry = record.exp;
  if (
    typeof grantId !== "string" ||
    grantId.length === 0 ||
    typeof refreshExpiry !== "number" ||
    !Number.isFinite(refreshExpiry)
  ) {
    return;
  }

  const grant = state.records.Grant?.[grantId];
  if (grant === undefined) {
    return;
  }

  const grantExpiry = grant.exp;
  if (
    typeof grantExpiry !== "number" ||
    !Number.isFinite(grantExpiry) ||
    grantExpiry < refreshExpiry
  ) {
    grant.exp = refreshExpiry;
  }
}

async function findFirstInModel(
  statePath: string,
  modelName: string,
  matches: (record: AdapterPayload) => boolean,
): Promise<AdapterPayload | undefined> {
  return enqueue(statePath, async () => {
    const state = await readState(statePath);
    const bucket = state.records[modelName];
    if (bucket === undefined) {
      return undefined;
    }
    for (const record of Object.values(bucket)) {
      if (!isExpired(record) && matches(record)) {
        return jsonSafeCopy(record);
      }
    }
    return undefined;
  });
}

async function readState(statePath: string): Promise<StateFile> {
  let raw: string;
  try {
    raw = await readFile(statePath, "utf8");
  } catch (error) {
    if (isNotFound(error)) {
      return emptyState();
    }
    throw error;
  }
  if (raw.trim() === "") {
    throw new Error("sidecar state file is empty; refusing to reset existing credentials");
  }
  const state = normalizeState(JSON.parse(raw));
  let migrated = false;
  for (const model of sensitiveModels) {
    const bucket = state.records[model];
    if (!bucket) continue;
    for (const [legacyId, record] of Object.entries(bucket)) {
      if (legacyId.startsWith("sha256:")) continue;
      const key = storedKey(model, legacyId);
      if (bucket[key] !== undefined) {
        throw new Error("duplicate legacy and hashed bearer credential");
      }
      bucket[key] = persistedPayload(model, legacyId, record);
      delete bucket[legacyId];
      migrated = true;
    }
  }
  if (migrated) await writeState(statePath, state);
  return state;
}

async function writeState(statePath: string, state: StateFile): Promise<void> {
  await mkdir(dirname(statePath), { recursive: true });
  const tempPath = `${statePath}.${process.pid}.${randomBytes(6).toString("hex")}.tmp`;
  await writeFile(tempPath, JSON.stringify(state), "utf8");
  await rename(tempPath, statePath);
}

function normalizeState(parsed: unknown): StateFile {
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("sidecar state file must contain a JSON object");
  }
  const records = (parsed as { records?: unknown }).records;
  if (records === undefined) {
    return emptyState();
  }
  if (typeof records !== "object" || records === null || Array.isArray(records)) {
    throw new Error("sidecar state file records must be a JSON object");
  }
  for (const bucket of Object.values(records)) {
    if (typeof bucket !== "object" || bucket === null || Array.isArray(bucket)) {
      throw new Error("sidecar state file has an invalid model bucket");
    }
  }
  return { records: records as Record<string, Record<string, AdapterPayload>> };
}

function modelBucket(state: StateFile, modelName: string): Record<string, AdapterPayload> {
  const existing = state.records[modelName];
  if (existing !== undefined) {
    return existing;
  }
  const created: Record<string, AdapterPayload> = {};
  state.records[modelName] = created;
  return created;
}

function deleteRecord(state: StateFile, modelName: string, id: string): boolean {
  const bucket = state.records[modelName];
  if (bucket === undefined || !Object.prototype.hasOwnProperty.call(bucket, id)) {
    return false;
  }
  delete bucket[id];
  if (Object.keys(bucket).length === 0) {
    delete state.records[modelName];
  }
  return true;
}

function emptyState(): StateFile {
  return { records: {} };
}

function jsonSafeCopy(record: AdapterPayload): AdapterPayload {
  return JSON.parse(JSON.stringify(record)) as AdapterPayload;
}

function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

function isExpired(record: AdapterPayload): boolean {
  const exp = record.exp;
  return typeof exp === "number" && exp <= nowSeconds();
}

function isNotFound(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}
