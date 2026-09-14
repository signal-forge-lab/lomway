# OpenAI Secure MCP Tunnel 統合（オプション）

状態: オプション統合

このディレクトリは、`tunnel-client` 経由で Lomway を ChatGPT に公開する OpenAI Secure MCP Tunnel ワークフローをパッケージ化したものです。Lomway Core はこの統合を**要求しません**。`src/` に OpenAI への参照はなく、`Cargo.toml` に OpenAI 依存もないため、`cargo build` / `cargo test` や通常の `lomway serve --config <path>` の実行に OpenAI 認証情報は不要です。

## ファイル

| ファイル | 目的 |
| --- | --- |
| `configure-tunnel.ps1` | Secure Tunnel エイリアスを一度だけ作成または再利用し、ゲートウェイの MCP エンドポイントへ接続します。 |
| `tunnel.ps1` | 設定済みエイリアスに対する `ensure`（必要なら再接続）、`status`、`stop`。 |

どちらのスクリプトも自己完結したラッパーです。`tunnel-client` を特定し、API キーをプロセス環境変数へ解決し、クライアントを一度だけ起動して、その JSON 出力を報告します。ゲートウェイの動作を再実装することはありません。

## 前提条件

- `PATH` 上の `tunnel-client`、または実行ファイルを指す `LOCAL_MCP_TUNNEL_CLIENT` プロセス環境変数。
- API キーはプロセス環境変数に直接設定するか、SOPS 統合（`integrations/sops/`）で正規ストアから解決します:
  - `tunnel.ps1` には `CONTROL_PLANE_API_KEY` が必要です。
  - `configure-tunnel.ps1` には `OPENAI_ADMIN_KEY` と `CONTROL_PLANE_API_KEY` が必要です。

## 使い方

初回設定のみ（ゲートウェイが起動済みで正常であること）:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
```

日常操作:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action stop
```

Tunnel プロファイルはリポジトリ外のユーザー application-data ディレクトリ配下に生成されます。Tunnel 関連のものがコミットされることはありません。

## 保証

- 各コマンドはクライアント操作を正確に一度だけ試みます。失敗した場合はオペレーターが手動で再実行します。変異的操作の自動再試行はありません。
- API キーはクライアント呼び出しの間だけプロセス環境変数に存在し、終了後は元に戻されます（`configure-tunnel.ps1`）。
