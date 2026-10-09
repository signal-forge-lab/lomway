import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const DEFAULT_DISCOVERY_TOOLS = ["lomway_search_tools", "lomway_describe_tool"];
const TOOL_NAME = /^[A-Za-z0-9_.:-]{1,128}$/;
const CLIENT_DIGEST = /^sha256:[a-f0-9]{64}$/;
const FORBIDDEN_TOOLS = new Set(["proxy/config", "proxy/add_backend"]);

export interface ClientToolPolicy {
  readonly unrestrictedLegacy: boolean;
  readonly allowedTools: string[];
}

export function clientDigest(clientId: string): string {
  return "sha256:" + createHash("sha256").update(clientId).digest("hex");
}

/** Immutable local operator policy. Existing clients are never auto-exempted.
 * Missing policy = discovery only; malformed policy = fail closed.
 */
export async function loadClientToolPolicy(runtimeDir: string, clientId: string): Promise<ClientToolPolicy> {
  let raw: string;
  try {
    raw = await readFile(join(runtimeDir, "client-tool-policy.json"), "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return { unrestrictedLegacy: false, allowedTools: [...DEFAULT_DISCOVERY_TOOLS] };
    }
    throw error;
  }
  const parsed: unknown = JSON.parse(raw);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("Invalid OAuth client tool policy");
  }
  const policy = parsed as Record<string, unknown>;
  if (policy.version !== 1 ||
      !Array.isArray(policy.unrestrictedLegacyClientHashes) ||
      !policy.unrestrictedLegacyClientHashes.every((entry) => typeof entry === "string" && CLIENT_DIGEST.test(entry)) ||
      typeof policy.clientToolAllowlists !== "object" ||
      policy.clientToolAllowlists === null || Array.isArray(policy.clientToolAllowlists)) {
    throw new Error("Invalid OAuth client tool policy schema");
  }
  const allowlists = policy.clientToolAllowlists as Record<string, unknown>;
  if (Object.entries(allowlists).some(([id, tools]) =>
    !CLIENT_DIGEST.test(id) || !Array.isArray(tools) ||
    tools.length > 100 || new Set(tools).size !== tools.length ||
    tools.some((tool) => typeof tool !== "string" ||
      !TOOL_NAME.test(tool) || FORBIDDEN_TOOLS.has(tool))
  )) {
    throw new Error("Invalid OAuth client tool allowlist");
  }
  const digest = clientDigest(clientId);
  const trustedLegacy = (policy.unrestrictedLegacyClientHashes as string[]).includes(digest);
  const explicit = allowlists[digest];
  // Explicit per-client allowlist is always more restrictive than legacy
  // exemption, so an operator can narrow a previously trusted client.
  if (Array.isArray(explicit)) {
    return { unrestrictedLegacy: false, allowedTools: [...explicit] as string[] };
  }
  return trustedLegacy
    ? { unrestrictedLegacy: true, allowedTools: [] }
    : { unrestrictedLegacy: false, allowedTools: [...DEFAULT_DISCOVERY_TOOLS] };
}
