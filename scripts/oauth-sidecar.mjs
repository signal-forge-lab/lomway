#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import http from "node:http";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sidecarRoot = join(repoRoot, "sidecar-mac");
const home = homedir();
const port = Number(process.env.LOMWAY_OAUTH_PORT || "7677");
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error("LOMWAY_OAUTH_PORT must be a valid TCP port");
}
const issuer = process.env.LOMWAY_OAUTH_ISSUER || "https://mcp-mac.maiteneru.com/";
const resourceUrl = process.env.LOMWAY_OAUTH_RESOURCE_URL || "https://mcp-mac.maiteneru.com/mcp";
const runtimeRoot = process.env.LOMWAY_OAUTH_RUNTIME_ROOT
  || join(home, "Library", "Application Support", "Lomway", "oauth-sidecar");
const stateDir = join(runtimeRoot, "state");
const databasePath = join(stateDir, "oauth.sqlite");
const pidFile = join(runtimeRoot, "sidecar.pid");
const stdoutPath = join(runtimeRoot, "sidecar.log");
const stderrPath = join(runtimeRoot, "sidecar.err.log");
const sopsStore = join(home, ".config", "sops", "secrets", "global.sops.json");
const ageKeyFile = join(home, ".config", "sops", "age", "keys.txt");
const workbridgeDatabase = join(home, ".local", "share", "devspace", "devspace.sqlite");
const healthUrl = `http://127.0.0.1:${port}/healthz`;

function httpOk(url) {
  return new Promise((resolvePromise) => {
    const request = http.get(url, { timeout: 1200 }, (response) => {
      response.resume();
      resolvePromise(Boolean(response.statusCode && response.statusCode >= 200 && response.statusCode < 300));
    });
    request.once("timeout", () => {
      request.destroy();
      resolvePromise(false);
    });
    request.once("error", () => resolvePromise(false));
  });
}

async function waitFor(expected, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await httpOk(healthUrl) === expected) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`OAuth sidecar did not become ${expected ? "ready" : "stopped"} within ${timeoutMs}ms`);
}

function pidFromFile() {
  if (!existsSync(pidFile)) return undefined;
  const pid = Number.parseInt(readFileSync(pidFile, "utf8").trim(), 10);
  if (!Number.isInteger(pid) || pid <= 1) return undefined;
  try {
    process.kill(pid, 0);
    return pid;
  } catch {
    return undefined;
  }
}

function processCommand(pid) {
  return spawnSync("/bin/ps", ["-p", String(pid), "-o", "command="], { encoding: "utf8" }).stdout.trim();
}

function processCwd(pid) {
  const output = spawnSync("/usr/sbin/lsof", ["-a", "-p", String(pid), "-d", "cwd", "-Fn"], {
    encoding: "utf8",
  }).stdout;
  const line = output.split(/\r?\n/).find((value) => value.startsWith("n"));
  return line?.slice(1);
}

function ownsPort(pid, port) {
  const result = spawnSync("/usr/sbin/lsof", ["-tiTCP:" + port, "-sTCP:LISTEN"], { encoding: "utf8" });
  return result.stdout.split(/\s+/).filter(Boolean).includes(String(pid));
}

function verifiedPid() {
  const pid = pidFromFile();
  if (!pid || !ownsPort(pid, port)) return undefined;
  const cwd = processCwd(pid);
  const command = processCommand(pid);
  if (!cwd || realpathSync(cwd) !== realpathSync(sidecarRoot)) return undefined;
  if (!command.includes("node") || !command.includes("src/server.mjs")) return undefined;
  return pid;
}

