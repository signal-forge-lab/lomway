import { randomBytes } from "node:crypto";
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
        const record = jsonSafeCopy(payload);
        if (typeof expiresIn === "number" && Number.isFinite(expiresIn)) {
          record.exp = nowSeconds() + expiresIn;
        }
        modelBucket(state, modelName)[id] = record;
        await writeState(statePath, state);
      });
    },

    async find(id) {
      return enqueue(statePath, async () => {
        const state = await readState(statePath);
        const record = state.records[modelName]?.[id];
        if (record === undefined) {
          return undefined;
        }
        if (isExpired(record)) {
          deleteRecord(state, modelName, id);
          await writeState(statePath, state);
          return undefined;
        }
        return jsonSafeCopy(record);
      });
    },

    async consume(id) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        const record = state.records[modelName]?.[id];
        if (record === undefined) {
          return;
        }
        record.consumed = nowSeconds();
        await writeState(statePath, state);
      });
    },

    async destroy(id) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        if (deleteRecord(state, modelName, id)) {
          await writeState(statePath, state);
        }
      });
    },

    async revokeByGrantId(grantId) {
      await enqueue(statePath, async () => {
        const state = await readState(statePath);
        let changed = false;
        for (const [model, bucket] of Object.entries(state.records)) {
          for (const [id, record] of Object.entries(bucket)) {
            if (record.grantId === grantId) {
              delete bucket[id];
              changed = true;
            }
          }
          if (Object.keys(bucket).length === 0) {
            delete state.records[model];
          }
        }
        if (changed) {
          await writeState(statePath, state);
        }
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
    return emptyState();
  }
  return normalizeState(JSON.parse(raw));
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
