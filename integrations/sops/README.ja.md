# SOPS 統合（オプション）

状態: オプション統合

このディレクトリは、SOPS によるシークレット解決ワークフローをパッケージ化したものです。コアゲートウェイは SOPS を**要求しません**。コアはプロセス環境変数のみを読み取るため、SOPS をインストールしていなくてもシークレットを供給できます。

## コア独立性

- `src/` に SOPS への参照はなく、`Cargo.toml` に SOPS 依存もありません。`sops` バイナリが存在しないマシンでも `cargo build` / `cargo test` は成功します。
- 正しいシークレットのソースはユーザーの外部グローバル SOPS ストアのままです。このリポジトリには何もコミットされず、`integrations/sops/` 配下のファイルにシークレット値は含まれません。

## ファイル

| ファイル | 目的 |
| --- | --- |
| `Import-SopsSecrets.ps1` | 正規ストアを一度復号し、指定された名前のシークレットをプロセス環境変数へコピーします。 |

## 使い方

指定したシークレットを現在のプロセス環境へ解決します:

```powershell
pwsh -NoProfile -File integrations/sops/Import-SopsSecrets.ps1 -Names @('CONTROL_PLANE_API_KEY')
```

その後、通常どおりゲートウェイを起動します。設定内の環境変数参照は、注入された変数から解決されます:

```powershell
pwsh -NoProfile -File scripts/start.ps1
```

ストアの場所は既定で `%USERPROFILE%\.config\sops\secrets\global.sops.json` です。現在のプロセスに限り、`LOCAL_MCP_SOPS_STORE` で上書きできます。

## 保証

- シークレット値はプロセス環境変数にのみ注入されます。ディスクへ書かれたり、表示されたりすることはありません。
- 各実行は復号と適用を正確に一度だけ行います。ここで変異的操作を自動的に再試行することはありません。
