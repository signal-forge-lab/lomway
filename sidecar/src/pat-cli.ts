import { join } from "node:path";
import { issuePat, listPats, patStorePath, revokePat } from "./pat.js";

function option(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  return index < 0 ? undefined : args[index + 1];
}

async function main(): Promise<void> {
  const [action, ...args] = process.argv.slice(2);
  const localAppData = process.env.LOCALAPPDATA;
  const runtimeDir = process.env.LOMWAY_OAUTH_RUNTIME_DIR ??
    (localAppData ? join(localAppData, "Lomway", "oauth-sidecar", "runtime") : undefined);
  if (!runtimeDir) throw new Error("LOMWAY_OAUTH_RUNTIME_DIR is required");
  const path = patStorePath(runtimeDir);
  switch (action) {
    case "issue": {
      const label = option(args, "--label");
      const days = Number(option(args, "--days") ?? "30");
      const audience = option(args, "--audience") ?? "https://mcp.maiteneru.com/mcp";
      const tools = option(args, "--tools")?.split(",").filter(Boolean) ?? [];
      const allTools = args.includes("--all-tools");
      if (!label) throw new Error("--label is required");
      if (allTools && option(args, "--tools") !== undefined) {
        throw new Error("--all-tools cannot be combined with --tools");
      }
      const { token, record } = await issuePat(path, {
        label, days, audience, allowedTools: allTools ? ["*"] : tools,
      });
      // stdout is deliberately secret-bearing for use by a local clipboard wrapper.
      process.stdout.write(JSON.stringify({ token, ...record }) + "\n");
      break;
    }
    case "list":
      process.stdout.write(JSON.stringify(await listPats(path), null, 2) + "\n");
      break;
    case "revoke": {
      const id = option(args, "--id");
      if (!id) throw new Error("--id is required");
      if (!await revokePat(path, id)) throw new Error("PAT not found or already revoked");
      process.stdout.write(JSON.stringify({ revoked: true, id }) + "\n");
      break;
    }
    default:
      throw new Error("usage: pat-cli [issue --label NAME --days 30 (--tools tool1,tool2 | --all-tools) | list | revoke --id ID]");
  }
}

main().catch((error: unknown) => {
  process.stderr.write((error instanceof Error ? error.message : "PAT operation failed") + "\n");
  process.exitCode = 1;
});
