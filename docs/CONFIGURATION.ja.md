# Configuration

更新日: 2026-09-13

## 1. 設定レイヤー

| Layer | 例 | Git |
|---|---|---|
| Public gateway policy | namespace / listen port / timeout / 禁止middleware | tracked |
| Machine-local endpoint inventory | 実backend URL/port | local-only |
| Secure Tunnel credential | `OPENAI_ADMIN_KEY`, `CONTROL_PLANE_API_KEY` | external SOPS only |
| Runtime state | logs / PID / Tunnel profile / health file | ignored / repository外 |

`config/proxy.example.toml` がtracked exampleです（legacyデプロイ形式・環境変数参照のみ）。公開schemaの再配布可能なexampleは `test-fixtures/configs/` にあり、最小の2-backend構成の手順は [QUICKSTART.ja.md](QUICKSTART.ja.md) にあります。`config/proxy.local.toml` はmachine-localでGit対象外です。

## 2. 構成フォーマットとパス優先順

gatewayは2つのディスク形式を受け入れ、自動判定します。

- **公開schema** — トップレベルの `schema_version = 1`。厳格にparseし、未知のkeyは拒否します。
- **legacyデプロイschema** — トップレベルの `[proxy]` テーブル（`mcp-proxy` 構成）。元のデプロイpolicy（1つ以上のbackendを要求し続ける）で検証し、**メモリ内で**移行します。ファイル自体は変更されません。全コマンドがロードしたフォーマットを報告します。

`--config` を省略した場合の構成パス優先順（上位優先）:

1. 明示的な `--config <path>`;
2. `LOMWAY_CONFIG` 環境変数;
3. 作業ディレクトリ相対で存在する最初の既定候補: `config/proxy.local.toml`、次に `gateway.toml`。

backend URL中の `${ENV_VAR}` 参照はロード時に解決され、未解決の変数はロードをloudlyに失敗させます。

## 3. 公開schemaリファレンス（`schema_version = 1`）

```toml
schema_version = 1

[server]
host = "127.0.0.1"                  # 127.0.0.1のみ許可
port = 17777
instructions = "..."                # 任意のMCP instructionsテキスト
shutdown_timeout_seconds = 30

[policy]
allow_non_loopback_listener = false # false固定
allow_non_loopback_backends = false # 既定false。trueでもTailscale HTTPSの厳密な/mcp URLのみ許可
hot_reload = false                  # false固定
max_argument_size_bytes = 1048576   # 1 MiB以下

[observability]
audit = true
log_level = "info"
json_logs = false

[[backends]]
id = "filesystem"                   # 一意・小文字・ambiguous形は不可
prefix = "fs_"                      # namespace契約: <prefix><tool>
url = "http://127.0.0.1:8001/mcp"   # 既定はloopbackのみ。${VAR} 可
required = true                     # 既定true。falseはdegrade-on-outage
timeout_seconds = 30                # 既定30
```

既定値: `[server]` は `127.0.0.1:17777` でlisten。`[policy]` / `[observability]` / `backends` は完全に省略可能です（公開schemaは0 backendを受け入れますが、デプロイゲートは1つ以上のbackendを要求し続けます）。未知のkeyは全レベルで検証に失敗します。

namespace prefix規則: 空でない小文字stem + `_`、backend間で一意、暗黙の正規化なし。reserved prefix（`proxy_`、`lomway_`、旧 `lmg_`）は拒否されます。

southboundのremote backendは明示opt-inです。`allow_non_loopback_backends = true` の場合でも、追加で許可されるのは既定TLS port上の厳密な `https://<machine>.<tailnet>.ts.net/mcp` だけです。userinfo、query、fragment、custom port、任意Internet host、非TLS remote URLは引き続き拒否します。trusted tailnet peer上のTailscale Serve endpointを想定しています。

## 4. CLIリファレンス

バイナリはcoreの全モードを単体で実装します（exit code: `0` 成功 / `1` 失敗 / `2` usage error）:

