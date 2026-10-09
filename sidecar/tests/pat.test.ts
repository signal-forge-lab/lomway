import assert from "node:assert/strict";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { issuePat, listPats, patStorePath, revokePat, verifyPat } from "../src/pat.js";

const audience = "https://mcp.maiteneru.com/mcp";

test("issue, verify, list and revoke scoped PAT without storing plaintext", async () => {
  const dir = await mkdtemp(join(tmpdir(), "lomway-pat-"));
  const path = patStorePath(dir);
  try {
    const { token, record } = await issuePat(path, {
      label: "external-ai", days: 30, audience, allowedTools: ["lomway_search_tools"],
    });
    assert.match(token, /^lompat_[A-Za-z0-9_-]{43}$/);
    const raw = await readFile(path, "utf8");
    assert.ok(!raw.includes(token));
    assert.deepEqual((await listPats(path))[0]?.allowedTools, ["lomway_search_tools"]);
    assert.equal((await verifyPat(path, token, audience))?.id, record.id);
    assert.equal(await verifyPat(path, token, "https://other.example/mcp"), undefined);
    assert.equal(await verifyPat(path, token + "x", audience), undefined);
    assert.equal(await revokePat(path, record.id), true);
    assert.equal(await verifyPat(path, token, audience), undefined);
    assert.equal(await revokePat(path, record.id), false);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("refuses missing tool restrictions and admin tools", async () => {
  const dir = await mkdtemp(join(tmpdir(), "lomway-pat-"));
  const path = patStorePath(dir);
  try {
    for (const tools of [[], ["proxy/config"]]) {
      await assert.rejects(issuePat(path, {
        label: "external-ai", days: 30, audience, allowedTools: tools,
      }));
    }
    await assert.rejects(issuePat(path, {
      label: "external-ai", days: 91, audience, allowedTools: ["safe_tool"],
    }));
    const { token } = await issuePat(path, {
      label: "deferred-ai", days: 30, audience,
      allowedTools: ["lomway_call_tool", "chrome_devtools_list_pages"],
    });
    assert.ok(await verifyPat(path, token, audience));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("new issuance reclaims revoked probe credentials but keeps active PATs", async () => {
  const dir = await mkdtemp(join(tmpdir(), "lomway-pat-"));
  const path = patStorePath(dir);
  try {
    const old = await issuePat(path, {
      label: "temporary", days: 1, audience, allowedTools: ["lomway_search_tools"],
    });
    const active = await issuePat(path, {
      label: "active", days: 7, audience, allowedTools: ["lomway_search_tools"],
    });
    assert.equal(await revokePat(path, old.record.id), true);
    await issuePat(path, {
      label: "next", days: 1, audience, allowedTools: ["lomway_search_tools"],
    });
    assert.equal((await listPats(path)).some((record) => record.id === old.record.id), false);
    assert.ok(await verifyPat(path, active.token, audience));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("explicit full-access PAT is represented by the single wildcard permission", async () => {
  const dir = await mkdtemp(join(tmpdir(), "lomway-pat-"));
  const path = patStorePath(dir);
  try {
    const { token } = await issuePat(path, {
      label: "owner-full-access", days: 1, audience, allowedTools: ["*"],
    });
    assert.deepEqual((await verifyPat(path, token, audience))?.allowedTools, ["*"]);
    await assert.rejects(issuePat(path, {
      label: "invalid-mixed", days: 1, audience,
      allowedTools: ["*", "lomway_search_tools"],
    }));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
