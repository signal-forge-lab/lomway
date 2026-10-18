# 機能検索（capability search）

[English](CAPABILITY_SEARCH.md)

`lomway_search_tools` は「この gateway に X はあるか」を確かめる唯一の入口です。
3 種類の対象を、ひとつのランキングで検索します。

| `invocation` | 対象 | 結果の使い方 |
|---|---|---|
| `direct` | `tools/list` に既に出ているツール | 名前で直接呼ぶ。`lomway_call_tool` は使わない。 |
| `deferred` | `exposure = "deferred"` のバックエンドのツール | `lomway_describe_tool` のあと `lomway_call_tool`。 |
| `internal` | 自前の MCP ツールを持たない宣言済み機能（シェルツール経由の CLI など） | `via` に書かれたツールを使う。 |

## 保証

- **検索はバックエンドを起動しない。** Direct は gateway がキャッシュしている集約カタログから、
  Deferred は最後に観測したマニフェストから返す（`status = "cached"`、`manifest_seen_at` は RFC 3339 UTC）。
  一度もツールを観測していない Deferred バックエンドは `<prefix>*` のプレースホルダー
  （`status = "unverified"`）として返す。そのプレースホルダーを describe することが、
  バックエンドへ接続する明示的な操作になる。
- **`status` は出どころを表し、正常性は表さない。** `listed` は gateway の現在の `tools/list` に
  載っていること（Direct ツール、または明示的な describe 直後の Deferred ツール）だけを意味し、
  バックエンドが正常であることや呼び出しが成功することは示さない。`cached` と `unverified` は上のとおりで、
  `declared` は internal の機能を表す。
- **結果は呼び出し主体の権限に限定される。** PAT と制限付き OAuth の呼び出しでは、
  北側の認可レイヤーが呼び出し主体の許可リストを search/describe に渡す（呼び出し側が渡した値は上書きする）。
  使えないツールはランキング前に除外し、describe しても存在しないツールと同じエラーを返す。
  これらの呼び出し主体では、未知の `backend` を指定しても、見えないバックエンドを指定したときと同じ空の結果を返す。
  制限のない呼び出し主体には、未知の backend id は従来どおりエラーになる。
- **メタツールの説明文は一般的な文にとどめる。** `tools/list` はすべての呼び出し主体に見えるため、
  `lomway_search_tools`・`lomway_describe_tool`・`lomway_call_tool` の説明文には設定済みの backend id を書かず、
  `server.instructions` も繰り返さない（こちらは MCP の `instructions` フィールドでクライアントに届く）。
  代わりに「Direct の結果は直接呼ぶ、internal は `via` のツール経由、Deferred の結果だけ describe→call」という規則を書く。
- **無関係なクエリには何も返さない。** スコアが閾値未満の結果は捨てる。

## ランキング

強い順に、上流 OSS 名、宣言した別名、action 名と action の別名、ツール名・バックエンド名、説明文。
小文字化と全角英数の半角化のあと、完全一致・包含・英単語の重なり・日本語の文字 bigram で照合する。
結果はツールごとに 1 件で、十分に一致した action があれば `action` に入る。

## 宣言ファイル

`capabilities/catalog.toml` に、`tool`（完全なツール名）または `backend`（そのバックエンドの全ツールと
unverified プレースホルダー）ごとに、OSS 名・分野・別名・action ごとの別名を宣言する。
`[[internal]]` は `via` のツール（または候補ツールの順序付きリスト）を指定する。internal の機能は、`via` の候補の
どれかが gateway の現在の `tools/list` に Direct ツールとして載っているときだけ索引・describe の対象になり、
最初に見つかった候補が結果の `via` になる。どの候補も公開していない構成では、その機能は出てこない。
そのため、存在しないツールへ案内することはない。`tool` 指定の宣言も、一覧に載っているツールにだけ付く。
このファイルは公開されるため、名前・別名・要約だけを書く。
検証はすべてのフィールド（`[[internal]]` の `domain` を含む）に対して行い、URL、ループバック・private・
リンクローカル・CGNAT の IPv4/IPv6 アドレス、`host:port`、POSIX の絶対パス、`~/`、Windows のパスと UNC、
メールアドレスを拒否する。通常の語の区切りとしてのスラッシュ（`read/write` など）は許可する。

## マニフェストキャッシュ

`lomway serve` は Deferred のマニフェストをユーザーごとの状態ディレクトリに保存する。
`LOMWAY_STATE_DIR` があればそこ、なければ Windows は `%LOCALAPPDATA%\Lomway\state`、
macOS は `~/Library/Application Support/Lomway/state`、それ以外は `$XDG_STATE_HOME/lomway`
（または `~/.local/state/lomway`）。gateway の識別（待ち受けポートと Deferred バックエンドの id・prefix）ごとに 1 ファイル。
各バックエンドのマニフェストは、接続先を決める設定（トランスポートと解決後の URL）の SHA-256 フィンガープリントでも
識別する。この設定が変わると古いマニフェストは使わず、再び観測されるまで `unverified` として返す。
ファイル名とキャッシュの中身にはハッシュだけを書き、接続先の値そのものは保存もログ出力もしない。
書き込みは、プロセスと書き込みごとに一意な一時ファイルに書いてから置き換える。プロセス内では書き込みを直列化するので、
並列に観測しても一時ファイルを壊し合ったり、古いスナップショットが残ったりしない。
起動時と、describe/call による明示的な有効化のあとに更新する。`Gateway::build` を使うライブラリ利用では
メモリ内だけに保持する。