| Command | 用途 |
|---|---|
| `lomway serve --config <path>` | 起動セマンティクスを検証し、loopback listenerでMCPをserve |
| `lomway check --config <path> [--probe]` | 非ブロッキング検証（parse・policy・configレベルcollision preflight・ネットワークI/Oなし）。`--probe` は各backendに正確に1回接触し最終tool名を検証 |
| `lomway list-backends --config <path>` | backendへ接触せず構成を一覧表示 |
| `lomway migrate --config <path> --output <path>` | legacy構成を公開schemaへ新ファイルとして変換（上書きなし。[MIGRATION.ja.md](MIGRATION.ja.md) を参照） |
| `lomway version` | バージョン・MCP protocol・構成schema情報を表示 |

## 5. legacyデプロイリファレンス

以下はlegacy `[proxy]` 形式における元の6-backend regression baselineの記録です。これは互換性referenceであり、公開製品のbackend数上限ではありません。移行後の公開schemaでも同じ内容がそのまま当てはまります（[MIGRATION.ja.md](MIGRATION.ja.md) を参照）。

公開exampleは以下を参照します。

```text
WORKBRIDGE_MCP_URL
MEMORY_GATEWAY_MCP_URL
MICROSOFT_UFO_MCP_URL
STEALTH_BROWSER_MCP_URL
XMIND_WORKBOARD_MCP_URL
PRAXIOM_MCP_URL
```

既定production policyでは、解決後の全backend URLを `http://127.0.0.1:<port>/mcp` に限定します。公開schemaのデプロイだけは、上記の狭いTailscale HTTPS backend profileへ明示opt-inできます。legacy deployment fileはloopback-onlyのままです。

Gatewayはadmin surfaceを公開しないため、Gateway本体用secretはありません。SOPSを使うのはSecure Tunnel scriptsだけで、one-time作成時に `OPENAI_ADMIN_KEY`、runtime接続に `CONTROL_PLANE_API_KEY` をprocess内へ読み込みます。

## 6. Listener

```toml
[proxy.listen]
host = "127.0.0.1"
port = 17777
```

local公開endpoint:

```text
http://127.0.0.1:17777/mcp
http://127.0.0.1:17777/healthz
http://127.0.0.1:17777/readyz
```

v1にはpublic/admin `/admin/*` APIはありません。

## 7. Stable backend names

```text
workbridge
memory
ufo
browser
xmind
praxiom
```

external prefixになるためpublic MCP contractの一部です。

## 8. Timeout defaults

| Backend | Timeout |
|---|---:|
| Workbridge | 300 s |
| Memory Gateway | 90 s |
| Microsoft UFO | 180 s |
| Stealth Browser | 180 s |
| XMind Workboard | 90 s |
| Praxiom | 180 s |

retry windowではなく1 callの上限です。

## 9. v1で強制するpolicy

起動時に以下をrejectします。

- loopback以外へのlisten
- HTTP以外のloopback backend、および明示Tailscale HTTPS profile外のremote backend
- `_` 以外のnamespace separator
- `hot_reload = true`
- search/discovery exposure
- Gateway内northbound auth
- retry / hedging / tool-call cache
- rate limit / concurrency / circuit-breaker / outlier middleware
- mirror / canary / failover / composite / request coalescing
- backend alias / injected/default args / parameter override / expose-hide filtering / read-only-destructive rewrite
- 1 MiBを超えるargument-size limit

v1をtimeout-onlyの薄いrouting boundaryに固定するためです。

## 10. FastMCP互換

Microsoft UFO / Stealth BrowserのHTTP endpointは以下を設定します。

```text
FASTMCP_INCLUDE_FASTMCP_META=false
```

これはbackend launch設定であり、Gatewayによるschema変換ではありません。

## 11. Commands

coreモードはバイナリ単体で実行します（全CLIリファレンスはセクション4）:

```powershell
target\release\lomway.exe check --config config\proxy.local.toml
target\release\lomway.exe check --config config\proxy.local.toml --probe
target\release\lomway.exe list-backends --config config\proxy.local.toml
target\release\lomway.exe version
```

PowerShellラッパー（任意の利便ラッパー。`integrations/powershell/` を参照）:

config/policy check:

```powershell
pwsh -NoProfile -File scripts/check.ps1
```

build/install:

```powershell
pwsh -NoProfile -File scripts/install.ps1
```

runtime:

```powershell
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/stop.ps1
```

Secure Tunnel（オプション統合 — コアゲートウェイには不要）:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action stop
```

Tunnel profileはrepository外のuser application-data領域に生成します。
