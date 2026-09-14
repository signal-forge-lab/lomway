# Portable test fixtures

Lomway regression suite（`tests/fixtures.rs`）向けの再配布可能なpublic設定fixtureです。machine-localでgitignore対象の `config/proxy.local.toml` の代わりを務め、private deploymentなしのクリーンなmachineで `cargo test --locked` がregressionを再現できるようにします。

## 一覧

| Fixture | backend数 | 検証内容 |
| --- | --- | --- |
| `configs/zero-backends.toml` | 0 | public schemaでは空のbackend listが有効。一方でdeployment gateは最低1 backendを要求し続ける（安全性の後退なし） |
| `configs/one-backend.toml` | 1 | 単一のrequired backendがload/validate/migrate/serveできる |
| `configs/many-backends.toml` | 3（required 2 + optional 1） | mixed population。到達不能なoptional backendはstartupを失敗させずdegradeする |

## 保証

- sample endpointはloopback限定（`http://127.0.0.1:<port>/mcp`）。sample backend用port 18701-18703とlistener用port 18790-18792は、production defaultの17777を避けたdocumentation用portです。自分のendpointや `${VAR}` 環境変数参照に差し替えて利用できます。
- private service名、credential、machine固有pathを含まないため、public repositoryで再配布しても安全です。
- すべてのfixtureは厳格なpublic schema（unknown keyは拒否）でparseされ、`lomway::config::load_gateway_config` を通してend to endで検証されます。

## 使い方

```text
cargo test --locked
cargo run --locked -- --config test-fixtures/configs/many-backends.toml --check
```

6 backendのprivate deployment regressionは別管理で、defaultのCIではskipされます:

```text
cargo test --locked --test real_backends -- --ignored
```

このsuiteはmachine-localのbackend群が稼働していること、およびgitignore対象の `config/proxy.local.toml` のみを読み込むことを前提にします。
