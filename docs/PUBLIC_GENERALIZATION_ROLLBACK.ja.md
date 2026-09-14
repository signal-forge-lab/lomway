# 公開一般化 — ブランチ & ワークツリー ロールバック方針

状態: 有効

タスク: LMG-G0-02 (`docs/PUBLIC_GENERALIZATION_TASKS.yaml`)。
関連記録: `docs/PUBLIC_GENERALIZATION_BASELINE.md` / `docs/PUBLIC_GENERALIZATION_BASELINE.ja.md`。

## 目的

公開一般化イニシアティブにおける、曖昧さのないロールバック地点とブランチ/ワークツリー
規律を定める。これにより、LMG-G1 以降のどの変更からでも、検証済みの一般化前ベースラインへ、
現行の本番デプロイを危険に晒すことなく戻せるようにする。

## 保護対象の状態

- 現行のデプロイは、`docs/PUBLIC_GENERALIZATION_BASELINE.md` に記録された 6 名前空間の
  本番プロファイルである。
- `config/proxy.local.toml`、SOPS 素材、生成された実行時状態はマシンローカルであり、
  gitignore を維持し、コミットもロールバック成果物への同梱も決して行わない。

## 安定したロールバック地点

- ロールバック地点は **`main` のルートコミット**、すなわち LMG-G1 の変更を一切加える前に
  取り込まれた一般化前ツリーから作成される初期ベースラインコミットである。
- 特定は機械的に行う: `git rev-list --max-parents=0 main` がハッシュをちょうど 1 件だけ
  表示しなければならず、そのハッシュがロールバック地点である。
- そのコミットはベースライン記録と紐付く: チェックアウトして fmt/clippy のクリーン、
  0 + 10 + 5 + 1 のテスト結果、`--check` による 6 バックエンドの報告が再現できなければならない。

## 記録時点の保護状況 (2026-09-13)

- `main` は未初期化ブランチ (コミット 0 件) だった。一般化前ツリーの全体が作業ツリーと
  インデックス (ステージ済み `A` 43 件、`AM` 4 件、未追跡 7 件) としてのみ存在し、
  スタッシュも存在しなかった。ベースラインコミットが存在するまで、この作業ツリーが
  デプロイの唯一のコピーである。
- ベースラインコミットが存在するまでの暫定厳守規則: いかなる `git reset` も禁止、
  破棄を伴うチェックアウト (`checkout -- .`) 禁止、`git clean` 禁止、
  `git stash drop/pop` 禁止、ブランチ削除禁止、rebase 禁止。

## ブランチ方針

- 一般化作業はすべて、ロールバック地点から切った作業ブランチで行う:
  `git switch -c public-generalization <rollback-point>`。
- `main` は検証済みタスク群の境界でのみファストフォワードで進める。`main` の
  強制プッシュや履歴書き換えは決して行わない。
- マシンローカル設定、SOPS 素材、実行時状態は永久に無視対象のままとする。

## 有効化ステップ (オペレーターが LMG-G1-01 の前に実施)

ベースライン記録は未初期化ブランチ上で実施されたため、ロールバック地点を不変にするのは
1 回の意図的なコミットである。これは記録責任を持つオペレーターが実行する:

```text
git add -A
git commit -m "chore: pre-generalization baseline (LMG-G0 rollback point)"
git rev-list --max-parents=0 main   # ハッシュがちょうど 1 件表示されること
git status --porcelain              # 空であること
```

有効化記録 (2026-09-13、検証ラウンド 2): コミット
`c98ea957582006b781edfc6fb50e9f96368b156b`(`chore: pre-generalization baseline
(LMG-G0 rollback point)`)として実行済み。`git rev-list --max-parents=0 main` は
このハッシュのみを表示し、作業ツリーはその後クリーンであった。

## ロールバック手順

1. まず安全スナップショット: `git status --porcelain` の出力をリポジトリ外に保存し、
   未コミットの作業を残して検査する必要がある場合に限り
   `git stash push -u -m "pre-rollback safety snapshot"` を実行する
   (安全用スタッシュを黙って破棄することはない)。
2. 作業ブランチ上の一般化変更は `git reset --hard <rollback-point>` で破棄するか、
   ブランチごと `git switch main && git branch -D <branch>` で放棄する。
   `main` のルートコミットは決して書き換えない。
3. G0 ベースラインスイート (fmt、clippy、`policy`、`proxy`、`real_backends --ignored`、
   `--check`) を再実行し、`docs/PUBLIC_GENERALIZATION_BASELINE.md` と比較して復元を検証する
   (合格テスト関数 16 件、名前空間 6 件)。
4. 記録済みベースライン証拠を再現できないロールバックは完了とはみなさない。
   それ以上の変更の前に停止して調査する。
