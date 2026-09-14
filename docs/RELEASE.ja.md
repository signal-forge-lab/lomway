# リリースメタデータとチェックリスト

Updated: 2026-09-14

Lomwayのリリース契約です。リリースを識別するもの、各リリースメタデータの置き場所、リリース前に通過が必須のゲートを定めます。**このrepositoryが自動でpublishやpushを行うことはありません** — タグ付け・成果物のアップロード・外部への公開は、すべてmaintainerによる明示的な手動操作です。

English: [RELEASE.md](RELEASE.md)

## 1. バージョン

- 単一の情報源: [`Cargo.toml`](../Cargo.toml) の `[package]` `version`。CLIは `lomway version` で（MCP protocol featureと構成 `schema_version` と併せて）報告し、`Cargo.lock` が解決済みツリーを固定します。
- バージョニングは [Semantic Versioning](https://semver.org/) に従います。公開構成schema・namespace契約（prefixはclient-visibleなtool名の一部です）・MCP surfaceの破壊的変更は、該当するmajor/minorを上げます。現行リリースは `0.1.0` です。
- 構成schemaの互換性は独立にバージョン管理されます: このリリースでサポートされる公開構成schemaは `schema_version = 1` のみです。

## 2. Changelog

- [`docs/CHANGELOG.md`](CHANGELOG.md) は利用者向けchangelogで、影響単位（Added / Changed / Fixed / Security）に整理し、新しい順に記述します。リリース時に遡って書くのではなく、変更を行う同じ変更内で書きます。
- changelogは英語で保守します。日本語ドキュメント群は挙動を記述し、関係する箇所からリリースentryへリンクします。

## 3. 成果物とチェックサム

リリースマシン上で手動で作成します。CIは行いません:

```powershell
$releaseTarget = Join-Path $env:TEMP ("lomway-release-" + [guid]::NewGuid().ToString('N'))
$env:CARGO_TARGET_DIR = $releaseTarget
cargo build --locked --release
Get-FileHash -Algorithm SHA256 (Join-Path $releaseTarget 'release/lomway.exe')
```

- buildは必ず `--locked` で。commit済みの `Cargo.lock` はレビュー済みリリース内容の一部です。
- Windowsで `target/release/lomway.exe` からLomwayを稼働中の場合、OSが実行中exeをlockするためin-place release rebuildはaccess deniedになり得ます。そのためfresh staging `CARGO_TARGET_DIR` をrelease標準手順とします。代替はLomwayを停止してからin-place artifactをbuildします。
- 公開するすべての成果物（プラットフォームごとのバイナリとsource archive）のSHA-256を、対応する成果物の隣にrelease notesへ記録します。
- チェックサムは具体的なリリース成果物セットの属性であり、リリース時に生成するものであって、repositoryへはcommitしません。

## 4. ライセンス

- `Cargo.toml` は `license = "MIT"`。ライセンス全文は [`LICENSE`](../LICENSE) です。
- 依存のライセンスレビューはリリースゲートです。`python scripts/dependency_gate.py` がcommit済みlockfileと `cargo metadata --locked` のpackage set一致、全packageのlicense metadata、OSVを検査します。未解決OSVを保守的にすべてblockするため、high/criticalは必ずリリースをblockします。

## 5. ソース

- リリースは不変なタグ付きポイントです: `git tag -a v<version> -m "Release <version>"`。タグは以下のゲートを通過した後にのみ作成します。
- タグ対象のツリーは意図されたリリースツリーでなければなりません: ローカルログ・runtime state・PIDファイル・マシンローカル構成・生成されたtunnel profileを含みません（すべてgitignore対象で、タグ前に不在を確認します）。

## 6. サンプル

repositoryに同梱され、ドキュメントから参照されます:

- [`config/proxy.example.toml`](../config/proxy.example.toml) — 公開サンプル構成。環境変数参照のみで、実endpointは含みません。
- [`test-fixtures/configs/`](../test-fixtures/) — テストスイートが検証する再配布可能な zero/one/many-backend 構成。

## 7. リリース前ゲート

タグ付けの前に、意図されたリリースツリー上ですべて通過していること:

1. `cargo fmt --check --all` → exit 0
2. `cargo clippy --all-targets --locked -- -D warnings` → exit 0
3. `cargo test --locked` → グリーン（モックbackendのみ。6-backendの実機regressionは `--ignored` かつmachine-localのまま）
4. locked release build → 成功。idle/clean machineでは `cargo build --locked --release` でよく、live Windows workstationではfresh staging `CARGO_TARGET_DIR` を使い、実行中exeを上書きしないこと。
5. ドキュメントが実際のCLIと構成schemaと一致していること（[QUICKSTART.ja.md](QUICKSTART.ja.md) と [CONFIGURATION.ja.md](CONFIGURATION.ja.md) のコマンドが記載どおり動くこと）。
6. プライバシー/衛生スキャン: `python scripts/release_privacy_scan.py` が、現在の候補treeだけでなく **`HEAD` から到達可能な全履歴blob** についてもcredential・非公開識別子・マシン固有パス・secret値・禁止されたgenerated/private path 0でPASSすること。
7. 依存/license/vulnerability gate: `python scripts/dependency_gate.py` がexact lockfile package setと一致し、全packageにlicense metadataがあり、未解決OSV recordが0であること。
8. クリーンマシン成果物スモーク: `pwsh -NoProfile -File scripts/clean-machine-smoke.ps1` がfresh temp directoryへrelease binaryとpublic fixtureをcopyし、再配布可能public mock backendだけで `check` / `serve` / `/healthz` / `/readyz` をPASSすること。
9. 公開push形状: `main` は公開対象履歴だけを含み、`git status --short --branch` がcleanであること。pushは `git push -u origin main` を使用し、`git push --all` / `git push --mirror` は使用しない。local review/backup refは公開対象ではありません。

## 8. 初回public push準備

公開repository URLはこのproject側で勝手に作りません。maintainerが実際の公開先を指定するまで、`Cargo.toml` の `repository` は未設定、Git remoteも未設定のままにします。

公開repository作成後のpush手順:

```powershell
git remote add origin <public-repository-url>
git remote -v
python scripts/release_privacy_scan.py
git status --short --branch
git push -u origin main
```

push対象は **`main` のみ**です。local toolingが非公開review/backup refを保持する場合があるため、`--all` / `--mirror` は使用しません。

## 9. このrepositoryが意図的に行わないこと

- 自動publish・タグpush・成果物アップロード・CI起動のリリースは行いません。上記の全ステップは手動です。
- リリースステップの自動retryはしません。失敗したゲートは失敗として調査対象になり、盲目的な再実行はしません。
- `Cargo.toml` の `repository` フィールドは、maintainerが公開リポジトリの場所を作るまで意図的に未設定です。privateな場所やマシン固有の場所を指してはいけません。
