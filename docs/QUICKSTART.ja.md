# クイックスタート

Updated: 2026-09-13

`lomway` バイナリと1つのTOMLファイルだけで、任意の2つのローカルMCPサーバーを **1つのloopback MCP endpoint** に集約します。この手順では **OpenAI Secure MCP Tunnel / Swibo / SOPS / PowerShell を一切使いません**。core gatewayにこれらは不要です。

English: [QUICKSTART.md](QUICKSTART.md)

## 1. 前提条件

- stable Rust toolchain（`rustup`）。検証の基準はWindowsですが、stable Rustが動く環境であればcoreはbuild・実行できます。
- `http://127.0.0.1:18701/mcp` と `http://127.0.0.1:18702/mcp` のように、loopback上でStreamable HTTP MCP endpointを公開する2つのローカルMCPサーバー。独立して管理されている任意のMCPサーバーでよく、gatewayはbackendプロセスを起動しません。

## 2. ビルド

```powershell
cargo build --locked --release
```

バイナリはWindowsでは `target/release/lomway.exe`（それ以外は `target/release/lomway`）です。以下の例では簡略化して `lomway` と表記します。

## 3. 最小の2-backend構成

`gateway.quickstart.toml` として保存します（パスは任意。`--config` を省略した場合の解決順はセクション7を参照）:

```toml
schema_version = 1

[[backends]]
id = "alpha"
prefix = "alpha_"
url = "http://127.0.0.1:18701/mcp"
required = true
timeout_seconds = 30

[[backends]]
id = "beta"
prefix = "beta_"
url = "http://127.0.0.1:18702/mcp"
required = false
timeout_seconds = 30
```

意味:

- `schema_version = 1` が唯一サポートされる公開schemaバージョンです。
- backend URLは `http://127.0.0.1:<port>/mcp` のみ許可（loopback-only policy）。`${ENV_VAR}` 参照も使え、ロード時に解決されます。
- `alpha` は **required**: 到達できない場合、起動はfail closedします。
- `beta` は **optional**: 到達できない場合、起動はdegradeし、healthyなbackendだけでserveします。
- `[server]` / `[policy]` / `[observability]` は省略しており、保守的な既定値が適用されます（loopbackのport `17777`、hot reload無効、引数上限1 MiB）。schema全体は [CONFIGURATION.ja.md](CONFIGURATION.ja.md) を参照してください。

zero/one/many backendの再配布可能な構成例は `test-fixtures/configs/` にあり、テストスイートで検証されています。

## 4. serveせずに検証

```powershell
lomway check --config gateway.quickstart.toml
```

`check` は非ブロッキングです: parse・policy検証・configレベルのcollision preflightを実行し、ネットワークI/Oはゼロです。`--probe` を付けるとdoctor形式の診断になり、各backendに正確に1回だけprobeし（probeは決してretryしません）、最終tool名を検証します:

```powershell
lomway check --config gateway.quickstart.toml --probe
```

backendに接せず構成を一覧表示するには:

```powershell
lomway list-backends --config gateway.quickstart.toml
```

## 5. serve

```powershell
lomway serve --config gateway.quickstart.toml
```

gatewayが公開するのは正確に次の3つです:

| Endpoint | 用途 |
|---|---|
| `http://127.0.0.1:17777/mcp` | 集約MCP endpoint（Streamable HTTP） |
| `http://127.0.0.1:17777/healthz` | liveness |
| `http://127.0.0.1:17777/readyz` | readiness（required backendは起動必須、optionalのdegradeは報告される） |

`/admin/*` は存在しません: upstreamのadmin経路は削除され `404` を返すため、gateway自身はsecretを必要としません。

## 6. 検証

任意のMCP clientで `/mcp` に接続した場合:

- `tools/list` は `alpha_status` と `beta_status` を返します。最終名は `<prefix><upstream tool名>` です。
- backendのtool descriptionとinput schemaはそのまま通ります。gatewayは書き換えません。
- upstreamの `proxy` control-plane backendはtoolを1つも公開しません（`proxy_*` は0件）。
- timeoutしたmutation呼び出しは正確に1回だけdispatchされ、errorとして表面化します。透過的な再送は行いません。

MCP clientなしでは:

```powershell
curl.exe http://127.0.0.1:17777/healthz
curl.exe http://127.0.0.1:17777/readyz
```

同じ公開schemaを使う2-backendの実E2E（モックMCP backend）は次のコマンドで検証できます:

```powershell
cargo test --locked --test fixtures
```

## 7. 構成パスの解決順

`--config` を省略した場合、バイナリは次の順で構成ファイルを解決します（上位優先）:

1. 明示的な `--config <path>`;
2. `LOMWAY_CONFIG` 環境変数;
3. 作業ディレクトリ相対で存在する最初の既定候補: `config/proxy.local.toml`、次に `gateway.toml`。

## 8. このクイックスタートで使わなかったもの

- **OpenAI Secure MCP Tunnel** — 任意のremote ingress（[INTEGRATIONS.ja.md](INTEGRATIONS.ja.md)）。
- **Swibo** — 任意の外部process supervisor。gatewayはloopback HTTP endpointに接続するだけです。
- **SOPS** — 任意のsecret解決。coreは通常の環境変数のみを読みます。
- **PowerShell** — `scripts/` のラッパーは利便性のためのもので、coreの全モードはバイナリ単体で動きます。

## 次のステップ

- [CONFIGURATION.ja.md](CONFIGURATION.ja.md) — 公開schemaとpolicyの完全なリファレンス。
- [ARCHITECTURE.ja.md](ARCHITECTURE.ja.md) — 責務境界とリクエストフロー。
- [TESTING.ja.md](TESTING.ja.md) — テストスイートと実行方法。
- [INTEGRATIONS.ja.md](INTEGRATIONS.ja.md) — 個別パッケージ化されたオプション統合。
