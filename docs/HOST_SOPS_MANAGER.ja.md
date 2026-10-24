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
