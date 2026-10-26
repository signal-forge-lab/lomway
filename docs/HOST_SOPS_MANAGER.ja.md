# ホスト内SOPS管理

Windows共通スクリプト: scripts/Invoke-HostSops.ps1

このツールは、キー名や発行ポリシーなど秘密ではない引数のみを受け付けます。
秘密値の生成、SOPSへの暗号化保存、読戻し照合はMCPホスト上で実行し、
応答には秘密値やダイジェストを含めません。

## 使用例

~~~powershell
# キーの存在と、Lomway PATの許可ツール件数を表示。値は表示しない。
pwsh -NoProfile -File scripts/Invoke-HostSops.ps1 -Action Status -Key DSH_LOMWAY_PAT -Provider LomwayPAT

# 新規のランダムなキーをホスト上で生成・暗号化保存。
pwsh -NoProfile -File scripts/Invoke-HostSops.ps1 -Action Generate -Key EXAMPLE_HOST_TOKEN -Provider RandomHex
~~~

Lomway PATについては、Generate / LomwayPAT / Label / AllowedTools / Daysを
指定して発行できます。既存キーの交換にはRotateを明示します。
PAT発行、暗号化保存、旧資格情報の失効までホスト内で完結します。
コマンドライン引数には秘密値を渡しません。

正本のSOPSファイルは固定パスです。更新時は暗号化済みステージ、
ACL検証、同時更新検出、読戻し検証、原子的な置換を使います。
失敗時の出力は、秘密値を含まないエラー種別と行番号に限定します。

## 保護範囲

秘密値をAIへのツール応答に含めない設計です。ただし、同じWindowsユーザー権限で
制限のない汎用シェルが実行可能なら、そのシェルから直接SOPSを復号できる点は
変わりません。秘密へのアクセス権そのものを隔離するには、別OSユーザーの
権限分離と制限されたIPC/MCP境界が必要です。
ホストプロセスのメモリ内には一時的に平文が存在します。

プラットフォーム側の操作制限を回避するための機構ではありません。

## 検証（2026-10-25）

実SOPS 3.13.3と使い捨てのage鍵・隔離されたUSERPROFILEおよび
LOCALAPPDATAを使ったEnd-to-Endテスト：

- 暗号化キーの存在確認（秘密値を返さない）
- 256-bit乱数の新規生成と暗号化、旧キーの交換
- Lomway PATの発行、権限メタデータの確認、旧PAT失効
- キー名の不正入力拒否
- 禁止ツールを含むPAT要求の拒否と非変更性
- 復号鍵なしでの存在確認

合計6件PASS。本番SOPSをテスト用に変更していません。
初期テストでステージファイルのACL差異による更新拒否が見つかったため、
正本ACLを引き継ぎ、実効権限が等価であることを検証するよう修正しました。
WindowsのSDDLに追加されるDACL自動継承の制御フラグだけを比較時に正規化します。

検証コマンド: py -3.14 -m pytest -q tests/test_host_sops_manager.py

## 2026-10-26 更新：書き込みポリシーと競合防止

- `Status` はこれまでどおり、指定キーの存在を秘密値なしで返します。
- `Generate` はホスト固定の `%USERPROFILE%\.config\sops\host-sops-policy.json`
  が必須です。未作成・不明キー・不一致の発行条件では拒否します。
- `keys` オブジェクトではキー名ごとに `provider` と `allowRotate` を指定します。
  LomwayPATではさらに `label`、`maxDays`、順不同で完全一致の
  `allowedTools` を指定してください。
- 同じ暗号化ストアを扱うSopshelfと共有する
  `%USERPROFILE%\.config\sops\secrets\.sops-write.lock` を排他的に確保し、
  更新を直列化します。異常終了で残ったロックは自動削除しません。
  他プロセスが操作していないことを確認してから管理者が復旧します。
- 成功した更新ごとに元の**暗号化済み**ファイルを
  `global.sops.json.backup-<UTC>-<id>.sops.json` に保存します。
  復元時はACLと暗号化データの確認を必須とし、平文へ展開しません。
- PAT交換ではSOPS保存後に旧PATの失効を最大3回試行します。
  失効できなければ `HOST_SOPS_PAT_REVOCATION_PENDING` を出し、
  新旧PATが同時に有効となり得ることを運用者へ通知します。

設定例（**実在する権限やキーではありません**）：

```json
{
  "version": 1,
  "keys": {
    "EXAMPLE_HOST_TOKEN": {
      "provider": "RandomHex",
      "allowRotate": false
    },
    "EXAMPLE_SCOPED_PAT": {
      "provider": "LomwayPAT",
      "allowRotate": false,
      "label": "example-agent",
      "maxDays": 7,
      "allowedTools": ["lomway_search_tools", "lomway_describe_tool"]
    }
  }
}
```

**重要な制約**：ポリシーは操作ミスを防ぐための制限であり、同じWindowsユーザーで
任意コードを実行できるAIを隔離するOS権限境界ではありません。真の隔離には
専用WindowsユーザーとACL保護されたSOPS/age鍵、承認付きの限定IPCが必要です。
この構成は本ツールの導入だけで自動的に実現するものではありません。

DSHの現在の16ツール権限に一致する**非アクティブな下書き**として
`docs/HOST_SOPS_POLICY_DRAFT_DSH.json` を収録しています。`praxiom_praxiom_execute`
を含むため、`allowRotate=false` としています。実際の本番ポリシーパスへ
自動コピーしません。適用やローテーション有効化は権限の確認・承認後に行います。
Praxiomプレビュー用の別発行スクリプトはHostSopsのキー別ポリシーを
直接呼び出すのではなく、同じホストポリシーの
`PRAXIOM_PREVIEW_TOKEN` 許可を照合する方式です。このDSH専用下書きには
含めていません。別途人間が発行を承認した場合だけ追加してください。
