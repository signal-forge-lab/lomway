# Gateway workbench（skills / actions / pipelines / file transfer）

英語版 [WORKBENCH.md](WORKBENCH.md) が正本です。ここでは要点だけをまとめます。

Lomway 自身が Workbridge 相当の 4 機能を提供します。すべて**既定で無効**で、public 設定の各セクションで有効化します。設定に誤りがあってもゲートウェイの起動は止めず、その機能のツールがエラーを返します。

| 機能 | Workbridge の相当機能 | Lomway |
|---|---|---|
| Skills | open_workspace の skills と SKILL.md の read | initialize instructions、`lomway_search_tools` / `lomway_describe_tool`（`skill:<name>`） |
| Actions | `run_workspace_action` | `lomway_list_actions` / `lomway_run_action` / `lomway_action_status` |
| Pipelines | orchestration（最小版） | `lomway_run_pipeline` |
| チャットのファイル受信 | `download_artifact` | `lomway_receive_file`（`openai/fileParams`） |
| ファイル返却 | `publish_artifact` | `lomway_send_file` |

- **Skills**: Workbridge と同じ走査順・発見規則（SKILL.md で再帰停止、ルート直下の `*.md`、`.` 始まりと node_modules を除外、リンクを辿る、同一ファイルは 1 回、同名は先勝ち）。instructions に短い一覧を載せ、検索索引に `skill:<name>` を登録（ツール許可リストによる絞り込みなし）。`rescan_seconds` ごとに再走査するので、追加・削除は再起動なしで検索に反映。instructions は起動時点の一覧。
- **Actions**: リポジトリ外の TOML 定義ファイルを変更時に再読込（クライアントのツール再読込不要）。シェルを使わず argv で実行し、`{name}` は 1 引数、`{name...}` は配列展開。シェル・バッチへのパラメータ受け渡しは定義ロード時に拒否。パラメータは JSON Schema サブセットで検証（`-` 始まりは既定で拒否）。cwd は `allowed_roots` 内、環境変数は許可リスト方式で `LOMWAY_*` は渡さない。タイムアウトでプロセスツリーごと終了。dry_run、run_id による非同期実行、監査ログ（引数は SHA-256 のみ）。
- **Pipelines**: 各 step は登録済み action。effect の allow/deny を検査（`EFFECT_DENIED`）。dry_run で計画と digest を返し、`expected_digest` 付きで実行すると計画が変わっていれば `PLAN_INVALIDATED`。LLM worker/reviewer 等の Workbridge orchestration 機能は未実装。
- **File transfer**: OpenAI のファイルホストからの HTTPS のみ（redirect ごとに再検査、最大 3 回、30 秒）。サイズ上限、inbox 配下の相対パスのみ、親ディレクトリのリンク・reparse point を拒否、partial ファイルに書いてハードリンクで公開（上書きしない）。Workbridge のようなディレクトリハンドル固定は未実装（公開前後の再検証で検出のみ）。
