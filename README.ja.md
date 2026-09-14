# Lomway

独立して動作する **0..N 個のローカルMCP** を1つのloopback MCP endpointへ集約するGatewayです。remote accessは任意で **1本のOpenAI Secure MCP Tunnel** に集約できます。

> Status: 2026-09-13 時点で実装・実機検証済みです。最終release判定は [docs/FINAL_REVIEW.ja.md](docs/FINAL_REVIEW.ja.md) に記録します。

English: [README.md](README.md) · 初めての方は [クイックスタート](docs/QUICKSTART.ja.md) からどうぞ。

## 目的

```text
ChatGPT
|- Google Drive                         # 独立connector。集約対象外
`- Local MCP
   `- OpenAI Secure MCP Tunnel
      `- 127.0.0.1:17777/mcp
         `- Lomway
            |- alpha_*                 -> 任意のlocal MCP backend A
            |- beta_*                  -> 任意のlocal MCP backend B
            `- ...                     -> 追加backend
```

Google Driveは専用connectorの特性を維持するためGatewayへ入れません。

## v1 実装方針

- MCP proxyを再実装せず、`joshrotenberg/mcp-proxy` **0.4.3** をlibraryとして利用します。
- `default-features = false` とし、`protocol-2026-07-28` featureを明示的に有効化します。`Cargo.lock` で `tower-mcp` 0.18.2 を含む解決済み依存を固定します。
- upstreamの管理MCP backend `proxy` をserve前に削除し、client-visibleな `proxy_*` 管理toolを0件にします。
- project所有のnorthbound routerから公開するのは `/mcp`、`/healthz`、`/readyz` だけです。upstream `/admin/*` は公開せず、`/admin/*` と `/mcp/admin/*` は404になります。
- v1のlistenは `127.0.0.1:17777` 固定です。
- tool名は `_` separatorで `<backend>_<tool>` にnamespace化します。
- backend本来のschemaを維持し、Gateway側で意味変換やgeneric dispatcher化を行いません。
- project固有policyを起動時にのみ検証するため、v1ではhot reloadを無効化します。
- automatic retry / hedging / fan-out / failover / tool-call cacheを無効化します。timeoutしたmutationを自動再送しません。
- backend process lifecycleはGatewayへ持ち込みません。Start/Stop/RestartはSwiboが担当し、Gatewayはloopback HTTP MCPへ接続するだけです。
- coreモードはすべてバイナリ単体で実装します: `serve` / `check`（`--probe`）/ `list-backends` / `migrate` / `version`。実行可能なexit codeを持ち、PowerShellは不要です。
- Gateway本体にはsecret不要です。Secure Tunnel credentialは外部の正本SOPS storeからprocess内だけへ読み込み、repositoryには保存しません。

## 実機regression検証済み状態

公開製品はbackend inventoryやtool数を固定しません。元の6-backend実機regression baselineは **183 tools**、現在のreview済みworkstationでは追加のChrome DevTools 30 toolsを含めて **213 tools** です。

| Namespace | Tools |
|---|---:|
| `workbridge_` | 12 |
| `memory_` | 32 |
| `ufo_` | 19 |
| `browser_` | 97 |
| `xmind_` | 21 |
| `praxiom_` | 2 |
| `chrome_` | 30 |
| **現在合計** | **213** |

確認済み事項:

- baseline 6 backendすべてが同じ `tower-mcp` client stackでinitialize / tools list成功。
- 現在構成の7 namespace（任意追加の `chrome_` を含む）すべてでGateway経由の代表call成功。
- `proxy_*` は0件。
- 同名toolはnamespaceで衝突回避。
- 起動時に1 backendが失敗しても、別backendがhealthyなら失敗backendをskipして起動可能。
- 初期構築時に全backendが失敗するとupstream proxyを構築できないため、Gatewayはfail closedで起動失敗。
- mutating mock toolを強制timeoutしてもbackend受信回数は正確に1回。
- Lomway用OpenAI Secure MCP Tunnel 1本がREADYになり、本MCP endpointへのprobe / initialize成功。
- Swiboで `READY -> restart -> READY -> stop -> STOPPED -> start -> READY` を実地確認。

Microsoft UFO / Stealth BrowserのHTTP集約endpointでは、FastMCP公式設定 `FASTMCP_INCLUDE_FASTMCP_META=false` を使ってtool metadataを標準互換にしています。Gateway側でschema変換は行いません。

## Repository構成

```text
lomway/
|- Cargo.toml
|- Cargo.lock
|- LICENSE
|- src/
|- config/
|  `- proxy.example.toml
|- scripts/
|  |- install.ps1
|  |- check.ps1
|  |- start.ps1
|  |- status.ps1
|  |- stop.ps1
|  |- common.ps1
|  `- integration-smoke.ps1
|- integrations/
|  |- openai-secure-tunnel/
|  |- swibo/
|  |- sops/
|  `- powershell/
|- test-fixtures/
|- tests/
|- docs/
|- AGENTS.md
|- .gitignore
|- README.md
`- README.ja.md
```

`config/proxy.local.toml`、runtime state、logs、PID、生成Secure Tunnel profileはmachine-localで、Git対象外またはrepository外に置きます。

## ローカル運用

coreモードはバイナリ単体で実行します — PowerShellは不要です:

```powershell
cargo build --locked --release
target\release\lomway.exe check --config <path-to-config>
target\release\lomway.exe serve --config <path-to-config>
```

任意のPowerShellラッパーも利用できます:

```powershell
pwsh -NoProfile -File scripts/install.ps1
pwsh -NoProfile -File scripts/check.ps1
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/integration-smoke.ps1
```

Secure MCP Tunnelワークフローはオプション統合として専用ディレクトリにあります:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
```

登録後の通常lifecycleはSwiboの `lomway` targetから操作します。workstation固有のtarget定義はpublic example registryへcommitしません。

## ドキュメント

- [クイックスタート](docs/QUICKSTART.ja.md) — 統合なしの任意2-backend構成
- [要件](docs/REQUIREMENTS.ja.md)
- [Architecture](docs/ARCHITECTURE.ja.md)
- [Security](docs/SECURITY.ja.md)
- [脅威モデル](docs/THREAT_MODEL.ja.md)
- [Configuration](docs/CONFIGURATION.ja.md)
- [Testing](docs/TESTING.ja.md)
- [Operations](docs/OPERATIONS.ja.md)
- [オプション統合](docs/INTEGRATIONS.ja.md)
- [移行とロールバック](docs/MIGRATION.ja.md)
- [Implementation plan / 完了記録](docs/IMPLEMENTATION_PLAN.ja.md)
- [公開・汎用化設計](docs/PUBLIC_GENERALIZATION_DESIGN.ja.md)
- [公開・汎用化タスク分解](docs/PUBLIC_GENERALIZATION_TASKS.ja.md)
- [公開・汎用化 task manifest](docs/PUBLIC_GENERALIZATION_TASKS.yaml)
- [リリースチェックリスト](docs/RELEASE.ja.md)
- [Changelog](docs/CHANGELOG.md)
- [Final review](docs/FINAL_REVIEW.ja.md)
- [ADR](docs/adr/)

## 主な参照

- MCP 2026-07-28: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- OpenAI custom MCP / Secure MCP Tunnel: https://help.openai.com/en/articles/12584461
- mcp-proxy: https://github.com/joshrotenberg/mcp-proxy
- mcp-proxy v0.4.3: https://github.com/joshrotenberg/mcp-proxy/releases/tag/v0.4.3
