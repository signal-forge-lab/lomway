# PowerShell ラッパー統合（オプション）

状態: オプション統合 — CLI ラッパーのみ

このリポジトリのすべての PowerShell はオプションのラッパーツールです。Lomway バイナリは `lomway serve|check|list-backends|migrate|version` を PowerShell なしで実行できます。

## ラッパー一覧

| スクリプト | ラップ対象 | 統合に関する知識 |
| --- | --- | --- |
| `scripts/install.ps1` | `cargo build --release --locked`（＋ツールチェーン確認） | なし |
| `scripts/check.ps1` | `cargo run` 経由の `lomway check --config <path>` | なし |
| `scripts/start.ps1` | バックグラウンドの `lomway serve --config <path>` ＋ pid ファイル ＋ `GET /healthz` 待ち | なし |
| `scripts/status.ps1` | pid 取得 ＋ `GET /healthz` | なし |
| `scripts/stop.ps1` | pid 取得 ＋ `Stop-Process` | なし |
| `scripts/common.ps1` | 共有 pid ファイルヘルパー | なし |
| `scripts/integration-smoke.ps1` | 起動中ゲートウェイへの MCP `initialize` / `tools/list` / `tools/call`（ローカル 6 バックエンド回帰ラッパー） | なし |

統合固有のスクリプトは `scripts/` にはもう存在しません。それぞれのオプション統合ディレクトリにパッケージされています:

- `integrations/openai-secure-tunnel/` — Secure Tunnel ラッパー;
- `integrations/sops/` — シークレット解決ラッパー;
- `integrations/swibo/` — スーパーバイザーテンプレート（ドキュメントのみ）。

## コアビジネスロジックの重複なし

ラッパーは設定検証・ポリシー・名前空間・ルーティング・ヘルスのセマンティクスを再実装しません。`cargo`、ゲートウェイ CLI、ゲートウェイの公開 HTTP エンドポイントを呼び出して結果を報告するだけです。ポリシーの決定は唯一の情報源として Rust コアに存在します。ライフサイクルラッパーが追加するのは、プロセス配管（pid ファイル、ウィンドウ非表示、準備完了ポーリング）のみで、これらは CLI が意図的に保持しないものです。

## 保証

- ラッパーはゲートウェイプロセスの起動または停止のみを行います。変異的なツール呼び出しを再試行したり、ゲートウェイに再試行ロジックを追加したりすることはありません。
- ラッパーの既定値はリポジトリ相対パスと環境変数のみを参照します。マシンローカルの値は gitignore されたローカル設定に留まります。
