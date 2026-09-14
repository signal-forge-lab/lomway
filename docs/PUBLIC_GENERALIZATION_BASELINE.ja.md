# 公開一般化 — G0 事前ベースライン

状態: 記録済み

タスク: LMG-G0-01 (`docs/PUBLIC_GENERALIZATION_TASKS.yaml`)。

本文書は Lomway（旧 Local MCP Gateway）の一般化前ベースラインを検証済みの記録として残すものである。
秘密情報、マシン固有の URL、ユーザー固有の絶対パスは含まない。

## 環境

| 項目 | 値 |
| --- | --- |
| 日付 (UTC) | 2026-09-13 |
| rustc | 1.98.0 |
| cargo | 1.98.0 |
| 依存関係 | `Cargo.lock` 固定、すべて `--locked` で実行 |
| mcp-proxy | `=0.4.3` にピン留め、feature `protocol-2026-07-28` |

## 記録時点の Git 状態

- ブランチ `main` は `refs/heads/main` としてのみ存在し、コミットは **0 件**(未初期化ブランチ)。
- 作業ツリーの状態: **54 エントリ、記録中にリセットや破棄は行っていない**。
  - ステージ済み新規ファイル (`A`): 43 件
  - ステージ後に追加修正があるファイル (`AM`): 4 件 (`README.md`, `README.ja.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/IMPLEMENTATION_PLAN.ja.md`)
  - 未追跡ファイル (`??`): 7 件 (`PUBLIC_GENERALIZATION_TASKS.yaml` を含む `docs/PUBLIC_GENERALIZATION_*` の設計・タスク一式)
- `target/` と `config/proxy.local.toml` は gitignore 済みであり、ビルド・テスト実行でも作業ツリーは汚れていない。
- `git status --porcelain` は検証実行の直前と直後で同一 (54 エントリ、スタッシュなし) であった。
  その後の変更は G0 の証拠・方針ドキュメント群のみである
  (`docs/PUBLIC_GENERALIZATION_ROLLBACK.md` 参照)。ソース・設定・テストコードは一切変更していない。

## 名前空間ベースライン

- セパレータ方針: `_` (`validate_policy` が強制)。
- 本番名前空間数: **6**(マシンローカル設定に対する `--check` で検証)。

| # | 名前空間 (バックエンド名) |
| --- | --- |
| 1 | `workbridge` |
| 2 | `memory` |
| 3 | `ufo` |
| 4 | `browser` |
| 5 | `xmind` |
| 6 | `praxiom` |

ツール名は `<名前空間>_<ツール>` で提供される(例: `browser_navigate`)。上流の
`proxy` コントロールプレーン名前空間はビルド時に除去され、クライアントからは見えない。
上記の名前空間名は `config/proxy.example.toml` に既に公開済みであり、URL はここに記録しない。

## テストベースライン

| スイート | コマンド | 結果 |
| --- | --- | --- |
| フォーマット | `cargo fmt --check` | 終了コード 0 |
| Lint | `cargo clippy --all-targets --locked -- -D warnings` | 終了コード 0、警告なし |
| ユニット (ライブラリ) | `cargo test --locked --lib` | 0 件、失敗 0 (`src/` に `#[cfg(test)]` モジュールはまだ存在しない) |
| ポリシー検証 | `cargo test --locked --test policy` | **10 合格、0 失敗** |
| モックバックエンド E2E | `cargo test --locked --test proxy` | **5 合格、0 失敗** |
| 実バックエンド回帰 | `cargo test --locked --test real_backends -- --ignored` | **1 合格、0 失敗**(6 バックエンドすべて到達可能、`tools/list` 数: workbridge 12、memory 32、ufo 19、browser 97、xmind 21、praxiom 2 — 合計 183 ツール) |
| 本番設定チェック | `cargo run --locked -- --config config/proxy.local.toml --check` | 終了コード 0 |

合計: **16 個のテスト関数が合格**(`policy` + `proxy` の無視されていない 15 件と、明示実行した
無視対象の実バックエンドテスト 1 件)。**ベースライン失敗はゼロ。**

補足:

- ライブラリには現在ユニットテストが存在しない。`tests/policy.rs`(10 テスト)が設定・ポリシー
  検証スイート、`tests/proxy.rs`(5 テスト)が `tower-mcp` を使うモックバックエンド E2E スイートである。
- 実バックエンドテストはマシンローカルのバックエンド群の起動を前提とするため既定で
  `#[ignore]` されている。今回は明示的に実行し、6 名前空間すべてで合格した。
- モック E2E は既に次を証明している: 上流 `proxy` バックエンドの除去、`/healthz` の存在と
  `/admin/*` の 404、衝突なしの名前空間切り分け、1 バックエンド offline 时的起動デグレード、
  タイムアウト時のミューテーションツールの exactly-once 実行。

## 本アーティファクトの秘密情報レビュー

- ベアラートークン、API キー、秘密鍵、メールアドレス、ユーザーパス、実 ID は含まない。
- バックエンド名前空間名は公開情報(`config/proxy.example.toml` に記載)である。
- マシンローカル設定ファイル自体は gitignore 済みであり、ここには転載しない。

## 再現方法

上記のツールチェーンバージョンで、リポジトリルートから各コマンドを実行する。
実バックエンドスイートは、6 つの監視対象ローカルバックエンドが起動していることを追加で要求する。

## 検証の由来

- 2026-09-13 に記録し、同日に上記の全コマンドを改めて実行して独立に再検証した。
  すべての結果は同一だった (fmt/clippy 終了コード 0、0 + 10 + 5 + 1 テスト合格、
  `--check` は 6 名前空間を報告)。
- このベースラインに紐付くロールバック方針: `docs/PUBLIC_GENERALIZATION_ROLLBACK.ja.md`
  (`docs/PUBLIC_GENERALIZATION_ROLLBACK.md`)。
