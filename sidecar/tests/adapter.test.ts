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

test("removes expired records during lookup and persists the cleanup", async () => {
  await withStateFile(async (statePath) => {
    const adapter = createPersistentAdapter(statePath)("AccessToken");

    await adapter.upsert("expired", { kind: "AccessToken" }, -1);

    assert.equal(await adapter.find("expired"), undefined);
    assert.deepEqual(JSON.parse(await readFile(statePath, "utf8")), { records: {} });
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
