import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { createPersistentAdapter } from "../src/adapter.js";

async function withStateFile(run: (statePath: string) => Promise<void>): Promise<void> {
  const directory = await mkdtemp(join(tmpdir(), "lomway-oauth-adapter-"));
  try {
    await run(join(directory, "state.json"));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

test("persists namespaced model records across adapter instances", async () => {
  await withStateFile(async (statePath) => {
    const accessTokens = createPersistentAdapter(statePath)("AccessToken");
    const refreshTokens = createPersistentAdapter(statePath)("RefreshToken");
    const payload = { kind: "AccessToken", accountId: "owner", grantId: "grant-1" };

    await accessTokens.upsert("same-id", payload);
    await refreshTokens.upsert("same-id", { kind: "RefreshToken", grantId: "grant-2" });

    const restartedAccessTokens = createPersistentAdapter(statePath)("AccessToken");
    assert.deepEqual(await restartedAccessTokens.find("same-id"), payload);

    await restartedAccessTokens.destroy("same-id");
    assert.equal(await restartedAccessTokens.find("same-id"), undefined);
    assert.deepEqual(await refreshTokens.find("same-id"), {
      kind: "RefreshToken",
      grantId: "grant-2",
    });
  });
});

test("does not persist bearer credential IDs and transparently migrates legacy state", async () => {
  await withStateFile(async (statePath) => {
    const adapter = createPersistentAdapter(statePath)("RefreshToken");
    const secret = "synthetic-super-secret-refresh-credential";
    const payload = { kind: "RefreshToken", jti: secret, grantId: "grant-1" };
    await adapter.upsert(secret, payload);
    const raw = await readFile(statePath, "utf8");
    assert.ok(!raw.includes(secret), "new bearer must not appear in state JSON");
    assert.deepEqual(await adapter.find(secret), payload);

    // Simulate an existing pre-migration state without using live credentials.
    const { writeFile } = await import("node:fs/promises");
    await writeFile(statePath, JSON.stringify({
      records: { RefreshToken: { [secret]: payload } },
    }));
    const migrated = await adapter.find(secret);
    assert.deepEqual(migrated, payload);
    const migratedRaw = await readFile(statePath, "utf8");
    assert.ok(!migratedRaw.includes(secret), "legacy plaintext must be removed");
  });
});

test("legacy refresh tokens become session-independent only while their session and grant remain valid", async () => {
  await withStateFile(async (statePath) => {
    const factory = createPersistentAdapter(statePath);
    const sessions = factory("Session");
    const grants = factory("Grant");
    const refresh = factory("RefreshToken");
    await grants.upsert("grant-1", {
      kind: "Grant", accountId: "owner", clientId: "client-1",
    }, 3600);
    await sessions.upsert("session-1", {
      kind: "Session", uid: "session-uid-1", accountId: "owner",
      authorizations: { "client-1": { grantId: "grant-1" } },
    }, 3600);
    const legacy = {
      kind: "RefreshToken", grantId: "grant-1", clientId: "client-1",
      accountId: "owner", sessionUid: "session-uid-1", expiresWithSession: true,
    };
    await refresh.upsert("valid-legacy-token", legacy, 3600);
    const migrated = await refresh.find("valid-legacy-token");
    assert.equal(migrated?.expiresWithSession, false);
    // Removing the session afterwards must not invalidate that token.
    await sessions.destroy("session-1");
    assert.equal((await refresh.find("valid-legacy-token"))?.expiresWithSession, false);
    await refresh.upsert("orphan-legacy-token", legacy, 3600);
    assert.equal((await refresh.find("orphan-legacy-token"))?.expiresWithSession, true);
  });
});

test("removes expired records during lookup and persists the cleanup", async () => {
  await withStateFile(async (statePath) => {
    const adapter = createPersistentAdapter(statePath)("AccessToken");

    await adapter.upsert("expired", { kind: "AccessToken" }, -1);

    assert.equal(await adapter.find("expired"), undefined);
    assert.deepEqual(JSON.parse(await readFile(statePath, "utf8")), { records: {} });
  });
});

test("does not silently reset an empty or invalid existing credential database", async () => {
  await withStateFile(async (statePath) => {
    const { writeFile } = await import("node:fs/promises");
    const adapter = createPersistentAdapter(statePath)("AccessToken");
    for (const invalid of ["", "{broken", '{"records":{"RefreshToken":[]}}']) {
      await writeFile(statePath, invalid);
      await assert.rejects(adapter.find("synthetic"), /state file|JSON/);
    }
  });
});

test("refresh token issuance extends its parent grant to the refresh expiry", async () => {
  await withStateFile(async (statePath) => {
    const grants = createPersistentAdapter(statePath)("Grant");
    const refreshTokens = createPersistentAdapter(statePath)("RefreshToken");
    const before = Math.floor(Date.now() / 1000);

    await grants.upsert("grant-1", { kind: "Grant" }, 10);
    await refreshTokens.upsert(
      "refresh-1",
      { kind: "RefreshToken", grantId: "grant-1" },
      120,
    );

    const grant = await grants.find("grant-1");
    assert.equal(typeof grant?.exp, "number");
    assert.ok((grant?.exp as number) >= before + 119);
  });
});

test("consume marks a record without exposing or logging its token value", async () => {
  await withStateFile(async (statePath) => {
    const adapter = createPersistentAdapter(statePath)("AuthorizationCode");

    await adapter.upsert("one-time-code", { kind: "AuthorizationCode" });
    await adapter.consume("one-time-code");

    const stored = await adapter.find("one-time-code");
    assert.equal(typeof stored?.consumed, "number");
  });
});

test("concurrent refresh consumption has one winner and revokes the entire family", async () => {
  await withStateFile(async (statePath) => {
    const adapter = createPersistentAdapter(statePath)("RefreshToken");
    const grants = createPersistentAdapter(statePath)("Grant");
    await grants.upsert("g1", { kind: "Grant", accountId: "owner", clientId: "client1" }, 600);
    await adapter.upsert("old-refresh", { kind: "RefreshToken", grantId: "g1" }, 600);
    // Two callers may both have read a pre-consumption snapshot.
    const snapshots = await Promise.all([adapter.find("old-refresh"), adapter.find("old-refresh")]);
    assert.equal(snapshots.length, 2);
    assert.ok(snapshots.every((s) => s && s.consumed === undefined));
    const results = await Promise.allSettled([adapter.consume("old-refresh"), adapter.consume("old-refresh")]);
    assert.equal(results.filter((r) => r.status === "fulfilled").length, 1);
    assert.equal(results.filter((r) => r.status === "rejected").length, 1);
    assert.equal(await adapter.find("old-refresh"), undefined);
    assert.equal(await grants.find("g1"), undefined);
    await assert.rejects(adapter.upsert("rotated-refresh", { kind: "RefreshToken", grantId: "g1" }, 600));
  });
});

test("revokeByGrantId removes matching records across model adapters", async () => {
  await withStateFile(async (statePath) => {
    const accessTokens = createPersistentAdapter(statePath)("AccessToken");
    const refreshTokens = createPersistentAdapter(statePath)("RefreshToken");

    await accessTokens.upsert("access", { kind: "AccessToken", grantId: "grant-1" });
    await refreshTokens.upsert("refresh", { kind: "RefreshToken", grantId: "grant-1" });
    await accessTokens.upsert("unrelated", { kind: "AccessToken", grantId: "grant-2" });

    await accessTokens.revokeByGrantId("grant-1");

    assert.equal(await accessTokens.find("access"), undefined);
    assert.equal(await refreshTokens.find("refresh"), undefined);
    assert.deepEqual(await accessTokens.find("unrelated"), {
      kind: "AccessToken",
      grantId: "grant-2",
    });
  });
});

test("findByUserCode and findByUid use their model namespace", async () => {
  await withStateFile(async (statePath) => {
    const deviceCodes = createPersistentAdapter(statePath)("DeviceCode");
    const sessions = createPersistentAdapter(statePath)("Session");

    await deviceCodes.upsert("device", { kind: "DeviceCode", userCode: "user-code" });
    await sessions.upsert("session", { kind: "Session", uid: "session-uid" });

    assert.deepEqual(await deviceCodes.findByUserCode("user-code"), {
      kind: "DeviceCode",
      userCode: "user-code",
    });
    assert.deepEqual(await sessions.findByUid("session-uid"), {
      kind: "Session",
      uid: "session-uid",
    });
    assert.equal(await sessions.findByUid("user-code"), undefined);
  });
});

test("DCR client registration is capped and does not evict existing clients", async () => {
  await withStateFile(async (statePath) => {
    const clients = createPersistentAdapter(statePath)("Client");
    for (let index = 0; index < 128; index++) {
      await clients.upsert("client-" + index, { kind: "Client" });
    }
    await assert.rejects(
      clients.upsert("client-over-limit", { kind: "Client" }),
      /registration capacity reached/,
    );
    assert.ok(await clients.find("client-0"));
    // Updating existing metadata does not consume a registration slot.
    await clients.upsert("client-0", { kind: "Client", clientName: "existing" });
    assert.equal((await clients.find("client-0"))?.clientName, "existing");
  });
});

test("expired never-approved DCR clients are reclaimed without deleting approved clients", async () => {
  await withStateFile(async (statePath) => {
    const clients = createPersistentAdapter(statePath)("Client");
    const grants = createPersistentAdapter(statePath)("Grant");
    await clients.upsert("unapproved", { kind: "Client" });
    await clients.upsert("approved", { kind: "Client" });
    await grants.upsert("grant", { kind: "Grant", clientId: "approved" }, 3600);
    const state = JSON.parse(await readFile(statePath, "utf8"));
    for (const record of Object.values(state.records.LomwayClientRegistered) as Array<{createdAt: number}>) {
      record.createdAt -= 49 * 3600;
    }
    const { writeFile } = await import("node:fs/promises");
    await writeFile(statePath, JSON.stringify(state));
    await clients.upsert("new", { kind: "Client" });
    assert.equal(await clients.find("unapproved"), undefined);
    assert.ok(await clients.find("approved"));
    assert.ok(await clients.find("new"));
  });
});

test("GC prunes only safe expired records and retains refresh replay evidence", async () => {
  await withStateFile(async (statePath) => {
    const factory = createPersistentAdapter(statePath);
    await factory("AccessToken").upsert("expired-at", { kind: "AccessToken" }, -600);
    await factory("AuthorizationCode").upsert("expired-code", { kind: "AuthorizationCode" }, -600);
    await factory("ReplayDetection").upsert("expired-replay", { kind: "ReplayDetection" }, -600);
    await factory("Grant").upsert("grant", { kind: "Grant" }, 3600);
    await factory("RefreshToken").upsert(
      "consumed-refresh", { kind: "RefreshToken", grantId: "grant", consumed: 12345 }, -600,
    );
    // Any adapter read triggers the bounded sweep.
    await factory("Client").find("missing");
    const data = JSON.parse(await readFile(statePath, "utf8"));
    assert.equal(data.records.AccessToken, undefined);
    assert.equal(data.records.AuthorizationCode, undefined);
    assert.equal(data.records.ReplayDetection, undefined);
    assert.equal(Object.keys(data.records.RefreshToken).length, 1);
    assert.equal(Object.keys(data.records.Grant).length, 1);
  });
});
