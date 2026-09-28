#!/usr/bin/env node
import { scryptSync } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const home = homedir();
const authPath = process.env.DEVSPACE_AUTH_PATH || join(home, ".devspace", "auth.json");
const storePath = process.env.LOMWAY_SOPS_STORE
  || join(home, ".config", "sops", "secrets", "global.sops.json");
const ageKeyFile = process.env.SOPS_AGE_KEY_FILE
  || join(home, ".config", "sops", "age", "keys.txt");

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    ...options,
    env: {
      ...process.env,
      SOPS_AGE_KEY_FILE: ageKeyFile,
      ...(options.env ?? {}),
    },
  });
  if (result.status !== 0) {
    throw new Error(`${command} failed: ${String(result.stderr || result.stdout).trim()}`);
  }
  return result.stdout;
}

if (!existsSync(authPath)) throw new Error(`Workbridge auth file not found: ${authPath}`);
for (const command of ["sops", "age-keygen"]) {
  const check = spawnSync("/usr/bin/env", ["which", command], { encoding: "utf8" });
  if (check.status !== 0) throw new Error(`${command} is required`);
}

mkdirSync(dirname(ageKeyFile), { recursive: true, mode: 0o700 });
mkdirSync(dirname(storePath), { recursive: true, mode: 0o700 });
if (!existsSync(ageKeyFile)) run("age-keygen", ["-o", ageKeyFile]);
chmodSync(ageKeyFile, 0o600);
const publicKey = run("age-keygen", ["-y", ageKeyFile]).trim();

const auth = JSON.parse(readFileSync(authPath, "utf8"));
if (typeof auth.ownerToken !== "string" || auth.ownerToken.length < 16) {
  throw new Error("Workbridge ownerToken is missing or invalid");
}
const registrationKey = typeof auth.clientRegistrationKey === "string" && auth.clientRegistrationKey.length >= 32
  ? auth.clientRegistrationKey
  : scryptSync(
    auth.ownerToken,
    "devspace-oauth-client-registration-v1",
    32,
    { N: 16_384, r: 8, p: 1 },
  ).toString("base64url");

let document = {};
if (existsSync(storePath)) {
  document = JSON.parse(run("sops", ["-d", "--output-type", "json", storePath]));
}
document.lomway ??= {};
document.lomway.oauth ??= {};
document.lomway.oauth.owner_credential = auth.ownerToken;
document.lomway.oauth.client_registration_key = registrationKey;

const encrypted = run(
  "sops",
  ["--encrypt", "--age", publicKey, "--input-type", "json", "--output-type", "json", "/dev/stdin"],
  { input: JSON.stringify(document) },
);
const temporaryPath = `${storePath}.tmp-${process.pid}`;
try {
  writeFileSync(temporaryPath, encrypted, { mode: 0o600 });
  renameSync(temporaryPath, storePath);
  chmodSync(storePath, 0o600);
} finally {
  rmSync(temporaryPath, { force: true });
}

const verified = JSON.parse(
  run("sops", ["-d", "--extract", '["lomway"]["oauth"]', "--output-type", "json", storePath]),
);
if (
  typeof verified.owner_credential !== "string"
  || verified.owner_credential.length < 16
  || typeof verified.client_registration_key !== "string"
  || verified.client_registration_key.length < 32
) throw new Error("Lomway OAuth secret verification failed");

console.log(JSON.stringify({
  ok: true,
  store: storePath,
  ownerCredentialPresent: true,
  clientRegistrationKeyPresent: true,
}));
