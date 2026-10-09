import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { clientDigest, loadClientToolPolicy } from "../src/client-tools.js";

test("unknown OAuth clients are discovery-only, never implicitly unrestricted", async () => {
  const directory = await mkdtemp(join(tmpdir(), "lomway-client-tool-policy-"));
  try {
    const fallback = await loadClientToolPolicy(directory, "new-client");
    assert.equal(fallback.unrestrictedLegacy, false);
    assert.deepEqual(fallback.allowedTools, ["lomway_search_tools", "lomway_describe_tool"]);
    const legacyId = clientDigest("existing-client");
    await writeFile(join(directory, "client-tool-policy.json"), JSON.stringify({
      version: 1,
      unrestrictedLegacyClientHashes: [legacyId],
      clientToolAllowlists: {},
    }));
    assert.equal((await loadClientToolPolicy(directory, "existing-client")).unrestrictedLegacy, true);
    assert.equal((await loadClientToolPolicy(directory, "new-client")).unrestrictedLegacy, false);
    await writeFile(join(directory, "client-tool-policy.json"), JSON.stringify({
      version: 1,
      unrestrictedLegacyClientHashes: [legacyId],
      clientToolAllowlists: { [legacyId]: ["lomway_search_tools"] },
    }));
    const narrowed = await loadClientToolPolicy(directory, "existing-client");
    assert.equal(narrowed.unrestrictedLegacy, false);
    assert.deepEqual(narrowed.allowedTools, ["lomway_search_tools"]);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("malformed and admin tool rules fail closed", async () => {
  const directory = await mkdtemp(join(tmpdir(), "lomway-client-tool-policy-"));
  try {
    for (const bad of [
      "{",
      "{}",
      JSON.stringify({ version: 1, unrestrictedLegacyClientHashes: [], clientToolAllowlists: {
        [clientDigest("client")]: ["proxy/config"],
      } }),
    ]) {
      await writeFile(join(directory, "client-tool-policy.json"), bad);
      await assert.rejects(loadClientToolPolicy(directory, "client"));
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
