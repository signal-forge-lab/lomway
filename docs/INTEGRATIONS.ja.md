# オプション統合

Updated: 2026-09-13

Lomwayのcoreはthinなloopback集約境界です。**OpenAI credential・Swibo・SOPS store・PowerShellが一切なくても**build・テスト・検証・serveができます。ローカル固有のワークフローはすべて `integrations/` 配下の個別のオプション統合としてパッケージ化され、それぞれ英語版と日本語版のドキュメントを持ちます。`src/` と `Cargo.toml` はこれらを一切import・参照しません。

English: [INTEGRATIONS.md](INTEGRATIONS.md)

## ガイド一覧

| 統合 | パス | 用途 |
|---|---|---|
| OpenAI Secure MCP Tunnel | [`integrations/openai-secure-tunnel/`](../integrations/openai-secure-tunnel/README.ja.md) ([en](../integrations/openai-secure-tunnel/README.md)) | remote ingress: loopback gatewayをSecure MCP Tunnel経由でChatGPTに公開します。tunnel credentialはprocess内だけに読み込み、client操作は1回きり（自動retryなし）。 |
| Swibo | [`integrations/swibo/`](../integrations/swibo/README.ja.md) ([en](../integrations/swibo/README.md)) | 外部process監督。gateway・tunnel・backendのstart/stop/restartを宣言的に担当します。テンプレートはplaceholderのみを含みます。 |
| SOPS | [`integrations/sops/`](../integrations/sops/README.ja.md) ([en](../integrations/sops/README.md)) | 正本SOPS storeから指定されたsecretを起動時にprocess環境変数へ解決します。secret値がrepositoryやgatewayのファイルに入ることはありません。 |
| PowerShellラッパー | [`integrations/powershell/`](../integrations/powershell/README.ja.md) ([en](../integrations/powershell/README.md)) | core CLIと公開HTTP endpointを包む `scripts/` のthinな利便ラッパー。policy・routing・namespaceロジックは複製しません。 |

## coreを独立させ続ける理由

- coreは通常のprocess環境変数と構成ファイルだけを読みます。secret解決はSOPS統合側の起動時の関心事です。
- gatewayは `http://127.0.0.1:<port>/mcp` に接続するだけで、backendプロセスを起動も監督もしません。lifecycle監督はSwiboの役割です。
- remote ingressは別のデプロイの関心事です。tunnelなしでも同じgatewayがloopback-onlyでserveします。
- バイナリはcoreの全モード（`serve` / `check` / `list-backends` / `migrate` / `version`）を単体で実装します。PowerShellラッパーは orchestration だけを行います。

この分離は願望ではなく強制されています: repositoryの衛生テストが `src/` と `Cargo.toml` を統合固有の識別子について走査し、CIのテストスイートは非公開デプロイを一切要求しません。

## どれが必要かの目安

- ローカルで動かして任意のMCP clientを接続する → **coreのみ**（[QUICKSTART.ja.md](QUICKSTART.ja.md) を参照）。
- gatewayをインターネット経由でChatGPTに公開する → **OpenAI Secure MCP Tunnel** 統合を追加。
- gateway/backend/tunnelプロセスを宣言的に監督する → **Swibo** 統合を追加。
- tunnel credentialをシェル環境ではなく暗号化ストアに置く → **SOPS** 統合を追加。
- install/check/start/status/stopを1コマンドで行いたい → **PowerShellラッパー** 統合を追加。

統合は組み合わせられます: tunnelラッパーは必須キーが環境変数に無いときだけSOPSヘルパーを呼び、Swiboはスタック全体を監督できます。

## 移行とロールバック

既存のlegacy `[proxy]` デプロイを公開schemaへ移行する手順とロールバックは [MIGRATION.ja.md](MIGRATION.ja.md) を参照してください。