function decryptSecrets() {
  if (!existsSync(sopsStore)) throw new Error(`Canonical SOPS store not found: ${sopsStore}`);
  if (!existsSync(ageKeyFile)) throw new Error(`SOPS age key not found: ${ageKeyFile}`);
  const result = spawnSync(
    "sops",
    ["-d", "--extract", '["lomway"]["oauth"]', "--output-type", "json", sopsStore],
    {
      encoding: "utf8",
      env: { ...process.env, SOPS_AGE_KEY_FILE: ageKeyFile },
    },
  );
  if (result.status !== 0) throw new Error("Failed to decrypt lomway.oauth from the canonical SOPS store");
  const secrets = JSON.parse(result.stdout);
  if (typeof secrets.owner_credential !== "string" || secrets.owner_credential.length < 16) {
    throw new Error("lomway.oauth.owner_credential is missing");
  }
  if (
    typeof secrets.client_registration_key !== "string"
    || secrets.client_registration_key.length < 32
  ) throw new Error("lomway.oauth.client_registration_key is missing");
  return secrets;
}

function initializeState() {
  mkdirSync(stateDir, { recursive: true, mode: 0o700 });
  chmodSync(stateDir, 0o700);
  if (!existsSync(workbridgeDatabase)) return;
  const result = spawnSync(
    process.execPath,
    ["src/migrate-state.mjs", workbridgeDatabase, databasePath],
    { cwd: sidecarRoot, encoding: "utf8" },
  );
  if (result.status !== 0) throw new Error(String(result.stderr || result.stdout).trim());
  if (result.stdout.trim()) console.log(`oauth-state: ${result.stdout.trim()}`);
}

async function status() {
  const pid = verifiedPid();
  return {
    ready: Boolean(pid && await httpOk(healthUrl)),
    pid: pid ?? null,
    port,
    secretSource: "sops",
    stateDatabase: databasePath,
  };
}

async function start() {
  const current = await status();
  if (current.ready) {
    console.log(`ALREADY_RUNNING pid=${current.pid}`);
    return;
  }
  if (await httpOk(healthUrl)) {
    throw new Error(`Port ${port} is already serving a process not owned by the Lomway OAuth sidecar`);
  }
  if (!existsSync(join(sidecarRoot, "node_modules"))) {
    throw new Error("OAuth sidecar dependencies are missing. Run npm install in sidecar first.");
  }

  const secrets = decryptSecrets();
  initializeState();
  mkdirSync(runtimeRoot, { recursive: true, mode: 0o700 });
  const stdout = openSync(stdoutPath, "a", 0o600);
  const stderr = openSync(stderrPath, "a", 0o600);
  const child = spawn(process.execPath, ["src/server.mjs"], {
    cwd: sidecarRoot,
    detached: true,
    stdio: ["ignore", stdout, stderr],
    env: {
      ...process.env,
      LOMWAY_OAUTH_BIND_HOST: "127.0.0.1",
      LOMWAY_OAUTH_PORT: String(port),
      LOMWAY_OAUTH_ISSUER: issuer,
      LOMWAY_OAUTH_RESOURCE_URL: resourceUrl,
      LOMWAY_OAUTH_OWNER_CREDENTIAL: secrets.owner_credential,
      LOMWAY_OAUTH_CLIENT_REGISTRATION_KEY: secrets.client_registration_key,
      LOMWAY_OAUTH_RUNTIME_DIR: stateDir,
      LOMWAY_OAUTH_DATABASE_PATH: databasePath,
    },
  });
  child.unref();
  closeSync(stdout);
  closeSync(stderr);
  writeFileSync(pidFile, String(child.pid), { mode: 0o600 });
  try {
    await waitFor(true, 15000);
  } catch (error) {
    rmSync(pidFile, { force: true });
    throw error;
  }
  console.log(`STARTED pid=${child.pid}`);
}

async function stop() {
  const pid = verifiedPid();
  if (!pid) {
    rmSync(pidFile, { force: true });
    console.log("NOT_RUNNING");
    return;
  }
  process.kill(pid, "SIGTERM");
  await waitFor(false, 10000);
  rmSync(pidFile, { force: true });
  console.log(`STOPPED pid=${pid}`);
}

const action = process.argv[2] ?? "status";
if (action === "start") await start();
else if (action === "stop") await stop();
else if (action === "status") console.log(JSON.stringify(await status()));
else throw new Error("Usage: oauth-sidecar.mjs start|stop|status");
