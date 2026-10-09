// An isolated browser fixture running the real, built Sidecar implementation.
// Never reads production secrets, ports, OAuth state, or browser profiles.
import { generateKeyPairSync, randomBytes } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createLomwaySidecar } from "../dist/src/server.js";

const runtimeDir = await mkdtemp(join(tmpdir(), "lomway-browser-test-"));
const socket = createServer();
await new Promise((resolve, reject) => {
  socket.once("error", reject);
  socket.listen(0, "127.0.0.1", resolve);
});
const address = socket.address();
const port = address.port;
await new Promise((resolve) => socket.close(resolve));
const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const sidecar = createLomwaySidecar({
  host: "127.0.0.1",
  port,
  issuer: `http://127.0.0.1:${port}`,
  resourceUrl: "https://mcp.example.test/mcp",
  ownerCredential: "synthetic-browser-test-password",
  jwks: { keys: [privateKey.export({ format: "jwk" })] },
  cookieKeys: [randomBytes(32).toString("base64url"), randomBytes(32).toString("base64url")],
  runtimeDir,
  statePath: join(runtimeDir, "state.json"),
});
let closed = false;
async function close() {
  if (closed) return;
  closed = true;
  await sidecar.close();
  await rm(runtimeDir, { recursive: true, force: true });
}
try {
  await sidecar.start();
  process.stdout.write(`BROWSER_FIXTURE_READY:${port}\n`);
  process.on("SIGTERM", () => void close().then(() => process.exit(0)));
  process.on("SIGINT", () => void close().then(() => process.exit(0)));
} catch (error) {
  await close();
  process.stderr.write(error instanceof Error ? error.message + "\n" : "fixture error\n");
  process.exitCode = 1;
}
