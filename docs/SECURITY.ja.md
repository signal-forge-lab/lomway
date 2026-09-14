# Security

更新日: 2026-09-13

## 1. Trust boundary

```text
ChatGPT
  -> OpenAI Secure MCP Tunnel
  -> Lomway
  -> 明示設定された6つのloopback MCP backend
```

Google Driveは独立connectorとしてこの境界外です。

## 2. Network boundary

- Gateway listenerは `127.0.0.1` 固定。
- v1はLAN / `0.0.0.0` / Tailscale / public bindをreject。
- southbound backend URLもloopback HTTP `/mcp` に限定。
- remote ingressはSecure MCP Tunnelだけ。

## 3. Upstream control plane除去

`mcp-proxy` 0.4.3はconfig確認やdynamic backend登録を含む管理toolを持つ `proxy` MCP backendを生成します。hostは `remove_backend("proxy")` を実行し、削除成功を証明できなければ起動失敗します。

upstream HTTP routerは `/admin/*` も持ちますが、project-owned routerからは公開しません。top-level `/admin/*` にrouteを作らず、nested `/mcp/admin/*` もmiddlewareでupstream dispatch前に拒否します。任意Authorization headerを付けても404になることをnegative E2Eで確認済みです。

結果:

- Gateway admin tokenは不要。
- ChatGPTからbackend追加/削除/config変更不可。
- local healthはadmin APIではなく `/healthz` を利用。

## 4. Secrets

Gateway runtime自体にはsecret不要です。Secure Tunnel setup/runtimeだけが以下を使います。

- `OPENAI_ADMIN_KEY`: one-time remote tunnel create/reuse
- `CONTROL_PLANE_API_KEY`: managed local tunnel runtime

両方とも `%USERPROFILE%\.config\sops\secrets\global.sops.json` からPowerShell process内だけへ読み込みます。利用後はprocess値をrestore/clearし、plaintextをrepositoryやcommand lineへ保存しません。

## 5. Tool surface安全性

- native typed toolを維持しgeneric dispatcherを追加しない。
- Gatewayでbackend schemaを書き換えない。
- dynamic backend registration toolを公開しない。
- retry / hedge / fan-out / failover / cacheを使わずmutation重複を防止。
- production policyでtool argumentを最大1 MiBに制限。
- backend alias/filter/default arg injection等もv1ではreject。

## 6. FastMCP interoperability

UFO / Stealth BrowserはFastMCP公式 `FASTMCP_INCLUDE_FASTMCP_META=false` を使い、framework-private `_fastmcp` metadataを発生元で無効化しました。Gatewayに任意third-party schema sanitizerを追加するより安全です。

## 7. Supply chain

- `mcp-proxy` 0.4.3 exact pin。
- protocol featureを明示指定。
- `Cargo.lock` tracked。
- dependency updateはsource/config/security review + 全regression gate必須。
- productionで `latest` 自動追従しない。

## 8. Public repository gate

public push前にtracked source/docs/history候補を以下でscanします。

- API key / bearer token / private key / secret-like assignment
- email等のpersonal identifier
- real usernameを含むWindows absolute path
- 実machine-local endpoint inventory
- tunnel ID / generated profile / log path
- `.env` / SOPS plaintext / local TOML・JSON / logs / PID・lock / build artifacts

branchをsecrecy boundaryとして扱いません。
