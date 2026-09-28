import Database from "better-sqlite3";

import { openOAuthDatabase } from "./store.mjs";

const [sourcePath, targetPath] = process.argv.slice(2);
if (!sourcePath || !targetPath) {
  throw new Error("Usage: node src/migrate-state.mjs <source-db> <target-db>");
}

const target = openOAuthDatabase(targetPath);
const existing = target.prepare(`
  select
    (select count(*) from oauth_clients) as clients,
    (select count(*) from oauth_access_tokens) as access_tokens,
    (select count(*) from oauth_refresh_tokens) as refresh_tokens
`).get();

if (existing.clients || existing.access_tokens || existing.refresh_tokens) {
  console.log(JSON.stringify({ migrated: false, reason: "target-not-empty", ...existing }));
  target.close();
  process.exit(0);
}

const source = new Database(sourcePath, { readonly: true, fileMustExist: true });
const copy = target.transaction(() => {
  for (const row of source.prepare("select client_id,client_json,issued_at from oauth_clients").iterate()) {
    target.prepare(
      "insert into oauth_clients(client_id,client_json,issued_at) values(?,?,?)",
    ).run(row.client_id, row.client_json, row.issued_at);
  }
  for (const row of source.prepare(
    "select token_hash,client_id,scopes_json,expires_at,resource from oauth_access_tokens",
  ).iterate()) {
    target.prepare(
      "insert into oauth_access_tokens(token_hash,client_id,scopes_json,expires_at,resource) values(?,?,?,?,?)",
    ).run(row.token_hash, row.client_id, row.scopes_json, row.expires_at, row.resource);
  }
  for (const row of source.prepare(
    "select token_hash,client_id,scopes_json,expires_at,resource from oauth_refresh_tokens",
  ).iterate()) {
    target.prepare(
      "insert into oauth_refresh_tokens(token_hash,client_id,scopes_json,expires_at,resource) values(?,?,?,?,?)",
    ).run(row.token_hash, row.client_id, row.scopes_json, row.expires_at, row.resource);
  }
});
copy.immediate();
source.close();

const result = target.prepare(`
  select
    (select count(*) from oauth_clients) as clients,
    (select count(*) from oauth_access_tokens) as access_tokens,
    (select count(*) from oauth_refresh_tokens) as refresh_tokens
`).get();
target.close();
console.log(JSON.stringify({ migrated: true, ...result }));
